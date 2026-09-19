//! Phase 7 of the native CLI harness: the first contained read-only inspection
//! path for `kerna code`.
//!
//! Eligible `file_read` proposals from a proposal preflight are converted into
//! receipt-bound reads executed by the trusted Kerna CLI process inside the
//! resolved Git worktree boundary. This is deliberately NOT a container
//! boundary: containment in this phase means canonical-path validation performed
//! by the trusted CLI, and the receipts never claim Docker or agent-container
//! isolation for this action surface.
//!
//! Lifecycle per action: `requested` -> `released` (the durable receipt commits
//! before the file is opened) -> `result_observed` (bounded read completed,
//! digest-only details). Unknown, malformed, denied, or unreceiptable
//! inspection requests fail closed. A failure after release is explicitly
//! recorded as `outcome_unknown` instead of claiming an observed result.

use crate::guard_policy::{self, ActionIntent, GuardPolicy, PolicyEffect};
use crate::memory::{GuardActionBinding, MemoryEngine};
use crate::native_code::ProposalAction;
use anyhow::{Context, Result};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Hard ceiling for any single inspected file. Larger files fail closed.
pub const INSPECTION_MAX_FILE_BYTES: u64 = 512 * 1024;
/// Portion of the bounded read that is returned in events and terminal output.
pub const INSPECTION_PREVIEW_BYTES: usize = 8 * 1024;
/// Honest containment label for this phase: trusted CLI path validation only.
pub const CONTAINMENT_LABEL: &str = "trusted_cli_worktree_read";

const AGENT: &str = "kerna_native";
const PROTOCOL: &str = "anthropic_messages";

const REASON_UNSUPPORTED_KIND: &str = "unsupported_action_kind";
const REASON_POLICY_DENIED: &str = "policy_denied";
const REASON_POLICY_REQUIRES_APPROVAL: &str = "policy_requires_approval";
const REASON_ABSOLUTE_PATH: &str = "absolute_path";
const REASON_HOME_REFERENCE: &str = "home_reference";
const REASON_EMPTY_PATH: &str = "empty_path";
const REASON_MALFORMED_PATH: &str = "malformed_path";
const REASON_PATH_TRAVERSAL: &str = "path_traversal";
const REASON_RESERVED_DEVICE: &str = "reserved_device_name";
const REPOSITORY_INTERNALS: &str = "repository_internals";
const REASON_SECRET_PATH: &str = "secret_path";
const REASON_EVIDENCE_DATABASE: &str = "evidence_database";
const REASON_NOT_FOUND: &str = "not_found";
const REASON_UNRESOLVABLE: &str = "unresolvable_path";
const REASON_OUTSIDE_WORKTREE: &str = "symlink_escape_outside_worktree";
const REASON_NOT_REGULAR: &str = "not_regular_file";
const REASON_OVERSIZED: &str = "oversized_file";
const REASON_UNRECEIPTABLE: &str = "unreceiptable";
const REASON_READ_FAILED: &str = "read_failed";
const REASON_OBSERVATION_FAILED: &str = "observation_failed";

/// Trusted per-session boundary for contained inspection: the canonical
/// worktree root, the startup baseline digest the receipts bind to, and the
/// evidence database location that must never be read back into the model
/// plane.
#[derive(Debug, Clone)]
pub struct InspectionContext {
    pub session_id: String,
    pub agent_version: String,
    repo_root: PathBuf,
    worktree_baseline: String,
    evidence_db_path: PathBuf,
}

impl InspectionContext {
    pub fn new(
        repo_root: &Path,
        head: &str,
        status_digest: &str,
        evidence_db_path: &Path,
        session_id: &str,
    ) -> Result<Self> {
        let repo_root = repo_root.canonicalize().with_context(|| {
            "could not canonicalize the worktree boundary for contained inspection"
        })?;
        let evidence_db_path = evidence_db_path
            .canonicalize()
            .unwrap_or_else(|_| evidence_db_path.to_path_buf());
        let worktree_baseline = baseline_digest(&repo_root, head, status_digest);
        Ok(Self {
            session_id: session_id.to_owned(),
            agent_version: env!("CARGO_PKG_VERSION").to_owned(),
            repo_root,
            worktree_baseline,
            evidence_db_path,
        })
    }
}

/// Resolve the trusted evidence database location so inspection can refuse to
/// read it or its SQLite sidecar files. The database is opened by the trusted
/// CLI before any inspection runs, so it normally canonicalizes.
pub fn resolve_evidence_db_path(db_path: &str) -> PathBuf {
    let raw = PathBuf::from(db_path);
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir().unwrap_or_default().join(raw)
    };
    absolute.canonicalize().unwrap_or(absolute)
}

/// Startup baseline the native inspection receipts bind to. It covers the
/// canonical repository root, HEAD, and the status digest already captured by
/// the dry-run context; raw Git output stays transient.
pub fn baseline_digest(repo_root: &Path, head: &str, status_digest: &str) -> String {
    let material = json!({
        "repo_root": repo_root.to_string_lossy(),
        "head": head,
        "status_digest_sha256": status_digest,
    });
    format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&material).expect("baseline material serializes"))
    )
}

/// Terminal outcome of one proposed `file_read` inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectionOutcome {
    /// Kerna refused to convert the proposal into a read. Nothing was opened,
    /// and no release receipt exists.
    Blocked {
        action_id: String,
        path: String,
        reason: &'static str,
        policy_effect: &'static str,
    },
    /// The release receipt committed before the read, the bounded read
    /// completed, and the receipt advanced to `result_observed`.
    Observed {
        action_id: String,
        call_id: String,
        proposed_path: String,
        canonical_path: String,
        bytes_read: u64,
        content_sha256: String,
        truncated: bool,
        content: String,
    },
    /// The release receipt committed but the read or its observation failed;
    /// the receipt is explicitly downgraded to `outcome_unknown`.
    OutcomeUnknown {
        action_id: String,
        call_id: String,
        canonical_path: String,
        reason: &'static str,
    },
}

#[derive(Debug)]
struct ValidatedTarget {
    canonical_abs: PathBuf,
    canonical_relative: String,
    file_size: u64,
}

/// Boundary validation for one proposed repository path. This performs
/// metadata-level access only; file content is never opened here. Every
/// rejection is fail-closed.
fn validate_inspection_target(
    context: &InspectionContext,
    proposed: &str,
) -> Result<ValidatedTarget, &'static str> {
    let proposed = proposed.trim();
    if proposed.is_empty() {
        return Err(REASON_EMPTY_PATH);
    }
    if proposed.contains('\0') {
        return Err(REASON_MALFORMED_PATH);
    }
    if proposed.starts_with('/') || proposed.starts_with('\\') {
        return Err(REASON_ABSOLUTE_PATH);
    }
    if proposed.starts_with('~') {
        return Err(REASON_HOME_REFERENCE);
    }
    if is_windows_absolute(proposed) {
        return Err(REASON_ABSOLUTE_PATH);
    }
    let normalized = proposed.replace('\\', "/");
    if guard_policy::secret_path(&normalized) {
        return Err(REASON_SECRET_PATH);
    }
    let components: Vec<&str> = normalized
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect();
    if components.is_empty() {
        return Err(REASON_EMPTY_PATH);
    }
    for component in &components {
        if *component == ".." {
            return Err(REASON_PATH_TRAVERSAL);
        }
        if component.eq_ignore_ascii_case(".git") {
            return Err(REPOSITORY_INTERNALS);
        }
        if is_windows_device_name(component) {
            return Err(REASON_RESERVED_DEVICE);
        }
    }
    let joined = context.repo_root.join(components.join("/"));
    let canonical_abs = joined.canonicalize().map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => REASON_NOT_FOUND,
        _ => REASON_UNRESOLVABLE,
    })?;
    if !is_strictly_inside(&canonical_abs, &context.repo_root) {
        // The lexical path stayed inside, so only a symlink or junction can
        // explain resolution outside the worktree.
        return Err(REASON_OUTSIDE_WORKTREE);
    }
    let canonical_relative = canonical_abs
        .strip_prefix(&context.repo_root)
        .expect("containment was just verified")
        .to_string_lossy()
        .replace('\\', "/");
    for component in canonical_relative.split('/') {
        if component.eq_ignore_ascii_case(".git") {
            return Err(REPOSITORY_INTERNALS);
        }
    }
    if guard_policy::secret_path(&canonical_relative) {
        return Err(REASON_SECRET_PATH);
    }
    if is_evidence_database(&canonical_abs, &context.evidence_db_path) {
        return Err(REASON_EVIDENCE_DATABASE);
    }
    let metadata = std::fs::metadata(&canonical_abs).map_err(|_| REASON_NOT_FOUND)?;
    if !metadata.is_file() {
        return Err(REASON_NOT_REGULAR);
    }
    if metadata.len() > INSPECTION_MAX_FILE_BYTES {
        return Err(REASON_OVERSIZED);
    }
    Ok(ValidatedTarget {
        canonical_abs,
        canonical_relative,
        file_size: metadata.len(),
    })
}

/// Bounded read of an already-validated target. The file is opened by canonical
/// path and read through a size-capped handle, so the bytes returned always
/// come from the same opened file.
fn read_bounded(canonical_abs: &Path) -> Result<BoundedRead, &'static str> {
    let file = File::open(canonical_abs).map_err(|_| REASON_READ_FAILED)?;
    let metadata = file.metadata().map_err(|_| REASON_READ_FAILED)?;
    if !metadata.is_file() {
        return Err(REASON_NOT_REGULAR);
    }
    if metadata.len() > INSPECTION_MAX_FILE_BYTES {
        return Err(REASON_OVERSIZED);
    }
    let mut bytes = Vec::new();
    file.take(INSPECTION_MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| REASON_READ_FAILED)?;
    if bytes.len() as u64 > INSPECTION_MAX_FILE_BYTES {
        return Err(REASON_OVERSIZED);
    }
    let bytes_read = bytes.len() as u64;
    let content_sha256 = format!("{:x}", Sha256::digest(&bytes));
    let truncated = bytes.len() > INSPECTION_PREVIEW_BYTES;
    let preview_bytes = &bytes[..bytes.len().min(INSPECTION_PREVIEW_BYTES)];
    let content = String::from_utf8_lossy(preview_bytes).to_string();
    Ok(BoundedRead {
        bytes_read,
        content_sha256,
        truncated,
        content,
    })
}

struct BoundedRead {
    bytes_read: u64,
    content_sha256: String,
    truncated: bool,
    content: String,
}

/// Convert one preflight `file_read` proposal into a receipt-bound contained
/// read. The release receipt must commit before the file is opened; if the
/// receipt cannot commit, the read never happens.
pub fn inspect_file_read(
    memory: &MemoryEngine,
    policy: &GuardPolicy,
    context: &InspectionContext,
    action: &ProposalAction,
) -> InspectionOutcome {
    let action_id = action.intent.id.clone();
    let proposed_path = action.intent.canonical_resource.clone().unwrap_or_default();
    if action.proposed_kind != "file_read" {
        return InspectionOutcome::Blocked {
            action_id,
            path: proposed_path,
            reason: REASON_UNSUPPORTED_KIND,
            policy_effect: effect_label(action.decision.effect),
        };
    }
    match action.decision.effect {
        PolicyEffect::Deny => {
            return InspectionOutcome::Blocked {
                action_id,
                path: proposed_path,
                reason: REASON_POLICY_DENIED,
                policy_effect: "deny",
            };
        }
        PolicyEffect::Ask => {
            return InspectionOutcome::Blocked {
                action_id,
                path: proposed_path,
                reason: REASON_POLICY_REQUIRES_APPROVAL,
                policy_effect: "ask",
            };
        }
        PolicyEffect::Allow => {}
    }
    let target = match validate_inspection_target(context, &proposed_path) {
        Ok(target) => target,
        Err(reason) => {
            return InspectionOutcome::Blocked {
                action_id,
                path: proposed_path,
                reason,
                policy_effect: "allow",
            };
        }
    };
    let binding = inspection_binding(context, &action.intent, policy);
    let summary = inspection_summary(context, &action.intent, policy, &target);
    match memory.create_guard_action(&binding, "allow", &summary, false) {
        Ok(None) => {}
        _ => {
            return InspectionOutcome::Blocked {
                action_id,
                path: proposed_path,
                reason: REASON_UNRECEIPTABLE,
                policy_effect: "allow",
            };
        }
    }
    let read = match read_bounded(&target.canonical_abs) {
        Ok(read) => read,
        Err(reason) => {
            let _ = memory.mark_guard_outcome_unknown(
                &binding.session_id,
                &binding.call_id,
                &unknown_payload(reason),
            );
            return InspectionOutcome::OutcomeUnknown {
                action_id,
                call_id: binding.call_id,
                canonical_path: target.canonical_relative,
                reason,
            };
        }
    };
    let details = json!({
        "result": "observed",
        "bytes_read": read.bytes_read,
        "content_sha256": read.content_sha256,
        "truncated": read.truncated,
    })
    .to_string();
    match memory.observe_guard_result_with_details(&binding.session_id, &binding.call_id, &details)
    {
        Ok(true) => InspectionOutcome::Observed {
            action_id,
            call_id: binding.call_id,
            proposed_path,
            canonical_path: target.canonical_relative,
            bytes_read: read.bytes_read,
            content_sha256: read.content_sha256,
            truncated: read.truncated,
            content: read.content,
        },
        _ => {
            let _ = memory.mark_guard_outcome_unknown(
                &binding.session_id,
                &binding.call_id,
                &unknown_payload(REASON_OBSERVATION_FAILED),
            );
            InspectionOutcome::OutcomeUnknown {
                action_id,
                call_id: binding.call_id,
                canonical_path: target.canonical_relative,
                reason: REASON_OBSERVATION_FAILED,
            }
        }
    }
}

fn unknown_payload(reason: &str) -> String {
    json!({"result": "unknown", "reason": reason}).to_string()
}

fn effect_label(effect: PolicyEffect) -> &'static str {
    match effect {
        PolicyEffect::Allow => "allow",
        PolicyEffect::Ask => "ask",
        PolicyEffect::Deny => "deny",
    }
}

/// Build the durable binding for one native inspection read. The binding covers
/// session, task scope, agent identity, protocol, canonical action digest,
/// policy digest, and the startup worktree baseline, mirroring the guard
/// server's binding material.
fn inspection_binding(
    context: &InspectionContext,
    intent: &ActionIntent,
    policy: &GuardPolicy,
) -> GuardActionBinding {
    let call_id = format!("{}:{}", context.session_id, intent.id);
    let policy_digest = policy.digest();
    let canonical_action_digest = intent.canonical_digest();
    let binding_material = json!({
        "session_id": context.session_id,
        "task_id": context.session_id,
        "agent": AGENT,
        "agent_version": context.agent_version,
        "protocol": PROTOCOL,
        "canonical_action_digest": canonical_action_digest,
        "policy_digest": policy_digest,
        "worktree_baseline": context.worktree_baseline,
    });
    let binding_hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&binding_material).expect("binding always serializes"))
    );
    GuardActionBinding {
        call_id,
        session_id: context.session_id.clone(),
        task_id: context.session_id.clone(),
        agent: AGENT.to_owned(),
        agent_version: context.agent_version.clone(),
        protocol: PROTOCOL.to_owned(),
        tool: intent.raw_tool_name.clone(),
        canonical_action_digest,
        policy_digest,
        worktree_baseline: context.worktree_baseline.clone(),
        binding_hash,
    }
}

/// Metadata-only receipt summary. Proposed and resolved paths, sizes, digests,
/// and policy identity are recorded; file content never enters the summary.
fn inspection_summary(
    context: &InspectionContext,
    intent: &ActionIntent,
    policy: &GuardPolicy,
    target: &ValidatedTarget,
) -> String {
    json!({
        "source": "native_code_proposal",
        "proposed_kind": "file_read",
        "protocol": PROTOCOL,
        "agent": AGENT,
        "agent_version": context.agent_version,
        "tool": intent.raw_tool_name,
        "kind": "file_read",
        "proposed_path": intent.canonical_resource,
        "resolved_path": target.canonical_relative,
        "file_size_bytes": target.file_size,
        "redacted_display": intent.redacted_display,
        "risk_tags": intent.risk_tags,
        "arguments_sha256": intent.arguments_digest,
        "canonical_action_digest": intent.canonical_digest(),
        "policy_digest": policy.digest(),
        "worktree_baseline": context.worktree_baseline,
        "policy_effect": "allow",
        "policy_reason": "native inspection release requires policy allow",
        "containment": CONTAINMENT_LABEL,
    })
    .to_string()
}

fn is_windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn is_windows_device_name(component: &str) -> bool {
    let base = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    match base.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            let bytes = base.as_bytes();
            (base.starts_with("COM") || base.starts_with("LPT"))
                && bytes.len() == 4
                && bytes[3].is_ascii_digit()
                && bytes[3] != b'0'
        }
    }
}

fn is_strictly_inside(child: &Path, root: &Path) -> bool {
    match child.strip_prefix(root) {
        Ok(rest) => !rest.as_os_str().is_empty(),
        Err(_) => false,
    }
}

fn is_evidence_database(canonical: &Path, evidence_db: &Path) -> bool {
    let canonical = canonical.to_string_lossy();
    let evidence_db = evidence_db.to_string_lossy();
    canonical == evidence_db
        || canonical == format!("{evidence_db}-wal")
        || canonical == format!("{evidence_db}-shm")
        || canonical == format!("{evidence_db}-journal")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guard_policy::AgentKind;
    use crate::guard_protocol::{ActionCandidate, Protocol};
    use serde_json::json;

    fn temp_repo(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kerna-inspect-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp repo must be created");
        dir
    }

    fn test_context(repo: &Path) -> InspectionContext {
        let evidence_db = repo.join("kerna.db");
        std::fs::write(&evidence_db, "evidence").expect("evidence db fixture");
        InspectionContext::new(
            repo,
            "abc123456789",
            &"d".repeat(64),
            &evidence_db,
            "session-1",
        )
        .expect("context must build")
    }

    fn file_read_action(session_id: &str, path: &str, policy: &GuardPolicy) -> ProposalAction {
        let candidate = ActionCandidate {
            protocol: Protocol::AnthropicMessages,
            id: "proposal_1".to_owned(),
            raw_tool_name: "Read".to_owned(),
            arguments: json!({ "file_path": path }),
        };
        let intent =
            ActionIntent::from_candidate(&candidate, session_id, AgentKind::KernaNative, "0.2.9");
        let decision = policy.evaluate(&intent);
        ProposalAction {
            proposed_kind: "file_read".to_owned(),
            intent,
            decision,
            content: None,
            command: None,
        }
    }

    fn validation_error(context: &InspectionContext, proposed: &str) -> &'static str {
        validate_inspection_target(context, proposed).unwrap_err()
    }

    #[test]
    fn valid_relative_file_is_contained_and_bounded() {
        let repo = temp_repo("valid");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let body = "fn main() { println!(\"inspect me\"); }\n";
        std::fs::write(repo.join("src").join("lib.rs"), body).unwrap();
        let context = test_context(&repo);

        let target = validate_inspection_target(&context, "src/lib.rs").expect("valid target");
        assert_eq!(target.canonical_relative, "src/lib.rs");
        assert_eq!(target.file_size, body.len() as u64);

        let read = read_bounded(&target.canonical_abs).expect("bounded read");
        assert_eq!(read.bytes_read, body.len() as u64);
        assert_eq!(
            read.content_sha256,
            format!("{:x}", Sha256::digest(body.as_bytes()))
        );
        assert!(!read.truncated);
        assert_eq!(read.content, body);

        // Backslash-separated proposals resolve to the same canonical target.
        assert!(validate_inspection_target(&context, "src\\lib.rs").is_ok());
    }

    #[test]
    fn absolute_and_home_paths_fail_closed() {
        let repo = temp_repo("absolute");
        let context = test_context(&repo);
        for proposed in ["/etc/passwd", "\\server\\share\\file", "\\\\?\\C:\\x"] {
            assert_eq!(validation_error(&context, proposed), REASON_ABSOLUTE_PATH);
        }
        for proposed in ["C:/Windows/win.ini", "c:\\Windows\\win.ini", "F:", "z:repo"] {
            assert_eq!(validation_error(&context, proposed), REASON_ABSOLUTE_PATH);
        }
        for proposed in ["~/notes.txt", "~"] {
            assert_eq!(validation_error(&context, proposed), REASON_HOME_REFERENCE);
        }
    }

    #[test]
    fn traversal_and_empty_paths_fail_closed() {
        let repo = temp_repo("traversal");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src").join("lib.rs"), "x").unwrap();
        let context = test_context(&repo);
        for proposed in ["../sibling.txt", "src/../../lib.rs", "..", "a/./../b"] {
            assert_eq!(validation_error(&context, proposed), REASON_PATH_TRAVERSAL);
        }
        for proposed in ["", "   ", ".", "./"] {
            assert_eq!(validation_error(&context, proposed), REASON_EMPTY_PATH);
        }
        assert_eq!(
            validation_error(&context, "bad\0path"),
            REASON_MALFORMED_PATH
        );
    }

    #[test]
    fn secret_paths_fail_closed() {
        let repo = temp_repo("secrets");
        std::fs::write(repo.join(".env"), "SECRET=1").unwrap();
        std::fs::create_dir_all(repo.join(".ssh")).unwrap();
        std::fs::write(repo.join(".ssh").join("id_rsa"), "key").unwrap();
        let context = test_context(&repo);
        for proposed in [
            ".env",
            ".env.production",
            ".ssh/id_rsa",
            "backup/id_rsa_old",
        ] {
            assert_eq!(
                validate_inspection_target(&context, proposed).unwrap_err(),
                REASON_SECRET_PATH
            );
        }
        #[cfg(unix)]
        {
            // A benign name resolving to a secret file must still be rejected
            // after canonicalization.
            let _ = std::os::unix::fs::symlink(repo.join(".env"), repo.join("notes.txt"));
            if repo.join("notes.txt").exists() {
                assert_eq!(validation_error(&context, "notes.txt"), REASON_SECRET_PATH);
            }
        }
    }

    #[test]
    fn git_internals_and_device_names_fail_closed() {
        let repo = temp_repo("gitdevice");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git").join("config"), "[core]").unwrap();
        let context = test_context(&repo);
        for proposed in [".git/config", ".GIT/config", ".git"] {
            assert_eq!(validation_error(&context, proposed), REPOSITORY_INTERNALS);
        }
        for proposed in ["NUL", "CON.txt", "com1", "lpt9.log"] {
            assert_eq!(validation_error(&context, proposed), REASON_RESERVED_DEVICE);
        }
    }

    #[test]
    fn evidence_database_paths_fail_closed() {
        let repo = temp_repo("evidence");
        let context = test_context(&repo);
        // The evidence database and its live SQLite sidecars exist.
        std::fs::write(repo.join("kerna.db-wal"), "wal").unwrap();
        std::fs::write(repo.join("kerna.db-shm"), "shm").unwrap();
        std::fs::write(repo.join("kerna.db-journal"), "journal").unwrap();
        for proposed in [
            "kerna.db",
            "kerna.db-wal",
            "kerna.db-shm",
            "kerna.db-journal",
        ] {
            let error = validate_inspection_target(&context, proposed).unwrap_err();
            assert_eq!(
                error, REASON_EVIDENCE_DATABASE,
                "{proposed} must be blocked"
            );
        }
        // Ordinary repository files beside the database still pass.
        std::fs::write(repo.join("README.md"), "readme").unwrap();
        assert!(validate_inspection_target(&context, "README.md").is_ok());
    }

    #[test]
    fn missing_oversized_and_non_regular_targets_fail_closed() {
        let repo = temp_repo("missing");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(
            repo.join("oversized.bin"),
            vec![0u8; INSPECTION_MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let context = test_context(&repo);
        assert_eq!(validation_error(&context, "missing.txt"), REASON_NOT_FOUND);
        assert_eq!(validation_error(&context, "src"), REASON_NOT_REGULAR);
        assert_eq!(
            validation_error(&context, "oversized.bin"),
            REASON_OVERSIZED
        );
    }

    #[test]
    fn symlink_escape_fails_closed() {
        let repo = temp_repo("symlink");
        let outside = temp_repo("symlink-outside");
        std::fs::write(outside.join("secret.txt"), "host secret").unwrap();
        std::fs::create_dir_all(repo.join("docs")).unwrap();
        std::fs::write(repo.join("docs").join("inside.txt"), "repo file").unwrap();
        let link = repo.join("docs").join("escape_link");
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&outside, &link);
            if link.exists() {
                let context = test_context(&repo);
                assert_eq!(
                    validation_error(&context, "docs/escape_link/secret.txt"),
                    REASON_OUTSIDE_WORKTREE
                );
            }
        }
        #[cfg(windows)]
        {
            // Symlink creation on Windows requires elevated privileges or
            // developer mode; the canonicalize-based containment check is the
            // same code path exercised by the Unix branch and by every
            // successful validation above.
            let _ = link;
        }
    }

    #[test]
    fn bounded_read_truncates_preview_but_digests_everything() {
        let repo = temp_repo("preview");
        let body = "k".repeat(INSPECTION_PREVIEW_BYTES + 100);
        std::fs::write(repo.join("big.txt"), &body).unwrap();
        let context = test_context(&repo);
        let target = validate_inspection_target(&context, "big.txt").unwrap();
        let read = read_bounded(&target.canonical_abs).unwrap();
        assert_eq!(read.bytes_read, body.len() as u64);
        assert_eq!(
            read.content_sha256,
            format!("{:x}", Sha256::digest(body.as_bytes()))
        );
        assert!(read.truncated);
        assert_eq!(read.content.len(), INSPECTION_PREVIEW_BYTES);
        assert!(read.content.chars().all(|character| character == 'k'));
    }

    #[test]
    fn full_lifecycle_commits_receipt_before_read_and_persists_no_content() {
        let repo = temp_repo("lifecycle");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let body = "pub fn answer() -> u32 { 42 }";
        std::fs::write(repo.join("src").join("lib.rs"), body).unwrap();
        let db_path = std::env::temp_dir().join(format!(
            "kerna-inspect-lifecycle-{}.db",
            uuid::Uuid::new_v4()
        ));
        let memory = MemoryEngine::new(&db_path).expect("test evidence db");
        let context = test_context(&repo);
        let policy = GuardPolicy::balanced();
        let action = file_read_action("session-1", "src/lib.rs", &policy);

        let outcome = inspect_file_read(&memory, &policy, &context, &action);
        let InspectionOutcome::Observed {
            action_id,
            call_id,
            proposed_path,
            canonical_path,
            bytes_read,
            content_sha256,
            truncated,
            content,
        } = outcome
        else {
            panic!("eligible read must be observed");
        };
        assert_eq!(action_id, "proposal_1");
        assert_eq!(call_id, "session-1:proposal_1");
        assert_eq!(proposed_path, "src/lib.rs");
        assert_eq!(canonical_path, "src/lib.rs");
        assert_eq!(bytes_read, body.len() as u64);
        assert_eq!(
            content_sha256,
            format!("{:x}", Sha256::digest(body.as_bytes()))
        );
        assert!(!truncated);
        assert_eq!(content, body);

        let receipts = memory.recent_tool_call_receipts(10).unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].call_id, call_id);
        assert_eq!(receipts[0].result_class.as_deref(), Some("result_observed"));
        assert_eq!(receipts[0].tool, "Read");
        assert_eq!(receipts[0].session_id, "session-1");

        let audit = memory.guard_audit_for_task("session-1").unwrap();
        let event_types: Vec<&str> = audit
            .iter()
            .map(|event| event["event_type"].as_str().unwrap())
            .collect();
        assert_eq!(
            event_types,
            vec!["requested", "released", "result_observed"]
        );
        let serialized = serde_json::to_string(&audit).unwrap();
        assert!(
            !serialized.contains(body),
            "raw file content must never persist in receipts"
        );
        assert!(serialized.contains("content_sha256"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn policy_denied_or_ask_reads_stay_preflight_only() {
        let repo = temp_repo("policy");
        std::fs::write(repo.join("README.md"), "readme").unwrap();
        let db_path =
            std::env::temp_dir().join(format!("kerna-inspect-policy-{}.db", uuid::Uuid::new_v4()));
        let memory = MemoryEngine::new(&db_path).expect("test evidence db");
        let context = test_context(&repo);
        let deny_policy = GuardPolicy {
            version: guard_policy::POLICY_VERSION,
            default: PolicyEffect::Deny,
            rules: Vec::new(),
        };
        let ask_policy = GuardPolicy {
            version: guard_policy::POLICY_VERSION,
            default: PolicyEffect::Ask,
            rules: Vec::new(),
        };

        let denied = file_read_action("session-1", "README.md", &deny_policy);
        match inspect_file_read(&memory, &deny_policy, &context, &denied) {
            InspectionOutcome::Blocked {
                reason,
                policy_effect,
                ..
            } => {
                assert_eq!(reason, REASON_POLICY_DENIED);
                assert_eq!(policy_effect, "deny");
            }
            other => panic!("denied read must block, got {other:?}"),
        }
        let held = file_read_action("session-1", "README.md", &ask_policy);
        match inspect_file_read(&memory, &ask_policy, &context, &held) {
            InspectionOutcome::Blocked {
                reason,
                policy_effect,
                ..
            } => {
                assert_eq!(reason, REASON_POLICY_REQUIRES_APPROVAL);
                assert_eq!(policy_effect, "ask");
            }
            other => panic!("ask read must block, got {other:?}"),
        }
        // A traversal proposal is denied by the immutable policy invariant even
        // under the balanced policy, before any boundary work.
        let balanced = GuardPolicy::balanced();
        let escape = file_read_action("session-1", "../outside.txt", &balanced);
        match inspect_file_read(&memory, &balanced, &context, &escape) {
            InspectionOutcome::Blocked { reason, .. } => {
                assert_eq!(reason, REASON_POLICY_DENIED);
            }
            other => panic!("traversal read must block, got {other:?}"),
        }
        assert!(
            memory.recent_tool_call_receipts(10).unwrap().is_empty(),
            "blocked proposals must not create receipts"
        );
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn non_file_read_actions_are_refused_by_the_inspection_path() {
        let repo = temp_repo("kind");
        let db_path =
            std::env::temp_dir().join(format!("kerna-inspect-kind-{}.db", uuid::Uuid::new_v4()));
        let memory = MemoryEngine::new(&db_path).expect("test evidence db");
        let context = test_context(&repo);
        let policy = GuardPolicy::balanced();
        let candidate = ActionCandidate {
            protocol: Protocol::AnthropicMessages,
            id: "proposal_1".to_owned(),
            raw_tool_name: "Bash".to_owned(),
            arguments: json!({ "command": "cargo test" }),
        };
        let intent =
            ActionIntent::from_candidate(&candidate, "session-1", AgentKind::KernaNative, "0.2.9");
        let decision = policy.evaluate(&intent);
        let shell_action = ProposalAction {
            proposed_kind: "shell".to_owned(),
            intent,
            decision,
            content: None,
            command: None,
        };
        match inspect_file_read(&memory, &policy, &context, &shell_action) {
            InspectionOutcome::Blocked { reason, .. } => {
                assert_eq!(reason, REASON_UNSUPPORTED_KIND);
            }
            other => panic!("shell proposal must be refused, got {other:?}"),
        }
        assert!(memory.recent_tool_call_receipts(10).unwrap().is_empty());
        let _ = std::fs::remove_file(&db_path);
    }
}
