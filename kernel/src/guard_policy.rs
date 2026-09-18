use crate::config::PermissionRule;
use crate::guard_protocol::{ActionCandidate, Protocol};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fmt;
use std::path::Path;

pub const POLICY_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    KernaNative,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    FileRead,
    FileWrite,
    Shell,
    Network,
    External,
    Mcp,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActionIntent {
    pub id: String,
    pub session_id: String,
    pub agent: AgentKind,
    pub agent_version: String,
    pub protocol: Protocol,
    pub raw_tool_name: String,
    pub kind: ActionKind,
    pub canonical_resource: Option<String>,
    pub redacted_display: String,
    pub risk_tags: Vec<String>,
    pub arguments_digest: String,
    pub executable: Option<String>,
    pub domain: Option<String>,
}

impl ActionIntent {
    pub fn from_candidate(
        candidate: &ActionCandidate,
        session_id: impl Into<String>,
        agent: AgentKind,
        agent_version: impl Into<String>,
    ) -> Self {
        let kind = action_kind(&candidate.raw_tool_name);
        let command = command(candidate);
        let executable = command.as_deref().and_then(command_executable);
        let resources = canonical_resources(kind, candidate, executable.as_deref());
        let canonical_resource = (resources.len() == 1).then(|| resources[0].clone());
        let domain = network_domain(candidate);
        let mut risk_tags = risk_tags(
            candidate,
            kind,
            command.as_deref(),
            executable.as_deref(),
            &resources,
        );
        risk_tags.sort();
        risk_tags.dedup();
        let redacted_display = canonical_resource
            .as_deref()
            .map(|resource| format!("{}: {resource}", candidate.raw_tool_name))
            .unwrap_or_else(|| candidate.raw_tool_name.clone());
        Self {
            id: candidate.id.clone(),
            session_id: session_id.into(),
            agent,
            agent_version: agent_version.into(),
            protocol: candidate.protocol,
            raw_tool_name: candidate.raw_tool_name.clone(),
            kind,
            canonical_resource,
            redacted_display,
            risk_tags,
            arguments_digest: arguments_digest(&candidate.arguments),
            executable,
            domain,
        }
    }

    /// Digest only the normalized action contract. Raw model arguments never enter this digest;
    /// their deterministic SHA-256 is already represented by `arguments_digest`.
    pub fn canonical_digest(&self) -> String {
        stable_digest(self)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEffect {
    Allow,
    Ask,
    Deny,
}

impl PolicyEffect {
    fn from_legacy(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "allow" | "auto_approve" => Self::Allow,
            "ask" | "require_confirmation" => Self::Ask,
            _ => Self::Deny,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRule {
    pub id: String,
    pub effect: PolicyEffect,
    #[serde(default)]
    pub agent: Option<AgentKind>,
    #[serde(default)]
    pub kind: Option<ActionKind>,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub risk_tag: Option<String>,
}

impl PolicyRule {
    fn matches(&self, intent: &ActionIntent) -> bool {
        self.agent.is_none_or(|value| value == intent.agent)
            && self.kind.is_none_or(|value| value == intent.kind)
            && self
                .tool
                .as_deref()
                .is_none_or(|value| value == intent.raw_tool_name)
            && self.path.as_deref().is_none_or(|pattern| {
                intent
                    .canonical_resource
                    .as_deref()
                    .is_some_and(|value| glob_matches(pattern, value))
            })
            && self
                .executable
                .as_deref()
                .is_none_or(|value| intent.executable.as_deref().is_some_and(|got| got == value))
            && self.domain.as_deref().is_none_or(|pattern| {
                intent
                    .domain
                    .as_deref()
                    .is_some_and(|value| glob_matches(pattern, value))
            })
            && self
                .risk_tag
                .as_deref()
                .is_none_or(|value| intent.risk_tags.iter().any(|candidate| candidate == value))
    }

    fn has_selector(&self) -> bool {
        self.agent.is_some()
            || self.kind.is_some()
            || self.tool.is_some()
            || self.path.is_some()
            || self.executable.is_some()
            || self.domain.is_some()
            || self.risk_tag.is_some()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuardPolicy {
    pub version: u32,
    pub default: PolicyEffect,
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

impl GuardPolicy {
    pub fn parse_toml(text: &str) -> Result<Self, PolicyError> {
        let policy: Self =
            toml::from_str(text).map_err(|error| PolicyError::Parse(error.to_string()))?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn load(path: &Path) -> Result<Self, PolicyError> {
        let text =
            std::fs::read_to_string(path).map_err(|error| PolicyError::Read(error.to_string()))?;
        Self::parse_toml(&text)
    }

    /// Digest the validated, ordered policy contract for approval binding.
    pub fn digest(&self) -> String {
        stable_digest(self)
    }

    pub fn validate(&self) -> Result<(), PolicyError> {
        if self.version != POLICY_VERSION {
            return Err(PolicyError::UnsupportedVersion(self.version));
        }
        let mut ids = HashSet::new();
        for rule in &self.rules {
            if rule.id.trim().is_empty() {
                return Err(PolicyError::InvalidRule("rule id is empty".to_owned()));
            }
            if !ids.insert(rule.id.as_str()) {
                return Err(PolicyError::InvalidRule(format!(
                    "duplicate rule id {}",
                    rule.id
                )));
            }
            if !rule.has_selector() {
                return Err(PolicyError::InvalidRule(format!(
                    "rule {} has no selector",
                    rule.id
                )));
            }
            for (name, value) in [
                ("tool", rule.tool.as_deref()),
                ("path", rule.path.as_deref()),
                ("executable", rule.executable.as_deref()),
                ("domain", rule.domain.as_deref()),
                ("risk_tag", rule.risk_tag.as_deref()),
            ] {
                if value.is_some_and(|value| value.trim().is_empty()) {
                    return Err(PolicyError::InvalidRule(format!(
                        "rule {} has an empty {name} selector",
                        rule.id
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn evaluate(&self, intent: &ActionIntent) -> PolicyDecision {
        if let Some(tag) = intent.risk_tags.iter().find(|tag| {
            matches!(
                tag.as_str(),
                "secret_path" | "docker_access" | "host_escape" | "malformed_arguments"
            )
        }) {
            return PolicyDecision {
                effect: PolicyEffect::Deny,
                rule_id: Some(format!("kerna.invariant.{tag}")),
                reason: format!("immutable Kerna safety invariant: {tag}"),
            };
        }

        let matched = self.rules.iter().find(|rule| rule.matches(intent));
        let (mut effect, rule_id, mut reason) = matched.map_or_else(
            || {
                (
                    self.default,
                    None,
                    format!("policy default: {:?}", self.default),
                )
            },
            |rule| {
                (
                    rule.effect,
                    Some(rule.id.clone()),
                    format!("matched policy rule {}", rule.id),
                )
            },
        );
        if effect == PolicyEffect::Allow
            && intent.risk_tags.iter().any(|tag| {
                matches!(
                    tag.as_str(),
                    "destructive" | "external_side_effect" | "multi_resource"
                )
            })
        {
            effect = PolicyEffect::Ask;
            reason = "built-in safety floor requires approval".to_owned();
        }
        PolicyDecision {
            effect,
            rule_id,
            reason,
        }
    }

    pub fn from_legacy_permissions(rules: &[PermissionRule], default: PolicyEffect) -> Self {
        let mut exact = Vec::new();
        let mut wildcard = Vec::new();
        for (index, rule) in rules.iter().enumerate() {
            let converted = PolicyRule {
                id: format!("legacy.{index}"),
                effect: PolicyEffect::from_legacy(&rule.action),
                agent: None,
                kind: None,
                tool: (rule.tool != "*").then(|| rule.tool.clone()),
                path: None,
                executable: None,
                domain: None,
                risk_tag: None,
            };
            if rule.tool == "*" {
                wildcard.push(converted);
            } else {
                exact.push(converted);
            }
        }
        exact.extend(wildcard);
        Self {
            version: POLICY_VERSION,
            default,
            rules: exact,
        }
    }

    pub fn balanced() -> Self {
        let rule = |id: &str, effect, kind, risk_tag: Option<&str>| -> PolicyRule {
            PolicyRule {
                id: id.to_owned(),
                effect,
                agent: None,
                kind,
                tool: None,
                path: None,
                executable: None,
                domain: None,
                risk_tag: risk_tag.map(str::to_owned),
            }
        };
        Self {
            version: POLICY_VERSION,
            default: PolicyEffect::Ask,
            rules: vec![
                rule(
                    "balanced.deny_unknown",
                    PolicyEffect::Deny,
                    Some(ActionKind::Unknown),
                    None,
                ),
                rule(
                    "balanced.deny_external",
                    PolicyEffect::Deny,
                    Some(ActionKind::External),
                    None,
                ),
                rule(
                    "balanced.deny_mcp",
                    PolicyEffect::Deny,
                    Some(ActionKind::Mcp),
                    None,
                ),
                rule(
                    "balanced.ask_network",
                    PolicyEffect::Ask,
                    Some(ActionKind::Network),
                    None,
                ),
                rule(
                    "balanced.ask_network_commands",
                    PolicyEffect::Ask,
                    None,
                    Some("network_access"),
                ),
                rule(
                    "balanced.ask_compound_commands",
                    PolicyEffect::Ask,
                    None,
                    Some("compound_command"),
                ),
                rule(
                    "balanced.ask_install",
                    PolicyEffect::Ask,
                    None,
                    Some("dependency_install"),
                ),
                rule(
                    "balanced.allow_offline",
                    PolicyEffect::Allow,
                    None,
                    Some("offline_build_or_test"),
                ),
                rule(
                    "balanced.allow_read",
                    PolicyEffect::Allow,
                    Some(ActionKind::FileRead),
                    None,
                ),
                rule(
                    "balanced.allow_write",
                    PolicyEffect::Allow,
                    Some(ActionKind::FileWrite),
                    None,
                ),
            ],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyDecision {
    pub effect: PolicyEffect,
    pub rule_id: Option<String>,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
// This vocabulary is part of the canonical cross-adapter contract. The
// production persistence path currently stores its stable string form.
#[allow(dead_code)]
pub enum ApprovalDecision {
    Approved,
    Denied,
    Expired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
// Keep the typed receipt vocabulary even while SQLite stores the serialized
// event names; protocol conformance tests protect these values.
#[allow(dead_code)]
pub enum ReceiptEvent {
    Requested,
    ApprovalPending,
    Denied,
    Released,
    ResultObserved,
    OutcomeUnknown,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PolicyError {
    Read(String),
    Parse(String),
    UnsupportedVersion(u32),
    InvalidRule(String),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(message) => write!(formatter, "cannot read policy: {message}"),
            Self::Parse(message) => write!(formatter, "cannot parse policy: {message}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported policy version {version}")
            }
            Self::InvalidRule(message) => write!(formatter, "invalid policy rule: {message}"),
        }
    }
}

impl std::error::Error for PolicyError {}

fn action_kind(tool: &str) -> ActionKind {
    match tool {
        "Read" | "Glob" | "Grep" | "read_file" | "view_image" => ActionKind::FileRead,
        "Edit" | "Write" | "NotebookEdit" | "apply_patch" => ActionKind::FileWrite,
        "Bash" | "PowerShell" | "exec_command" | "write_stdin" | "shell" => ActionKind::Shell,
        "WebFetch" | "WebSearch" | "web_search" => ActionKind::Network,
        "send_email" | "send_message" | "publish" => ActionKind::External,
        name if name.starts_with("mcp__") => ActionKind::Mcp,
        _ => ActionKind::Unknown,
    }
}

fn command(candidate: &ActionCandidate) -> Option<String> {
    candidate
        .arguments
        .get("command")
        .or_else(|| candidate.arguments.get("cmd"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| candidate.arguments.as_str().map(str::to_owned))
}

fn command_executable(command: &str) -> Option<String> {
    let token = command.split_whitespace().next()?.trim_matches(['\'', '"']);
    Path::new(token)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
}

fn canonical_resources(
    kind: ActionKind,
    candidate: &ActionCandidate,
    executable: Option<&str>,
) -> Vec<String> {
    match kind {
        ActionKind::FileRead | ActionKind::FileWrite => candidate
            .arguments
            .get("file_path")
            .or_else(|| candidate.arguments.get("path"))
            .or_else(|| candidate.arguments.get("notebook_path"))
            .and_then(Value::as_str)
            .map(normalize_path)
            .filter(|path| !path.trim().is_empty())
            .into_iter()
            .chain(patch_resources(candidate))
            .collect(),
        ActionKind::Shell => executable.map(str::to_owned).into_iter().collect(),
        ActionKind::Network => network_domain(candidate).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn patch_resources(candidate: &ActionCandidate) -> Vec<String> {
    if candidate.raw_tool_name != "apply_patch" {
        return Vec::new();
    }
    let Some(patch) = candidate.arguments.as_str() else {
        return Vec::new();
    };
    let mut paths = patch
        .lines()
        .filter_map(|line| {
            ["*** Add File: ", "*** Update File: ", "*** Delete File: "]
                .iter()
                .find_map(|prefix| line.strip_prefix(prefix))
        })
        .map(normalize_path)
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn network_domain(candidate: &ActionCandidate) -> Option<String> {
    let value = candidate
        .arguments
        .get("url")
        .or_else(|| candidate.arguments.get("domain"))?
        .as_str()?
        .trim();
    if value.is_empty() {
        return None;
    }
    let host = value.split_once("://").map_or(value, |(_, rest)| rest);
    let host = host.split(['/', ':']).next().unwrap_or(host).trim();
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

fn web_search_is_valid(candidate: &ActionCandidate) -> bool {
    if !matches!(candidate.raw_tool_name.as_str(), "WebSearch" | "web_search") {
        return false;
    }
    candidate
        .arguments
        .get("query")
        .and_then(Value::as_str)
        .is_some_and(|query| !query.trim().is_empty())
        || candidate
            .arguments
            .get("search_query")
            .and_then(Value::as_array)
            .is_some_and(|queries| !queries.is_empty())
}

fn risk_tags(
    candidate: &ActionCandidate,
    kind: ActionKind,
    command: Option<&str>,
    executable: Option<&str>,
    resources: &[String],
) -> Vec<String> {
    let mut tags = Vec::new();
    if arguments_are_malformed(candidate, kind, command, resources) {
        tags.push("malformed_arguments".to_owned());
    }
    if resources.iter().any(|resource| secret_path(resource)) {
        tags.push("secret_path".to_owned());
    }
    if resources.iter().any(|resource| path_escapes(resource)) {
        tags.push("host_escape".to_owned());
    }
    if resources
        .iter()
        .any(|resource| resource.to_ascii_lowercase().ends_with("docker.sock"))
    {
        tags.push("docker_access".to_owned());
    }
    if resources.len() > 1 {
        tags.push("multi_resource".to_owned());
    }
    if matches!(
        candidate.raw_tool_name.as_str(),
        "delete_file" | "remove_directory" | "format_disk"
    ) {
        tags.push("destructive".to_owned());
    }
    if kind == ActionKind::External {
        tags.push("external_side_effect".to_owned());
    }
    if let Some(command) = command {
        let lower = command.to_ascii_lowercase();
        if lower.contains("docker.sock") || executable == Some("docker") {
            tags.push("docker_access".to_owned());
        }
        if secret_command(&lower) {
            tags.push("secret_path".to_owned());
        }
        if network_command(executable, &lower) {
            tags.push("network_access".to_owned());
        }
        if lower.contains("&&")
            || lower.contains("||")
            || lower.contains(';')
            || lower.contains('|')
            || lower.contains('>')
            || lower.contains('<')
        {
            tags.push("compound_command".to_owned());
        }
        if destructive_command(executable, &lower) {
            tags.push("destructive".to_owned());
        }
        if lower.contains("../") || lower.contains("..\\") {
            tags.push("host_escape".to_owned());
        }
        if dependency_install(executable, &lower) {
            tags.push("dependency_install".to_owned());
        } else if offline_build_or_test(executable, &lower) {
            tags.push("offline_build_or_test".to_owned());
        }
    }
    tags
}

fn secret_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [
        "/.ssh/",
        "/.aws/",
        "/.gnupg/",
        "/.config/gcloud/",
        "/.env",
        "id_rsa",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || lower == ".env"
        || lower.starts_with(".env.")
        || lower.starts_with(".ssh/")
        || lower.starts_with(".aws/")
        || lower.starts_with(".gnupg/")
        || lower.starts_with(".config/gcloud/")
}

fn path_escapes(path: &str) -> bool {
    path.split('/').any(|component| component == "..")
}

fn arguments_are_malformed(
    candidate: &ActionCandidate,
    kind: ActionKind,
    command: Option<&str>,
    resources: &[String],
) -> bool {
    match kind {
        ActionKind::FileRead | ActionKind::FileWrite => resources.is_empty(),
        ActionKind::Shell => command.is_none_or(|value| value.trim().is_empty()),
        ActionKind::Network => {
            network_domain(candidate).is_none() && !web_search_is_valid(candidate)
        }
        ActionKind::External | ActionKind::Mcp | ActionKind::Unknown => false,
    }
}

fn dependency_install(executable: Option<&str>, command: &str) -> bool {
    match executable {
        Some("npm" | "pnpm" | "yarn" | "pip" | "pip3") => command
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| matches!(word, "add" | "ci" | "install" | "i")),
        Some("cargo") => command
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| matches!(word, "add" | "install")),
        Some("go") => command.split_whitespace().nth(1) == Some("get"),
        Some("dotnet") => command.split_whitespace().nth(1) == Some("restore"),
        _ => false,
    }
}

fn secret_command(command: &str) -> bool {
    [
        "/.ssh/",
        "~/.ssh/",
        "/.aws/",
        "~/.aws/",
        "/.gnupg/",
        "~/.gnupg/",
        ".env",
        "id_rsa",
        "id_ed25519",
    ]
    .iter()
    .any(|needle| command.contains(needle))
}

fn network_command(executable: Option<&str>, command: &str) -> bool {
    match executable {
        Some("curl" | "wget" | "ftp" | "nc" | "netcat") => true,
        Some("git") => command
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| matches!(word, "clone" | "fetch" | "pull" | "push")),
        Some("powershell" | "powershell.exe" | "pwsh" | "pwsh.exe") => [
            "invoke-webrequest",
            "invoke-restmethod",
            "start-bitstransfer",
        ]
        .iter()
        .any(|word| command.contains(word)),
        _ => false,
    }
}

fn destructive_command(executable: Option<&str>, command: &str) -> bool {
    match executable {
        Some("rm" | "rmdir" | "del" | "erase" | "shred" | "format") => true,
        Some("git") => command
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| matches!(word, "clean" | "reset")),
        Some("powershell" | "powershell.exe" | "pwsh" | "pwsh.exe") => [
            "remove-item",
            "clear-disk",
            "format-volume",
            "remove-partition",
        ]
        .iter()
        .any(|word| command.contains(word)),
        _ => false,
    }
}

fn offline_build_or_test(executable: Option<&str>, command: &str) -> bool {
    match executable {
        Some("cargo") => command
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| matches!(word, "build" | "check" | "test" | "fmt" | "clippy")),
        Some("npm" | "pnpm" | "yarn") => command
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| matches!(word, "run" | "test")),
        Some("rg" | "rustc") => true,
        Some("git") => command.split_whitespace().nth(1).is_some_and(|word| {
            matches!(
                word,
                "branch" | "diff" | "log" | "ls-files" | "rev-parse" | "show" | "status"
            )
        }),
        Some("go") => command
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| matches!(word, "build" | "fmt" | "test")),
        Some("dotnet") => command
            .split_whitespace()
            .nth(1)
            .is_some_and(|word| matches!(word, "build" | "test")),
        _ => false,
    }
}

fn arguments_digest(arguments: &Value) -> String {
    let bytes = serde_json::to_vec(arguments).expect("JSON value always serializes");
    format!("{:x}", Sha256::digest(bytes))
}

fn stable_digest<T: Serialize>(value: &T) -> String {
    let bytes = serde_json::to_vec(value).expect("canonical policy values always serialize");
    format!("{:x}", Sha256::digest(bytes))
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut table = vec![vec![false; value.len() + 1]; pattern.len() + 1];
    table[0][0] = true;
    for index in 1..=pattern.len() {
        if pattern[index - 1] == b'*' {
            table[index][0] = table[index - 1][0];
        }
    }
    for p in 1..=pattern.len() {
        for v in 1..=value.len() {
            table[p][v] = match pattern[p - 1] {
                b'*' => table[p - 1][v] || table[p][v - 1],
                b'?' => table[p - 1][v - 1],
                byte => byte == value[v - 1] && table[p - 1][v - 1],
            };
        }
    }
    table[pattern.len()][value.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(tool: &str, arguments: Value) -> ActionCandidate {
        ActionCandidate {
            protocol: Protocol::AnthropicMessages,
            id: "tool_test".to_owned(),
            raw_tool_name: tool.to_owned(),
            arguments,
        }
    }

    fn intent(tool: &str, arguments: Value) -> ActionIntent {
        ActionIntent::from_candidate(
            &candidate(tool, arguments),
            "session",
            AgentKind::ClaudeCode,
            "test",
        )
    }

    #[test]
    fn normalizes_claude_actions_without_retaining_raw_arguments() {
        let action = intent("Bash", serde_json::json!({"command": "cargo test"}));
        assert_eq!(action.kind, ActionKind::Shell);
        assert_eq!(action.executable.as_deref(), Some("cargo"));
        assert_eq!(action.canonical_resource.as_deref(), Some("cargo"));
        assert!(action
            .risk_tags
            .contains(&"offline_build_or_test".to_owned()));
        assert_eq!(action.arguments_digest.len(), 64);
        assert!(!action.redacted_display.contains("cargo test"));
    }

    #[test]
    fn ordered_rules_are_first_match() {
        let action = intent("Read", serde_json::json!({"file_path": "src/lib.rs"}));
        let policy = GuardPolicy {
            version: POLICY_VERSION,
            default: PolicyEffect::Ask,
            rules: vec![
                PolicyRule {
                    id: "first".to_owned(),
                    effect: PolicyEffect::Deny,
                    agent: None,
                    kind: Some(ActionKind::FileRead),
                    tool: None,
                    path: None,
                    executable: None,
                    domain: None,
                    risk_tag: None,
                },
                PolicyRule {
                    id: "second".to_owned(),
                    effect: PolicyEffect::Allow,
                    agent: None,
                    kind: Some(ActionKind::FileRead),
                    tool: None,
                    path: None,
                    executable: None,
                    domain: None,
                    risk_tag: None,
                },
            ],
        };
        let decision = policy.evaluate(&action);
        assert_eq!(decision.effect, PolicyEffect::Deny);
        assert_eq!(decision.rule_id.as_deref(), Some("first"));
    }

    #[test]
    fn balanced_policy_allows_repo_work_and_asks_for_installs() {
        let policy = GuardPolicy::balanced();
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Read",
                    serde_json::json!({"file_path": "src/lib.rs"})
                ))
                .effect,
            PolicyEffect::Allow
        );
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Bash",
                    serde_json::json!({"command": "cargo test"})
                ))
                .effect,
            PolicyEffect::Allow
        );
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Bash",
                    serde_json::json!({"command": "npm install left-pad"})
                ))
                .effect,
            PolicyEffect::Ask
        );
        assert_eq!(
            policy
                .evaluate(&intent("Mystery", serde_json::json!({})))
                .effect,
            PolicyEffect::Deny
        );
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Bash",
                    serde_json::json!({"command": "rm -rf target"})
                ))
                .effect,
            PolicyEffect::Ask
        );
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Bash",
                    serde_json::json!({"command": "cargo test && curl https://example.com"})
                ))
                .effect,
            PolicyEffect::Ask
        );
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Bash",
                    serde_json::json!({"command": "cat ~/.ssh/id_rsa"})
                ))
                .effect,
            PolicyEffect::Deny
        );
        assert_eq!(
            policy
                .evaluate(&intent(
                    "WebSearch",
                    serde_json::json!({"query": "Kerna docs"})
                ))
                .effect,
            PolicyEffect::Ask
        );
    }

    #[test]
    fn immutable_risks_override_an_allow_rule() {
        let action = intent(
            "Read",
            serde_json::json!({"file_path": "/home/user/.ssh/id_rsa"}),
        );
        let policy = GuardPolicy {
            version: POLICY_VERSION,
            default: PolicyEffect::Allow,
            rules: Vec::new(),
        };
        assert_eq!(policy.evaluate(&action).effect, PolicyEffect::Deny);
    }

    #[test]
    fn malformed_known_actions_fail_closed() {
        let policy = GuardPolicy::balanced();
        for action in [
            intent("Read", serde_json::json!({})),
            intent("Bash", serde_json::json!({"command": ""})),
            intent("WebFetch", serde_json::json!({"url": 42})),
            intent("apply_patch", Value::String("not a patch".to_owned())),
        ] {
            let decision = policy.evaluate(&action);
            assert_eq!(decision.effect, PolicyEffect::Deny);
            assert_eq!(
                decision.rule_id.as_deref(),
                Some("kerna.invariant.malformed_arguments")
            );
        }
    }

    #[test]
    fn empty_paths_and_urls_are_malformed() {
        let policy = GuardPolicy::balanced();
        for action in [
            intent("Read", serde_json::json!({"file_path": ""})),
            intent("Edit", serde_json::json!({"file_path": "   "})),
            intent("WebFetch", serde_json::json!({"url": ""})),
        ] {
            let decision = policy.evaluate(&action);
            assert_eq!(decision.effect, PolicyEffect::Deny);
            assert_eq!(
                decision.rule_id.as_deref(),
                Some("kerna.invariant.malformed_arguments")
            );
        }
    }

    #[test]
    fn patch_normalization_never_hides_additional_or_secret_paths() {
        let single = intent(
            "apply_patch",
            Value::String("*** Begin Patch\n*** Update File: src/lib.rs\n*** End Patch".to_owned()),
        );
        assert_eq!(single.canonical_resource.as_deref(), Some("src/lib.rs"));

        let multiple = intent(
            "apply_patch",
            Value::String(
                "*** Begin Patch\n*** Update File: src/lib.rs\n*** Add File: .env\n*** End Patch"
                    .to_owned(),
            ),
        );
        assert_eq!(multiple.canonical_resource, None);
        assert!(multiple.risk_tags.contains(&"multi_resource".to_owned()));
        assert!(multiple.risk_tags.contains(&"secret_path".to_owned()));
        assert_eq!(
            GuardPolicy::balanced().evaluate(&multiple).effect,
            PolicyEffect::Deny
        );
    }

    #[test]
    fn legacy_conversion_preserves_exact_over_wildcard() {
        let rules = vec![
            PermissionRule {
                tool: "*".to_owned(),
                action: "deny".to_owned(),
            },
            PermissionRule {
                tool: "Read".to_owned(),
                action: "auto_approve".to_owned(),
            },
        ];
        let policy = GuardPolicy::from_legacy_permissions(&rules, PolicyEffect::Deny);
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Read",
                    serde_json::json!({"file_path": "src/lib.rs"})
                ))
                .effect,
            PolicyEffect::Allow
        );
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Write",
                    serde_json::json!({"file_path": "src/lib.rs"})
                ))
                .effect,
            PolicyEffect::Deny
        );
    }

    #[test]
    fn parses_versioned_policy_and_rejects_unknown_versions() {
        let policy = GuardPolicy::parse_toml(
            r#"
version = 1
default = "ask"

[[rules]]
id = "source-read"
effect = "allow"
kind = "file_read"
path = "src/*"
"#,
        )
        .unwrap();
        assert_eq!(
            policy
                .evaluate(&intent(
                    "Read",
                    serde_json::json!({"file_path": "src/lib.rs"})
                ))
                .effect,
            PolicyEffect::Allow
        );
        assert_eq!(
            GuardPolicy::parse_toml("version = 2\ndefault = \"deny\""),
            Err(PolicyError::UnsupportedVersion(2))
        );
        assert!(matches!(
            GuardPolicy::parse_toml(
                "version = 1\ndefault = \"deny\"\n[[rules]]\nid = \"same\"\neffect = \"allow\"\ntool = \"Read\"\n[[rules]]\nid = \"same\"\neffect = \"deny\"\ntool = \"Write\""
            ),
            Err(PolicyError::InvalidRule(message)) if message.contains("duplicate rule id")
        ));
    }

    #[test]
    fn shared_legacy_conformance_cases_keep_their_meaning() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("benchmarks/policy/policy-conformance.json");
        let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let rules = case["rules"]
                .as_array()
                .unwrap()
                .iter()
                .map(|rule| PermissionRule {
                    tool: rule["tool"].as_str().unwrap().to_owned(),
                    action: rule["action"].as_str().unwrap().to_owned(),
                })
                .collect::<Vec<_>>();
            let default = PolicyEffect::from_legacy(case["default"].as_str().unwrap());
            let policy = GuardPolicy::from_legacy_permissions(&rules, default);
            for (tool, expected) in case["expect"].as_object().unwrap() {
                let arguments = match tool.as_str() {
                    "Read" => serde_json::json!({"file_path": "src/lib.rs"}),
                    "Edit" => serde_json::json!({"file_path": "src/lib.rs"}),
                    "Bash" => serde_json::json!({"command": "echo conformance"}),
                    _ => serde_json::json!({}),
                };
                let got = policy.evaluate(&intent(tool, arguments)).effect;
                assert_eq!(
                    got,
                    PolicyEffect::from_legacy(expected.as_str().unwrap()),
                    "{}: {tool}",
                    case["name"]
                );
            }
        }
    }

    #[test]
    fn protocol_adapters_share_canonical_policy_decisions() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("benchmarks/policy/guard-policy-conformance.json");
        let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(fixture["version"], 1);

        for case in fixture["cases"].as_array().unwrap() {
            let policy: GuardPolicy = serde_json::from_value(case["policy"].clone()).unwrap();
            policy.validate().unwrap();
            let expected = serde_json::from_value::<PolicyEffect>(case["expect"].clone()).unwrap();
            let mut decisions = Vec::new();
            for (agent_key, protocol, agent) in [
                ("claude", Protocol::AnthropicMessages, AgentKind::ClaudeCode),
                ("openai", Protocol::OpenAiResponses, AgentKind::Codex),
            ] {
                let adapted = &case[agent_key];
                let candidate = ActionCandidate {
                    protocol,
                    id: format!("{}-{agent_key}", case["name"].as_str().unwrap()),
                    raw_tool_name: adapted["tool"].as_str().unwrap().to_owned(),
                    arguments: adapted["arguments"].clone(),
                };
                let intent = ActionIntent::from_candidate(&candidate, "session", agent, "test");
                let decision = policy.evaluate(&intent).effect;
                assert_eq!(decision, expected, "{}: {agent_key}", case["name"]);
                decisions.push(decision);
            }
            assert_eq!(decisions[0], decisions[1], "{}", case["name"]);
        }
    }

    #[test]
    fn decision_and_receipt_vocabularies_are_stable() {
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::Approved).unwrap(),
            "\"approved\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::Denied).unwrap(),
            "\"denied\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalDecision::Expired).unwrap(),
            "\"expired\""
        );
        assert_eq!(
            serde_json::to_string(&[
                ReceiptEvent::Requested,
                ReceiptEvent::ApprovalPending,
                ReceiptEvent::Denied,
                ReceiptEvent::Released,
                ReceiptEvent::ResultObserved,
                ReceiptEvent::OutcomeUnknown,
            ])
            .unwrap(),
            "[\"requested\",\"approval_pending\",\"denied\",\"released\",\"result_observed\",\"outcome_unknown\"]"
        );
    }

    #[test]
    fn glob_matching_is_small_and_deterministic() {
        assert!(glob_matches("src/*.rs", "src/lib.rs"));
        assert!(!glob_matches("src/*.rs", "tests/lib.rs"));
        assert!(glob_matches("*.example.com", "packages.example.com"));
    }
}
