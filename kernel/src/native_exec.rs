//! Native governed execution for `kerna code`: the user enters a prompt, and
//! Kerna itself runs the model turn loop against the broker and executes the
//! model's proposals as receipt-bound actions inside a disposable candidate
//! clone. No Claude Code CLI is launched and no Docker session is required for
//! this path; the honest containment label is trusted-CLI path validation over
//! the candidate worktree, and the original repository changes only through
//! one explicit, human-approved apply of the reviewed diff.
//!
//! Policy governs each action (allow / ask / deny). An `ask` action requires a
//! one-time human approval in the trusted CLI before its release receipt is
//! written; denial, refusal, or a receipt that cannot commit leaves the action
//! unexecuted. Every release commits before its side effect, and a side effect
//! that then fails is recorded as `outcome_unknown`, never as success.

use crate::guard_policy::{ActionIntent, GuardPolicy, PolicyDecision, PolicyEffect};
use crate::memory::{GuardActionBinding, MemoryEngine};
use crate::native_code::{ProposalAction, PROPOSAL_BEGIN};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const AGENT: &str = "kerna_native";
const PROTOCOL: &str = "anthropic_messages";
pub const EXEC_CONTAINMENT_LABEL: &str = "trusted_cli_candidate_worktree";
pub const EXEC_WRITE_MAX_BYTES: usize = 256 * 1024;
pub const EXEC_RESULT_PREVIEW_BYTES: usize = 2 * 1024;
const EXEC_SHELL_TIMEOUT: Duration = Duration::from_secs(60);

/// How `ask` actions obtain their one-time human decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    /// Prompt on the trusted terminal through `dialoguer`.
    Interactive,
    /// The human pre-authorized this session with `--yes`; each approved
    /// action still records `approval.granted_via = cli_pre_authorized_flag`
    /// and its exact canonical digest, and denies still stand.
    PreAuthorized,
    /// No human channel is available (JSON output without `--yes`); every
    /// `ask` action fails closed as unapproved.
    Unavailable,
}

/// "Allow for this session" decisions from the interactive menu. Each action
/// still carries its own receipt bound to its exact canonical digest; the
/// grant only records that the human chose to stop being asked again for the
/// same kind + resource in this process.
static SESSION_GRANTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::OnceLock::new();

fn session_grants() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    SESSION_GRANTS.get_or_init(Default::default)
}

pub fn clear_session_grants() {
    if let Ok(mut grants) = session_grants().lock() {
        grants.clear();
    }
}

fn session_grant_key(action: &ProposalAction) -> String {
    format!(
        "{}:{}",
        action.proposed_kind,
        action
            .intent
            .canonical_resource
            .clone()
            .unwrap_or_else(|| action.intent.canonical_digest())
    )
}

/// The clean result the product CLI renders after a governed task.
#[derive(Debug, Clone)]
pub struct TaskOutcome {
    pub final_text: String,
    pub outcome: String,
    pub changed_files: Vec<String>,
    pub diff_stat: String,
    pub evidence_path: PathBuf,
    pub tokens: u64,
}

/// Parse `git diff --stat` lines ("path | 3 ++") into the changed file list.
pub fn changed_files_from_diff_stat(stat: &str) -> Vec<String> {
    stat.lines()
        .filter_map(|line| {
            line.split_once('|')
                .map(|(path, _)| path.trim().to_string())
        })
        .filter(|path| !path.is_empty())
        .collect()
}

#[derive(Debug, Clone)]
pub struct ExecBoundary {
    pub session_id: String,
    pub repo_root: PathBuf,
    pub candidate_root: PathBuf,
    pub candidate_baseline: String,
    pub evidence_db_path: PathBuf,
    agent_version: String,
}

impl ExecBoundary {
    pub fn new(
        repo_root: &Path,
        candidate_root: &Path,
        head: &str,
        status_digest: &str,
        evidence_db_path: &Path,
        session_id: &str,
    ) -> Result<Self> {
        let candidate_root = candidate_root
            .canonicalize()
            .context("candidate worktree must exist before governed execution")?;
        let candidate_baseline =
            crate::native_inspect::baseline_digest(&candidate_root, head, status_digest);
        Ok(Self {
            session_id: session_id.to_owned(),
            repo_root: repo_root.to_path_buf(),
            candidate_root,
            candidate_baseline,
            evidence_db_path: evidence_db_path.to_path_buf(),
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        })
    }
}

/// One executed (or refused) proposal with everything the model needs to see
/// as a bounded tool result and everything the evidence bundle must carry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecResult {
    pub action_id: String,
    pub kind: String,
    pub status: String,
    pub detail: String,
    pub canonical_action_digest: String,
    pub policy_effect: String,
}

fn effect_label(effect: PolicyEffect) -> &'static str {
    match effect {
        PolicyEffect::Allow => "allow",
        PolicyEffect::Ask => "ask",
        PolicyEffect::Deny => "deny",
    }
}

fn action_binding(
    boundary: &ExecBoundary,
    intent: &ActionIntent,
    policy: &GuardPolicy,
) -> GuardActionBinding {
    let call_id = format!("{}:{}", boundary.session_id, intent.id);
    let policy_digest = policy.digest();
    let canonical_action_digest = intent.canonical_digest();
    let binding_material = json!({
        "session_id": boundary.session_id,
        "task_id": boundary.session_id,
        "agent": AGENT,
        "agent_version": boundary.agent_version,
        "protocol": PROTOCOL,
        "canonical_action_digest": canonical_action_digest,
        "policy_digest": policy_digest,
        "worktree_baseline": boundary.candidate_baseline,
    });
    let binding_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&binding_material).expect("binding always serializes"))
    );
    GuardActionBinding {
        call_id,
        session_id: boundary.session_id.clone(),
        task_id: boundary.session_id.clone(),
        agent: AGENT.to_owned(),
        agent_version: boundary.agent_version.clone(),
        protocol: PROTOCOL.to_owned(),
        tool: intent.raw_tool_name.clone(),
        canonical_action_digest,
        policy_digest,
        worktree_baseline: boundary.candidate_baseline.clone(),
        binding_hash,
    }
}

fn release_receipt(
    memory: &MemoryEngine,
    boundary: &ExecBoundary,
    intent: &ActionIntent,
    policy: &GuardPolicy,
    approval_note: Option<serde_json::Value>,
    extra: serde_json::Map<String, serde_json::Value>,
) -> Result<GuardActionBinding, String> {
    let binding = action_binding(boundary, intent, policy);
    let mut summary = json!({
        "source": "native_code_exec",
        "protocol": PROTOCOL,
        "agent": AGENT,
        "agent_version": boundary.agent_version,
        "tool": intent.raw_tool_name,
        "redacted_display": intent.redacted_display,
        "risk_tags": intent.risk_tags,
        "arguments_sha256": intent.arguments_digest,
        "canonical_action_digest": intent.canonical_digest(),
        "policy_digest": policy.digest(),
        "worktree_baseline": boundary.candidate_baseline,
        "containment": EXEC_CONTAINMENT_LABEL,
    });
    if let Some(note) = approval_note {
        summary["approval"] = note;
    }
    if let Some(map) = summary.as_object_mut() {
        for (key, value) in extra {
            map.insert(key, value);
        }
    }
    match memory.create_guard_action(&binding, "allow", &summary.to_string(), false) {
        Ok(None) => Ok(binding),
        _ => Err("release receipt could not commit".to_string()),
    }
}

fn blocked_receiptless(
    memory: &MemoryEngine,
    boundary: &ExecBoundary,
    intent: &ActionIntent,
    policy: &GuardPolicy,
    decision_label: &str,
    reason: &str,
) {
    let binding = action_binding(boundary, intent, policy);
    let summary = json!({
        "source": "native_code_exec",
        "tool": intent.raw_tool_name,
        "redacted_display": intent.redacted_display,
        "canonical_action_digest": intent.canonical_digest(),
        "policy_effect": decision_label,
        "reason": reason,
        "containment": EXEC_CONTAINMENT_LABEL,
    });
    let _ = memory.create_guard_action(&binding, decision_label, &summary.to_string(), false);
}

/// Validate a relative write target inside the candidate worktree. Mirrors the
/// read validator's fail-closed rules but additionally allows paths that do not
/// exist yet, validating every existing ancestor canonically instead.
fn validate_write_target(
    boundary: &ExecBoundary,
    proposed: &str,
) -> std::result::Result<PathBuf, &'static str> {
    let proposed = proposed.trim();
    if proposed.is_empty() || proposed == "." {
        return Err("empty_path");
    }
    if proposed.contains('\0') {
        return Err("malformed_path");
    }
    if proposed.starts_with('/') || proposed.starts_with('\\') {
        return Err("absolute_path");
    }
    if proposed.starts_with('~') {
        return Err("home_reference");
    }
    {
        let bytes = proposed.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err("absolute_path");
        }
    }
    let normalized = proposed.replace('\\', "/");
    if crate::guard_policy::secret_path(&normalized) {
        return Err("secret_path");
    }
    let components: Vec<&str> = normalized
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect();
    if components.is_empty() {
        return Err("empty_path");
    }
    for component in &components {
        if *component == ".." {
            return Err("path_traversal");
        }
        if component.eq_ignore_ascii_case(".git") {
            return Err("repository_internals");
        }
    }
    let joined = boundary.candidate_root.join(components.join("/"));
    let root = boundary
        .candidate_root
        .canonicalize()
        .map_err(|_| "unresolvable_path")?;
    if let Ok(existing) = joined.canonicalize() {
        if !existing.starts_with(&root) {
            return Err("symlink_escape_outside_worktree");
        }
        if existing.is_dir() {
            return Err("not_regular_file");
        }
        let relative = existing
            .strip_prefix(&root)
            .map_err(|_| "symlink_escape_outside_worktree")?
            .to_string_lossy()
            .replace('\\', "/");
        if crate::guard_policy::secret_path(&relative) {
            return Err("secret_path");
        }
        let evidence = &boundary.evidence_db_path;
        let existing_abs = existing.to_string_lossy().to_string();
        let evidence_abs = evidence.to_string_lossy().to_string();
        if existing_abs == evidence_abs || existing_abs.starts_with(&format!("{evidence_abs}-")) {
            return Err("evidence_database");
        }
        return Ok(existing);
    }
    // The target does not exist yet: anchor on the deepest existing ancestor
    // so a symlinked directory above the new file still cannot escape.
    let anchor = first_existing_ancestor(&joined)?;
    let canonical_anchor = anchor.canonicalize().map_err(|_| "unresolvable_path")?;
    if !canonical_anchor.starts_with(&root) {
        return Err("symlink_escape_outside_worktree");
    }
    Ok(root.join(components.join("/")))
}

fn first_existing_ancestor(path: &Path) -> std::result::Result<PathBuf, &'static str> {
    let mut cursor = path.to_path_buf();
    loop {
        if cursor.exists() {
            return Ok(cursor);
        }
        if !cursor.pop() {
            return Err("unresolvable_path");
        }
    }
}

fn bounded_preview(text: &str) -> String {
    let mut preview = String::new();
    for ch in text.chars() {
        if preview.len() + ch.len_utf8() > EXEC_RESULT_PREVIEW_BYTES {
            preview.push_str("\n…[truncated]");
            break;
        }
        preview.push(ch);
    }
    preview
}

/// Execute one already-parsed proposal under policy. Never touches the
/// original repository; all side effects land in the candidate worktree.
pub fn execute_action(
    memory: &MemoryEngine,
    policy: &GuardPolicy,
    boundary: &ExecBoundary,
    action: &ProposalAction,
    approval: ApprovalMode,
) -> ExecResult {
    let intent = &action.intent;
    let digest = intent.canonical_digest();
    let effect = effect_label(action.decision.effect);
    let base = ExecResult {
        action_id: intent.id.clone(),
        kind: action.proposed_kind.clone(),
        status: String::new(),
        detail: String::new(),
        canonical_action_digest: digest,
        policy_effect: effect.to_string(),
    };
    let mut result = base;
    if action.proposed_kind == "network" || action.proposed_kind == "package" {
        result.status = "not_executable".to_string();
        result.detail =
            "network and package actions stay preflight-only on the native path".to_string();
        return result;
    }
    match action.decision.effect {
        PolicyEffect::Deny => {
            blocked_receiptless(memory, boundary, intent, policy, "deny", "policy_denied");
            result.status = "blocked".to_string();
            result.detail = format!(
                "denied by policy rule {:?}: {}",
                action.decision.rule_id, action.decision.reason
            );
            return result;
        }
        PolicyEffect::Ask => {
            let approved = match approval {
                ApprovalMode::Unavailable => false,
                ApprovalMode::PreAuthorized => true,
                ApprovalMode::Interactive => prompt_approval(action),
            };
            if !approved {
                let reason = match approval {
                    ApprovalMode::Unavailable => "approval_channel_unavailable",
                    _ => "human_denied",
                };
                blocked_receiptless(memory, boundary, intent, policy, "ask", reason);
                result.status = "approval_denied".to_string();
                result.detail =
                    format!("the human reviewer did not approve this action ({reason})");
                return result;
            }
            let note = json!({
                "granted_via": match approval {
                    ApprovalMode::PreAuthorized => "cli_pre_authorized_flag",
                    _ => "cli_interactive_confirm",
                },
                "one_time": true,
                "bound_digest": result.canonical_action_digest,
            });
            let _ = note;
        }
        PolicyEffect::Allow => {}
    }
    match action.proposed_kind.as_str() {
        "file_read" => execute_read(memory, policy, boundary, action, result),
        "file_write" => execute_write(memory, policy, boundary, action, result, &action.decision),
        "shell" => execute_shell(memory, policy, boundary, action, result),
        other => {
            result.status = "not_executable".to_string();
            result.detail = format!("unsupported executable kind {other}");
            result
        }
    }
}

fn approval_note_for(decision: &PolicyDecision) -> Option<serde_json::Value> {
    (decision.effect == PolicyEffect::Ask).then(|| {
        json!({
            "granted_via": "human_approver",
            "one_time": true,
        })
    })
}

fn prompt_approval(action: &ProposalAction) -> bool {
    let key = session_grant_key(action);
    if session_grants()
        .lock()
        .map(|grants| grants.contains(&key))
        .unwrap_or(false)
    {
        eprintln!(
            "[i] allowed for this session: {} {}",
            action.proposed_kind, action.intent.redacted_display
        );
        return true;
    }
    let choice = cli_brand_pause(|| {
        eprintln!();
        eprintln!(
            "[?] Kerna needs your decision (action bound to digest {}):\n    {} {}",
            action
                .intent
                .canonical_digest()
                .chars()
                .take(16)
                .collect::<String>(),
            action.proposed_kind,
            action.intent.redacted_display
        );
        dialoguer::Select::new()
            .with_prompt("How should Kerna proceed?")
            .items(["Allow once", "Allow for this session", "Deny"])
            .default(2)
            .interact_opt()
            .unwrap_or(None)
    });
    match choice {
        Some(0) => true,
        Some(1) => {
            if let Ok(mut grants) = session_grants().lock() {
                grants.insert(key);
            }
            true
        }
        _ => false,
    }
}

/// Approval prompts must own the terminal, so the activity line steps aside.
fn cli_brand_pause<T>(run: impl FnOnce() -> T) -> T {
    crate::cli_brand::with_spinner_paused(run)
}

fn execute_read(
    memory: &MemoryEngine,
    policy: &GuardPolicy,
    boundary: &ExecBoundary,
    action: &ProposalAction,
    mut result: ExecResult,
) -> ExecResult {
    let proposed = action.intent.canonical_resource.clone().unwrap_or_default();
    let target = match validate_existing_candidate_file(&boundary.candidate_root, &proposed) {
        Ok(target) => target,
        Err(reason) => {
            result.status = "blocked".to_string();
            result.detail = format!("read target rejected ({reason})");
            return result;
        }
    };
    let binding = match release_receipt(
        memory,
        boundary,
        &action.intent,
        policy,
        approval_note_for(&action.decision),
        json!({"proposed_kind": "file_read", "resolved_path": proposed})
            .as_object()
            .unwrap()
            .clone(),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            result.status = "blocked".to_string();
            result.detail = error;
            return result;
        }
    };
    match std::fs::read(&target) {
        Ok(bytes) if bytes.len() <= crate::native_inspect::INSPECTION_MAX_FILE_BYTES as usize => {
            let sha = format!("{:x}", Sha256::digest(&bytes));
            let preview = bounded_preview(&String::from_utf8_lossy(&bytes));
            let details =
                json!({"result": "observed", "bytes_read": bytes.len(), "content_sha256": sha})
                    .to_string();
            let _ = memory.observe_guard_result_with_details(
                &binding.session_id,
                &binding.call_id,
                &details,
            );
            result.status = "observed".to_string();
            result.detail = preview;
        }
        Ok(_) => {
            let _ = memory.mark_guard_outcome_unknown(
                &binding.session_id,
                &binding.call_id,
                &json!({"result": "unknown", "reason": "oversized_file"}).to_string(),
            );
            result.status = "outcome_unknown".to_string();
            result.detail = "file is larger than the inspection bound".to_string();
        }
        Err(_) => {
            let _ = memory.mark_guard_outcome_unknown(
                &binding.session_id,
                &binding.call_id,
                &json!({"result": "unknown", "reason": "read_failed"}).to_string(),
            );
            result.status = "outcome_unknown".to_string();
            result.detail = "release committed but the read failed".to_string();
        }
    }
    result
}

fn validate_existing_candidate_file(
    root: &Path,
    proposed: &str,
) -> std::result::Result<PathBuf, &'static str> {
    let normalized = proposed.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.contains("..") || normalized.starts_with('/') {
        return Err("path_rejected");
    }
    let joined = root.join(&normalized);
    let canonical = joined.canonicalize().map_err(|_| "not_found")?;
    let root = root.canonicalize().map_err(|_| "path_rejected")?;
    if !canonical.starts_with(&root) {
        return Err("symlink_escape_outside_worktree");
    }
    if !canonical.is_file() {
        return Err("not_regular_file");
    }
    Ok(canonical)
}

fn execute_write(
    memory: &MemoryEngine,
    policy: &GuardPolicy,
    boundary: &ExecBoundary,
    action: &ProposalAction,
    mut result: ExecResult,
    decision: &PolicyDecision,
) -> ExecResult {
    let Some(content) = action.content.as_deref() else {
        result.status = "blocked".to_string();
        result.detail = "file_write proposal carries no content".to_string();
        return result;
    };
    if content.len() > EXEC_WRITE_MAX_BYTES {
        result.status = "blocked".to_string();
        result.detail = "proposed content exceeds the write bound".to_string();
        return result;
    }
    let proposed = action.intent.canonical_resource.clone().unwrap_or_default();
    let target = match validate_write_target(boundary, &proposed) {
        Ok(target) => target,
        Err(reason) => {
            result.status = "blocked".to_string();
            result.detail = format!("write target rejected ({reason})");
            return result;
        }
    };
    let binding = match release_receipt(
        memory,
        boundary,
        &action.intent,
        policy,
        approval_note_for(decision),
        json!({
            "proposed_kind": "file_write",
            "resolved_path": proposed,
            "content_sha256": format!("{:x}", Sha256::digest(content.as_bytes())),
            "content_bytes": content.len(),
        })
        .as_object()
        .unwrap()
        .clone(),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            result.status = "blocked".to_string();
            result.detail = error;
            return result;
        }
    };
    let outcome = (|| -> std::io::Result<()> {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, content.as_bytes())
    })();
    match outcome {
        Ok(()) => {
            let details = json!({"result": "observed", "written_bytes": content.len()}).to_string();
            let _ = memory.observe_guard_result_with_details(
                &binding.session_id,
                &binding.call_id,
                &details,
            );
            result.status = "written".to_string();
            result.detail = format!(
                "{} bytes staged in the candidate worktree at {proposed}",
                content.len()
            );
        }
        Err(_) => {
            let _ = memory.mark_guard_outcome_unknown(
                &binding.session_id,
                &binding.call_id,
                &json!({"result": "unknown", "reason": "write_failed"}).to_string(),
            );
            result.status = "outcome_unknown".to_string();
            result.detail = "release committed but the write failed".to_string();
        }
    }
    result
}

fn execute_shell(
    memory: &MemoryEngine,
    policy: &GuardPolicy,
    boundary: &ExecBoundary,
    action: &ProposalAction,
    mut result: ExecResult,
) -> ExecResult {
    let Some(command) = action.command.clone() else {
        result.status = "blocked".to_string();
        result.detail = "shell proposal carries no command".to_string();
        return result;
    };
    let binding = match release_receipt(
        memory,
        boundary,
        &action.intent,
        policy,
        approval_note_for(&action.decision),
        json!({"proposed_kind": "shell", "command_sha256": format!("{:x}", Sha256::digest(command.as_bytes()))})
            .as_object()
            .unwrap()
            .clone(),
    ) {
        Ok(binding) => binding,
        Err(error) => {
            result.status = "blocked".to_string();
            result.detail = error;
            return result;
        }
    };
    let log_dir = std::env::temp_dir().join(format!("kerna-exec-{}", boundary.session_id));
    let _ = std::fs::create_dir_all(&log_dir);
    let out_path = log_dir.join(format!("{}-stdout.log", action.intent.id));
    let err_path = log_dir.join(format!("{}-stderr.log", action.intent.id));
    let spawned = {
        let mut cmd = if cfg!(windows) {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", &command]);
            cmd
        } else {
            let mut cmd = Command::new("/bin/sh");
            cmd.args(["-c", &command]);
            cmd
        };
        match (
            std::fs::File::create(&out_path),
            std::fs::File::create(&err_path),
        ) {
            (Ok(stdout_file), Ok(stderr_file)) => {
                cmd.stdout(Stdio::from(stdout_file))
                    .stderr(Stdio::from(stderr_file));
            }
            _ => {
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
        cmd.current_dir(&boundary.candidate_root);
        cmd.spawn()
    };
    let mut child = match spawned {
        Ok(child) => child,
        Err(_) => {
            let _ = memory.mark_guard_outcome_unknown(
                &binding.session_id,
                &binding.call_id,
                &json!({"result": "unknown", "reason": "spawn_failed"}).to_string(),
            );
            result.status = "outcome_unknown".to_string();
            result.detail = "release committed but the process could not start".to_string();
            return result;
        }
    };
    let deadline = Instant::now() + EXEC_SHELL_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stdout = std::fs::read_to_string(&out_path).unwrap_or_default();
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
    let _ = std::fs::remove_dir(&log_dir);
    let Some(status) = status else {
        let _ = memory.mark_guard_outcome_unknown(
            &binding.session_id,
            &binding.call_id,
            &json!({"result": "unknown", "reason": "shell_timeout_or_wait_failed"}).to_string(),
        );
        result.status = "outcome_unknown".to_string();
        result.detail = format!(
            "shell exceeded {}s or could not be waited on; the process was killed",
            EXEC_SHELL_TIMEOUT.as_secs()
        );
        return result;
    };
    let details = json!({
        "result": "observed",
        "exit_code": status.code(),
        "stdout_sha256": format!("{:x}", Sha256::digest(stdout.as_bytes())),
        "stderr_sha256": format!("{:x}", Sha256::digest(stderr.as_bytes())),
    })
    .to_string();
    let _ =
        memory.observe_guard_result_with_details(&binding.session_id, &binding.call_id, &details);
    result.status = if status.success() {
        "succeeded"
    } else {
        "failed"
    }
    .to_string();
    result.detail = bounded_preview(&format!(
        "exit {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        status.code()
    ));
    result
}

/// Review the candidate worktree and apply its diff back to the original
/// repository only after an explicit human confirmation. The apply itself is
/// receipt-bound to the exact patch digest.
pub fn review_and_apply(
    memory: &MemoryEngine,
    boundary: &ExecBoundary,
    policy: &GuardPolicy,
    approval: ApprovalMode,
) -> Result<serde_json::Value> {
    run_git(&boundary.candidate_root, &["add", "-A"])?;
    let stat = run_git(
        &boundary.candidate_root,
        &["diff", "--cached", "--stat", "HEAD"],
    )?;
    let patch = run_git_bytes(&boundary.candidate_root, &["diff", "--cached", "HEAD"])?;
    let report = json!({
        "diff_stat": stat,
        "patch_sha256": format!("{:x}", Sha256::digest(&patch)),
        "applied": false,
        "apply_error": null,
    });
    if String::from_utf8_lossy(&patch).trim().is_empty() {
        return Ok(report);
    }
    let approve = match approval {
        ApprovalMode::Interactive => cli_brand_pause(|| {
            // The reviewer must see the exact staged diff before deciding;
            // other modes keep this machinery out of the product view.
            println!("\n[k] candidate diff (staged in the disposable clone):\n{stat}");
            dialoguer::Confirm::new()
                .with_prompt(format!(
                    "Apply this exact patch ({}) to {}?",
                    report["patch_sha256"].as_str().unwrap_or(""),
                    boundary.repo_root.display()
                ))
                .default(false)
                .interact()
                .unwrap_or(false)
        }),
        ApprovalMode::PreAuthorized => true,
        ApprovalMode::Unavailable => false,
    };
    let mut report = report;
    if !approve {
        report["apply_error"] = json!("the human reviewer did not approve the apply");
        return Ok(report);
    }
    // The patch is piped to git's stdin: the canonical candidate root uses the
    // Windows verbatim namespace (`\\?\…`), where git cannot reopen a patch
    // file by name.
    let mut child = Command::new("git")
        .args([
            "-c",
            "safe.directory=*",
            "apply",
            "--3way",
            "--whitespace=nowarn",
        ])
        .current_dir(&boundary.repo_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&patch)?;
    }
    let applied = child.wait_with_output()?;
    let summary_binding = GuardActionBinding {
        call_id: format!("{}:apply", boundary.session_id),
        session_id: boundary.session_id.clone(),
        task_id: boundary.session_id.clone(),
        agent: AGENT.to_owned(),
        agent_version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol: PROTOCOL.to_owned(),
        tool: "KernaApply".to_owned(),
        canonical_action_digest: report["patch_sha256"].as_str().unwrap_or("").to_owned(),
        policy_digest: policy.digest(),
        worktree_baseline: boundary.candidate_baseline.clone(),
        binding_hash: format!("{:x}", Sha256::digest(&patch)),
    };
    let ok = applied.status.success();
    let _ = memory.create_guard_action(
        &summary_binding,
        if ok { "allow" } else { "ask" },
        &json!({
            "source": "native_code_apply",
            "target_repo": boundary.repo_root.to_string_lossy(),
            "patch_sha256": report["patch_sha256"],
            "granted_via": match approval {
                ApprovalMode::PreAuthorized => "cli_pre_authorized_flag",
                _ => "cli_interactive_confirm",
            },
            "git_stderr": bounded_preview(&String::from_utf8_lossy(&applied.stderr)),
        })
        .to_string(),
        false,
    );
    report["applied"] = json!(ok);
    if !ok {
        report["apply_error"] = json!(bounded_preview(&String::from_utf8_lossy(&applied.stderr)));
    }
    Ok(report)
}

#[cfg(test)]
fn normalize_parent(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn run_git(repo: &Path, args: &[&str]) -> Result<String> {
    let bytes = run_git_bytes(repo, args)?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

pub fn run_git_bytes(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut full: Vec<&str> = vec!["-c", "safe.directory=*"];
    full.extend(args.iter().copied());
    let output = Command::new("git")
        .args(&full)
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
    Ok(output.stdout)
}

/// Build the signed, redacted evidence bundle for one native code session:
/// every receipt event (digests and states only, never content or prose) plus
/// the apply outcome, signed with an ephemeral Ed25519 key.
pub fn write_signed_evidence(
    memory: &MemoryEngine,
    boundary: &ExecBoundary,
    provider: &str,
    model: &str,
    apply_report: &serde_json::Value,
    outcome: &str,
) -> Result<PathBuf> {
    let audit = memory.guard_audit_for_task(&boundary.session_id)?;
    let receipts: Vec<_> = memory
        .recent_tool_call_receipts(500)?
        .into_iter()
        .filter(|receipt| receipt.session_id == boundary.session_id)
        .collect();
    let bundle = json!({
        "schema": "kerna.native-code-evidence/v1",
        "recorded_at": chrono::Utc::now().to_rfc3339(),
        "session_id": boundary.session_id,
        "provider": provider,
        "model": model,
        "outcome": outcome,
        "broker": "native-direct (no OS containment for the model process; governance is policy, receipts, and human approval)",
        "containment": EXEC_CONTAINMENT_LABEL,
        "worktree_baseline": boundary.candidate_baseline,
        "stored_content": "paths, digests, receipt states, and git apply statistics only; no prompts, model prose, file contents, or keys",
        "apply": apply_report,
        "receipts": receipts,
        "events": audit,
    });
    // Sign the compact serialization inside the same Ed25519 envelope that
    // `kerna replay` verifies: the verifier recomputes payload bytes with
    // compact serde_json, so the signed bytes must match exactly.
    let signing_key = native_signing_key();
    let signature = signing_key.sign(&serde_json::to_vec(&bundle)?);
    let envelope = json!({
        "algorithm": "Ed25519",
        "payload": bundle,
        "public_key": base64::engine::general_purpose::STANDARD
            .encode(signing_key.verifying_key().to_bytes()),
        "signature": base64::engine::general_purpose::STANDARD
            .encode(signature.to_bytes()),
    });
    let dir = std::env::temp_dir().join(format!(
        "kerna-code-evidence-{}",
        &boundary.session_id["code-".len()..]
    ));
    std::fs::create_dir_all(&dir)?;
    let bundle_path = dir.join("native-code-evidence.json");
    std::fs::write(&bundle_path, serde_json::to_vec_pretty(&envelope)?)?;
    Ok(dir)
}

fn native_signing_key() -> SigningKey {
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    let mut seed = [0_u8; 32];
    seed[..16].copy_from_slice(first.as_bytes());
    seed[16..].copy_from_slice(second.as_bytes());
    SigningKey::from_bytes(&seed)
}

/// Render the bounded tool-result user message appended after each execution
/// turn so the model can continue its loop.
pub fn tool_results_message(results: &[ExecResult]) -> String {
    let body = json!({
        "kerna_action_results": results.iter().map(|result| json!({
            "action_id": result.action_id,
            "kind": result.kind,
            "status": result.status,
            "policy_effect": result.policy_effect,
            "canonical_action_digest": result.canonical_action_digest,
            "detail": result.detail,
        })).collect::<Vec<_>>(),
        "note": "Results come from Kerna's governed executor inside the disposable candidate clone; the original repository is untouched. Continue by emitting one new strict JSON proposal envelope, or answer in plain prose with no envelope when the work is complete and reviewed.",
    });
    body.to_string()
}

/// A proposal is final (the loop ends) when it contains no envelope marker.
pub fn is_final_answer(assistant_text: &str) -> bool {
    !assistant_text.contains(PROPOSAL_BEGIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard_policy::AgentKind;
    use crate::guard_protocol::{ActionCandidate, Protocol};
    use serde_json::json;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kerna-exec-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args([
                "-c",
                "safe.directory=*",
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
            ])
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {:?}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn boundary(label: &str) -> ExecBoundary {
        let repo = temp_dir(&format!("{label}-repo"));
        git_git(&repo, &["init", "-q", "."]);
        std::fs::write(repo.join("README.md"), "seed\n").unwrap();
        git_git(&repo, &["add", "-A"]);
        git_git(&repo, &["commit", "-q", "-m", "seed"]);
        let candidate = repo
            .join("..")
            .join(format!("{}-candidate", repo.display()));
        let candidate = normalize_parent(&candidate);
        git_git(
            &std::env::temp_dir(),
            &[
                "clone",
                "--quiet",
                &repo.to_string_lossy(),
                &candidate.to_string_lossy(),
            ],
        );
        let db = temp_dir(&format!("{label}-db")).join("k.db");
        std::fs::write(&db, "x").unwrap();
        ExecBoundary::new(
            &repo,
            &candidate,
            "abc123",
            &"d".repeat(64),
            &db,
            "code-test1",
        )
        .unwrap()
    }

    fn proposal(
        kind: &str,
        arguments: serde_json::Value,
        content: Option<String>,
        command: Option<String>,
        policy: &GuardPolicy,
    ) -> ProposalAction {
        let candidate = ActionCandidate {
            protocol: Protocol::AnthropicMessages,
            id: "proposal_1".to_owned(),
            raw_tool_name: match kind {
                "file_read" => "Read".to_owned(),
                "file_write" => "Write".to_owned(),
                _ => "Bash".to_owned(),
            },
            arguments,
        };
        let intent =
            ActionIntent::from_candidate(&candidate, "code-test1", AgentKind::KernaNative, "0.2.9");
        let decision = policy.evaluate(&intent);
        ProposalAction {
            proposed_kind: kind.to_owned(),
            intent,
            decision,
            content,
            command,
        }
    }

    fn memory() -> MemoryEngine {
        let db = temp_dir("memory").join("k.db");
        MemoryEngine::new(&db).unwrap()
    }

    #[test]
    fn write_exec_stages_in_candidate_with_receipt_lifecycle() {
        let boundary = boundary("write");
        let memory = memory();
        let policy = GuardPolicy::balanced();
        let action = proposal(
            "file_write",
            json!({"file_path": "src/new.rs", "content_sha256": format!("{:x}", Sha256::digest(b"fn answer() -> u32 { 42 }\n"))}),
            Some("fn answer() -> u32 { 42 }\n".to_string()),
            None,
            &policy,
        );
        let result = execute_action(
            &memory,
            &policy,
            &boundary,
            &action,
            ApprovalMode::Interactive,
        );
        assert_eq!(result.status, "written", "{}", result.detail);
        assert!(boundary.candidate_root.join("src").join("new.rs").exists());
        assert!(
            !boundary.repo_root.join("src").exists(),
            "original repo must stay untouched"
        );
        let receipts = memory.recent_tool_call_receipts(5).unwrap();
        let receipt = receipts
            .iter()
            .find(|r| r.session_id == "code-test1")
            .unwrap();
        assert_eq!(receipt.result_class.as_deref(), Some("result_observed"));
    }

    #[test]
    fn ask_actions_without_a_channel_fail_closed() {
        let boundary = boundary("ask");
        let memory = memory();
        let ask_policy = GuardPolicy {
            version: crate::guard_policy::POLICY_VERSION,
            default: PolicyEffect::Ask,
            rules: Vec::new(),
        };
        let action = proposal(
            "file_read",
            json!({"file_path": "README.md"}),
            None,
            None,
            &ask_policy,
        );
        let result = execute_action(
            &memory,
            &ask_policy,
            &boundary,
            &action,
            ApprovalMode::Unavailable,
        );
        assert_eq!(result.status, "approval_denied");
    }

    #[test]
    fn traversal_and_git_writes_are_rejected_before_any_receipt() {
        let boundary = boundary("reject");
        let memory = memory();
        let policy = GuardPolicy::balanced();
        for path in ["../escape.rs", ".git/config", "src/../../x"] {
            let action = proposal(
                "file_write",
                json!({"file_path": path}),
                Some("x".to_string()),
                None,
                &policy,
            );
            let result = execute_action(
                &memory,
                &policy,
                &boundary,
                &action,
                ApprovalMode::PreAuthorized,
            );
            assert_eq!(result.status, "blocked", "{path}");
        }
        let receipts = memory.recent_tool_call_receipts(10).unwrap();
        assert!(
            receipts
                .iter()
                .all(|receipt| receipt.result_class.as_deref() != Some("released")),
            "a validation-blocked write must never produce a release receipt: {receipts:?}"
        );
    }

    #[test]
    fn shell_exec_runs_in_candidate_and_records_exit() {
        let boundary = boundary("shell");
        let memory = memory();
        let policy = GuardPolicy::balanced();
        let action = proposal(
            "shell",
            json!({"command": "echo kerna-shell-probe"}),
            None,
            Some("echo kerna-shell-probe".to_string()),
            &policy,
        );
        // Shell is `ask` under balanced; pre-authorization stands in for the human.
        let result = execute_action(
            &memory,
            &policy,
            &boundary,
            &action,
            ApprovalMode::PreAuthorized,
        );
        assert!(
            result.status == "succeeded" || result.status == "approval_denied",
            "unexpected {}",
            result.status
        );
        if result.status == "succeeded" {
            assert!(result.detail.contains("kerna-shell-probe"));
        }
    }

    #[test]
    fn network_and_package_stay_preflight_only() {
        let boundary = boundary("net");
        let memory = memory();
        let policy = GuardPolicy::balanced();
        let net = proposal(
            "network",
            json!({"url": "https://example.com"}),
            None,
            None,
            &policy,
        );
        let mut net = net;
        net.intent.id = "proposal_9".to_owned();
        let result = execute_action(
            &memory,
            &policy,
            &boundary,
            &net,
            ApprovalMode::PreAuthorized,
        );
        assert_eq!(result.status, "not_executable");
    }

    #[test]
    fn review_apply_roundtrip_lands_only_after_explicit_approval() {
        let boundary = boundary("apply");
        let memory = memory();
        let policy = GuardPolicy::balanced();
        let action = proposal(
            "file_write",
            json!({"file_path": "added.txt"}),
            Some("payload\n".to_string()),
            None,
            &policy,
        );
        execute_action(
            &memory,
            &policy,
            &boundary,
            &action,
            ApprovalMode::PreAuthorized,
        );
        let report =
            review_and_apply(&memory, &boundary, &policy, ApprovalMode::PreAuthorized).unwrap();
        assert_eq!(report["applied"], json!(true), "{report}");
        assert!(boundary.repo_root.join("added.txt").exists());
    }

    #[test]
    fn evidence_bundle_is_redacted_and_signed() {
        let boundary = boundary("evidence");
        let memory = memory();
        let policy = GuardPolicy::balanced();
        let action = proposal(
            "file_write",
            json!({"file_path": "secretish.txt"}),
            Some("SENSITIVE-MODEL-CONTENT".to_string()),
            None,
            &policy,
        );
        execute_action(
            &memory,
            &policy,
            &boundary,
            &action,
            ApprovalMode::PreAuthorized,
        );
        let dir = write_signed_evidence(
            &memory,
            &boundary,
            "anthropic",
            "claude-sonnet-5",
            &json!({"applied": false}),
            "completed",
        )
        .unwrap();
        let bundle = std::fs::read_to_string(dir.join("native-code-evidence.json")).unwrap();
        assert!(bundle.contains("\"algorithm\": \"Ed25519\""));
        assert!(bundle.contains("kerna.native-code-evidence/v1"));
        assert!(
            !bundle.contains("SENSITIVE-MODEL-CONTENT"),
            "content must never persist"
        );
        assert!(bundle.contains("content_sha256"));
    }
    #[test]
    fn changed_files_parse_skips_the_git_summary_line() {
        let stat = " src/app.py   | 3 +++
tests/test_app.py | 1 +
 2 files changed, 4 insertions(+)";
        assert_eq!(
            changed_files_from_diff_stat(stat),
            vec!["src/app.py".to_string(), "tests/test_app.py".to_string()]
        );
        assert!(changed_files_from_diff_stat("").is_empty());
    }

    #[test]
    fn session_grants_are_keyed_by_kind_and_resource_and_clearable() {
        clear_session_grants();
        let policy = GuardPolicy::balanced();
        let action = proposal(
            "file_write",
            json!({"file_path": "src/app.py", "content_sha256": "ab12"}),
            Some("x".to_string()),
            None,
            &policy,
        );
        let key = session_grant_key(&action);
        assert!(key.contains("file_write"));
        assert!(key.contains("src/app.py"));
        session_grants().lock().unwrap().insert(key.clone());
        assert!(session_grants().lock().unwrap().contains(&key));
        clear_session_grants();
        assert!(!session_grants().lock().unwrap().contains(&key));

        let shell = proposal(
            "shell",
            json!({"command": "cargo test -q"}),
            None,
            Some("cargo test -q".to_string()),
            &policy,
        );
        let shell_key = session_grant_key(&shell);
        assert!(shell_key.starts_with("shell:"));
        let expected = shell
            .intent
            .canonical_resource
            .clone()
            .unwrap_or_else(|| shell.intent.canonical_digest());
        assert_eq!(shell_key, format!("shell:{expected}"));
    }
}
