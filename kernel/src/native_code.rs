use crate::guard_policy::{ActionIntent, AgentKind, GuardPolicy, PolicyDecision};
use crate::guard_protocol::{ActionCandidate, Protocol};
use crate::scheduler::ChatMessage;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

const TRACKED_FILE_LIMIT: usize = 160;
const STATUS_LINE_LIMIT: usize = 80;
const PROPOSAL_ACTION_LIMIT: usize = 20;
const PROPOSAL_BEGIN: &str = "KERNA_PROPOSAL_JSON_BEGIN";
const PROPOSAL_END: &str = "KERNA_PROPOSAL_JSON_END";

#[derive(Debug, Clone)]
pub struct CodeDryRunContext {
    pub repo_root: PathBuf,
    pub head: String,
    pub status_digest: String,
    pub tracked_file_count: usize,
    pub status_line_count: usize,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProposalPreflight {
    pub schema_version: u32,
    pub mode: &'static str,
    pub receipt_state: &'static str,
    pub actions: Vec<ProposalActionPreflight>,
}

/// Parse result for one assistant proposal: the stable preflight rendering plus
/// the canonical intent and policy decision that later governed phases need to
/// bind receipts without re-normalizing raw model text.
#[derive(Debug, Clone)]
pub struct ProposalPreflightParsed {
    pub preflight: ProposalPreflight,
    pub actions: Vec<ProposalAction>,
}

#[derive(Debug, Clone)]
pub struct ProposalAction {
    pub proposed_kind: String,
    pub intent: ActionIntent,
    pub decision: PolicyDecision,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProposalActionPreflight {
    pub id: String,
    pub proposed_kind: String,
    pub reason: String,
    pub raw_tool_name: String,
    pub action_kind: String,
    pub canonical_resource: Option<String>,
    pub canonical_action_digest: String,
    pub policy_effect: String,
    pub policy_rule: Option<String>,
    pub policy_reason: String,
    pub required_containment: &'static str,
    pub executable: bool,
    pub receipt_state: &'static str,
    pub risk_tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalEnvelope {
    actions: Vec<ProposalActionInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalActionInput {
    kind: String,
    reason: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    manager: Option<String>,
}

pub fn build_code_dry_run_context(repo: &Path, goal: &str) -> Result<CodeDryRunContext> {
    if goal.trim().is_empty() {
        return Err(anyhow!("the code goal cannot be empty"));
    }
    let repo_root = git_text(repo, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .context("could not resolve the Git repository for kerna code")?;
    let head = git_text(&repo_root, &["rev-parse", "--short=12", "HEAD"])
        .unwrap_or_else(|_| "unborn-or-detached".to_string());
    let status = git_text(
        &repo_root,
        &["status", "--short", "--untracked-files=normal"],
    )?;
    let tracked = git_text(&repo_root, &["ls-files"])?;
    let status_lines = bounded_lines(&status, STATUS_LINE_LIMIT);
    let tracked_lines = bounded_lines(&tracked, TRACKED_FILE_LIMIT);
    let status_digest = format!("{:x}", Sha256::digest(status.as_bytes()));
    let prompt = render_prompt(
        goal,
        &head,
        &status_digest,
        status.lines().count(),
        &status_lines,
        tracked.lines().count(),
        &tracked_lines,
    );
    Ok(CodeDryRunContext {
        repo_root,
        head,
        status_digest,
        tracked_file_count: tracked.lines().count(),
        status_line_count: status.lines().count(),
        prompt,
    })
}

pub fn dry_run_messages(prompt: String) -> Vec<ChatMessage> {
    vec![ChatMessage {
        role: "user".to_string(),
        content: Some(prompt),
        tool_calls: None,
        tool_call_id: None,
    }]
}

pub fn parse_proposal_preflight(
    assistant_text: &str,
    policy: &GuardPolicy,
    session_id: &str,
) -> Result<ProposalPreflightParsed> {
    let raw = extract_proposal_json(assistant_text)?;
    let envelope: ProposalEnvelope =
        serde_json::from_str(raw).context("proposal envelope was malformed")?;
    if envelope.actions.is_empty() {
        return Err(anyhow!("proposal envelope contained no actions"));
    }
    if envelope.actions.len() > PROPOSAL_ACTION_LIMIT {
        return Err(anyhow!(
            "proposal envelope contained more than {PROPOSAL_ACTION_LIMIT} actions"
        ));
    }
    let mut preflight_actions = Vec::with_capacity(envelope.actions.len());
    let mut actions = Vec::with_capacity(envelope.actions.len());
    for (index, action) in envelope.actions.iter().enumerate() {
        let (preflight_action, action) = preflight_action(index, action, policy, session_id)?;
        preflight_actions.push(preflight_action);
        actions.push(action);
    }
    Ok(ProposalPreflightParsed {
        preflight: ProposalPreflight {
            schema_version: 1,
            mode: "preflight_only",
            receipt_state: "preflight_only_not_requested",
            actions: preflight_actions,
        },
        actions,
    })
}

fn extract_proposal_json(text: &str) -> Result<&str> {
    let (_, tail) = text
        .split_once(PROPOSAL_BEGIN)
        .ok_or_else(|| anyhow!("proposal envelope missing {PROPOSAL_BEGIN}"))?;
    let (json, _) = tail
        .split_once(PROPOSAL_END)
        .ok_or_else(|| anyhow!("proposal envelope missing {PROPOSAL_END}"))?;
    let json = json.trim();
    if json.is_empty() {
        return Err(anyhow!("proposal envelope was empty"));
    }
    Ok(json)
}

fn preflight_action(
    index: usize,
    action: &ProposalActionInput,
    policy: &GuardPolicy,
    session_id: &str,
) -> Result<(ProposalActionPreflight, ProposalAction)> {
    let proposed_kind = action.kind.trim().to_ascii_lowercase();
    let reason = bounded_text("reason", &action.reason)?;
    let (raw_tool_name, arguments, required_containment) = match proposed_kind.as_str() {
        "file_read" => (
            "Read".to_string(),
            json!({"file_path": required_field("path", action.path.as_deref())?}),
            "future contained worktree read",
        ),
        "file_write" => (
            "Write".to_string(),
            json!({"file_path": required_field("path", action.path.as_deref())?}),
            "future contained worktree write plus review/apply",
        ),
        "shell" => (
            "Bash".to_string(),
            json!({"command": required_field("command", action.command.as_deref())?}),
            "future contained Docker process plus receipt-bound approval",
        ),
        "network" => (
            "WebFetch".to_string(),
            json!({"url": required_field("url", action.url.as_deref())?}),
            "future broker egress allowlist plus receipt-bound approval",
        ),
        "package" => (
            "Bash".to_string(),
            json!({"command": package_command(action)?}),
            "future contained package-manager execution plus receipt-bound approval",
        ),
        _ => return Err(anyhow!("unsupported proposal action kind: {proposed_kind}")),
    };
    let candidate = ActionCandidate {
        protocol: Protocol::AnthropicMessages,
        id: format!("proposal_{}", index + 1),
        raw_tool_name,
        arguments,
    };
    let intent = ActionIntent::from_candidate(
        &candidate,
        session_id,
        AgentKind::KernaNative,
        env!("CARGO_PKG_VERSION"),
    );
    let decision = policy.evaluate(&intent);
    let canonical_action_digest = intent.canonical_digest();
    Ok((
        ProposalActionPreflight {
            id: candidate.id,
            proposed_kind: proposed_kind.clone(),
            reason,
            raw_tool_name: intent.raw_tool_name.clone(),
            action_kind: stable_label(intent.kind)?,
            canonical_resource: intent.canonical_resource.clone(),
            canonical_action_digest: canonical_action_digest.clone(),
            policy_effect: stable_label(decision.effect)?,
            policy_rule: decision.rule_id.clone(),
            policy_reason: decision.reason.clone(),
            required_containment,
            executable: false,
            receipt_state: "preflight_only_not_requested",
            risk_tags: intent.risk_tags.clone(),
        },
        ProposalAction {
            proposed_kind,
            intent,
            decision,
        },
    ))
}

fn stable_label<T: Serialize>(value: T) -> Result<String> {
    serde_json::to_value(value)?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("stable label did not serialize as a string"))
}

fn required_field<'a>(name: &str, value: Option<&'a str>) -> Result<&'a str> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("proposal action missing required field {name}"))?;
    if value.len() > 512 {
        return Err(anyhow!("proposal action field {name} is too long"));
    }
    Ok(value)
}

fn bounded_text(name: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("proposal action missing required field {name}"));
    }
    if value.len() > 512 {
        return Err(anyhow!("proposal action field {name} is too long"));
    }
    Ok(value.to_string())
}

fn package_command(action: &ProposalActionInput) -> Result<String> {
    let manager = required_field("manager", action.manager.as_deref())?.to_ascii_lowercase();
    let package = required_field("package", action.package.as_deref())?;
    match manager.as_str() {
        "npm" => Ok(format!("npm install {package}")),
        "cargo" => Ok(format!("cargo add {package}")),
        "pip" => Ok(format!("pip install {package}")),
        other => Err(anyhow!("unsupported package manager in proposal: {other}")),
    }
}

fn git_text(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-c")
        .arg("safe.directory=*")
        .args(args)
        .current_dir(repo)
        .output()
        .with_context(|| format!("could not run git {}", args.join(" ")))?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn bounded_lines(input: &str, limit: usize) -> Vec<String> {
    input
        .lines()
        .take(limit)
        .map(|line| line.chars().take(240).collect())
        .collect()
}

fn render_prompt(
    goal: &str,
    head: &str,
    status_digest: &str,
    status_total: usize,
    status_lines: &[String],
    tracked_total: usize,
    tracked_lines: &[String],
) -> String {
    let status_block = if status_lines.is_empty() {
        "(clean)".to_string()
    } else {
        status_lines.join("\n")
    };
    let tracked_block = if tracked_lines.is_empty() {
        "(no tracked files reported)".to_string()
    } else {
        tracked_lines.join("\n")
    };
    format!(
        "You are Kerna code planner in dry-run mode.\n\
         You have no tools, no file contents, no shell, no network, no patch authority, and no approval grant.\n\
         Do not claim you changed files. Do not output a patch. Do not ask to run commands.\n\
         Produce only a concise implementation proposal with:\n\
         1. likely files or areas to inspect,\n\
         2. proposed steps,\n\
         3. security boundary notes,\n\
         4. what approval/receipt/containment would be required before execution.\n\
         After the prose, output one strict JSON object between {PROPOSAL_BEGIN} and {PROPOSAL_END}.\n\
         The JSON shape is:\n\
         {{\"actions\":[{{\"kind\":\"file_read|file_write|shell|network|package\",\"reason\":\"why\",\"path\":\"relative/path\",\"command\":\"command\",\"url\":\"https://example.com\",\"manager\":\"npm|cargo|pip\",\"package\":\"name\"}}]}}\n\
         Include only actions you propose for future review. Do not include unknown fields.\n\n\
         Goal:\n{goal}\n\n\
         Repository snapshot, metadata only:\n\
         HEAD: {head}\n\
         status_digest_sha256: {status_digest}\n\
         status_lines: {status_total} total, showing up to {STATUS_LINE_LIMIT}\n\
         {status_block}\n\n\
         tracked_files: {tracked_total} total, showing up to {TRACKED_FILE_LIMIT}\n\
         {tracked_block}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_prompt_is_explicitly_non_executing() {
        let context = render_prompt(
            "fix the parser",
            "abc123",
            "d".repeat(64).as_str(),
            1,
            &[" M src/lib.rs".to_string()],
            2,
            &["src/lib.rs".to_string(), "Cargo.toml".to_string()],
        );
        assert!(context.contains("dry-run mode"));
        assert!(context.contains("no shell"));
        assert!(context.contains("Do not output a patch"));
        assert!(context.contains("approval/receipt/containment"));
        assert!(!context.contains("C:\\"));
    }

    #[test]
    fn dry_run_messages_contain_no_tool_authority() {
        let messages = dry_run_messages("plan".to_string());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert!(messages[0].tool_calls.is_none());
        assert!(messages[0].tool_call_id.is_none());
    }

    #[test]
    fn proposal_preflight_normalizes_actions_without_execution() {
        let text = r#"Plan first.
KERNA_PROPOSAL_JSON_BEGIN
{"actions":[
  {"kind":"file_write","path":"src/lib.rs","reason":"update parser"},
  {"kind":"shell","command":"cargo test","reason":"verify"}
]}
KERNA_PROPOSAL_JSON_END"#;
        let parsed = parse_proposal_preflight(text, &GuardPolicy::balanced(), "session-1").unwrap();
        let preflight = parsed.preflight;
        assert_eq!(preflight.mode, "preflight_only");
        assert_eq!(preflight.receipt_state, "preflight_only_not_requested");
        assert_eq!(preflight.actions.len(), 2);
        assert!(preflight.actions.iter().all(|action| !action.executable));
        assert!(preflight
            .actions
            .iter()
            .all(|action| action.receipt_state == "preflight_only_not_requested"));
        assert_eq!(
            preflight.actions[0].canonical_resource.as_deref(),
            Some("src/lib.rs")
        );
        assert_eq!(
            preflight.actions[1].canonical_resource.as_deref(),
            Some("cargo")
        );
        // The parsed actions keep the canonical intent and decision for governed phases
        // without exposing raw model text beyond the already-validated envelope fields.
        assert_eq!(parsed.actions[0].proposed_kind, "file_write");
        assert_eq!(parsed.actions[0].intent.id, "proposal_1");
        assert_eq!(
            parsed.actions[0].intent.canonical_resource.as_deref(),
            Some("src/lib.rs")
        );
        assert!(!parsed.actions[0].intent.canonical_digest().is_empty());
    }

    #[test]
    fn malformed_or_unknown_proposals_fail_closed() {
        assert!(
            parse_proposal_preflight("no envelope", &GuardPolicy::balanced(), "session-1").is_err()
        );
        let unknown = r#"KERNA_PROPOSAL_JSON_BEGIN
{"actions":[{"kind":"docker","reason":"escape"}]}
KERNA_PROPOSAL_JSON_END"#;
        assert!(parse_proposal_preflight(unknown, &GuardPolicy::balanced(), "session-1").is_err());
        let extra = r#"KERNA_PROPOSAL_JSON_BEGIN
{"actions":[{"kind":"file_read","path":"src/lib.rs","reason":"inspect","extra":true}]}
KERNA_PROPOSAL_JSON_END"#;
        assert!(parse_proposal_preflight(extra, &GuardPolicy::balanced(), "session-1").is_err());
    }
}
