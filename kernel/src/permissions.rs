use crate::config::Config;
use anyhow::Result;
use std::io::{self, Write};

/// Risk level for tool actions.
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionLevel {
    AutoApprove,
    RequireConfirmation,
    Deny,
}

/// How much of the policy's verdict is actually applied.
///
/// Rung 1 of the rollout ladder (Decision 040). A layer that starts at full strength is a
/// migration, not an install: fail-closed with no observe mode means a fresh install
/// denies everything until tools are granted, so the customer's first experience of a
/// governance product is their agent breaking.
///
/// `Observe` records every decision and enforces none of them. It answers "what would
/// this have blocked?" without anything being blocked, which is the only honest way to
/// let someone size a policy before trusting it. The constraint 040 states is that **no
/// enforcement point may ship without its shadow mode** — a rule that can only be tested
/// by enforcing it is a rule nobody will turn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementMode {
    /// Record the decision; do not act on it.
    Observe,
    /// Apply the decision.
    Enforce,
}

impl EnforcementMode {
    pub fn as_str(self) -> &'static str {
        match self {
            EnforcementMode::Observe => "observe",
            EnforcementMode::Enforce => "enforce",
        }
    }
}

/// What the policy said, and what will actually happen.
///
/// Both, always, and deliberately not collapsed into one value. The entire product of
/// rung 1 is the gap between them: an audit trail that recorded the *enforced* outcome
/// would report a clean run in observe mode and tell the customer nothing, which is the
/// same failure as a cohort label that does not describe the cohort.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// The policy's verdict, unchanged by the mode. This is what gets recorded.
    pub policy: PermissionLevel,
    /// What the runtime will do about it.
    pub effective: PermissionLevel,
    /// True when the two differ because enforcement is off.
    pub observed_only: bool,
    pub mode: EnforcementMode,
}

impl Decision {
    /// A one-word summary for an audit row: what the policy decided.
    pub fn recorded(&self) -> String {
        format!("{:?}", self.policy)
    }

    /// Whether the policy would have stopped or paused this action, regardless of mode.
    pub fn policy_would_intervene(&self) -> bool {
        self.policy != PermissionLevel::AutoApprove
    }
}

/// Manages permission policies for tool calls.
pub struct PermissionManager {
    config: Config,
    mode: EnforcementMode,
}

impl PermissionManager {
    /// Enforcing, which stays the default. Observe is always something a person asked
    /// for: a governance layer that quietly stopped governing would be worse than one
    /// that was never installed, because the customer believes they are covered.
    pub fn new(config: Config) -> Self {
        PermissionManager {
            config,
            mode: EnforcementMode::Enforce,
        }
    }

    pub fn with_mode(config: Config, mode: EnforcementMode) -> Self {
        PermissionManager { config, mode }
    }

    pub fn mode(&self) -> EnforcementMode {
        self.mode
    }

    /// The policy's verdict together with what the runtime will do about it.
    ///
    /// In `Observe` the effective level is always `AutoApprove` — no denials and no
    /// approval prompts — because rung 1 promises the customer risks nothing, and a
    /// prompt is still an interruption. `policy` keeps the real verdict so the audit
    /// trail says what would have happened.
    pub fn decide(&self, tool_name: &str, server_name: Option<&str>) -> Decision {
        let policy = self.check(tool_name, server_name);

        let effective = match self.mode {
            EnforcementMode::Enforce => policy.clone(),
            EnforcementMode::Observe => PermissionLevel::AutoApprove,
        };

        Decision {
            observed_only: policy != effective,
            policy,
            effective,
            mode: self.mode,
        }
    }

    /// Determine the permission level for a given tool.
    pub fn check(&self, tool_name: &str, server_name: Option<&str>) -> PermissionLevel {
        let action = self.config.check_permission(tool_name);

        let mut level = match action {
            "require_confirmation" => PermissionLevel::RequireConfirmation,
            "deny" => PermissionLevel::Deny,
            "auto_approve" => {
                // Apply built-in safety defaults for dangerous operations even if auto-approved
                match tool_name {
                    "delete_file" | "remove_directory" | "format_disk" => {
                        PermissionLevel::RequireConfirmation
                    }
                    "desktop_click" | "desktop_type" | "send_email" => {
                        PermissionLevel::RequireConfirmation
                    }
                    _ => PermissionLevel::AutoApprove,
                }
            }
            _ => PermissionLevel::Deny, // Fail-closed on typos
        };

        if level == PermissionLevel::AutoApprove {
            if let Some(s_name) = server_name {
                if let Some(s_cfg) = self.config.mcp_servers.iter().find(|s| s.name == s_name) {
                    if s_cfg.approval_required.contains(&tool_name.to_string())
                        || s_cfg.approval_required.contains(&"*".to_string())
                    {
                        level = PermissionLevel::RequireConfirmation;
                    }
                }
            }
        }

        level
    }

    /// Prompt the user for confirmation in the terminal, with a readable preview
    /// of exactly what the tool will do. For side-effectful tools (send/post/
    /// create/reply), the actual content (recipient, subject, body) is shown in
    /// full so a non-technical user can eyeball it before approving.
    pub fn prompt_approval(tool_name: &str, args_display: &str) -> Result<bool> {
        println!();
        println!("  ⚠️  APPROVAL REQUIRED");
        println!("  ────────────────────────────────────────────────────────");
        println!("  Tool: {}", tool_name);

        let lower = tool_name.to_lowercase();
        let is_side_effect = [
            "send", "post", "reply", "create", "publish", "email", "message",
        ]
        .iter()
        .any(|k| lower.contains(k));
        if is_side_effect {
            println!("  \x1b[33mThis will take an external action on your behalf.\x1b[0m");
        }

        // Pretty-print the arguments so the human sees the real content.
        match serde_json::from_str::<serde_json::Value>(args_display) {
            Ok(val) => {
                if let Some(obj) = val.as_object() {
                    println!("  Details:");
                    for (k, v) in obj {
                        let text = match v {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        // Show the full value for the fields that matter most on a
                        // side-effectful action; cap the rest so the prompt stays readable.
                        let important = [
                            "to",
                            "recipient",
                            "subject",
                            "body",
                            "text",
                            "message",
                            "content",
                        ]
                        .contains(&k.as_str());
                        if important || text.chars().count() <= 200 {
                            println!("    {}: {}", k, text);
                        } else {
                            let short: String = text.chars().take(200).collect();
                            println!("    {}: {}… [{} chars]", k, short, text.chars().count());
                        }
                    }
                } else {
                    println!("  Args: {}", val);
                }
            }
            Err(_) => {
                let short: String = args_display.chars().take(400).collect();
                println!("  Args: {}", short);
            }
        }
        println!("  ────────────────────────────────────────────────────────");
        print!("  Allow this action? [y/N]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        Ok(input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PermissionRule;

    #[test]
    fn test_sabotage_rogue_plugin_denied() {
        let config = Config {
            permissions: vec![PermissionRule {
                tool: "fs.read".to_string(),
                action: "auto_approve".to_string(),
            }],
            ..Default::default()
        };

        let pm = PermissionManager::new(config);

        assert_eq!(pm.check("fs.read", None), PermissionLevel::AutoApprove);
        // Escalation Sabotage: Tool tries to use a different capability
        assert_eq!(pm.check("fs.write", None), PermissionLevel::Deny);
        assert_eq!(pm.check("run_command", None), PermissionLevel::Deny);
    }

    // ------------------------------------------------ rung 1: observe mode
    //
    // Decision 040: a layer that starts at full strength is a migration, not an install.
    // Fail-closed with no observe mode means a fresh install denies everything until
    // tools are granted -- an onboarding wall rather than a first day.
    //
    // The whole product of this rung is the GAP between what policy decided and what was
    // enforced. Every test below is about keeping that gap visible.

    fn strict_config() -> Config {
        Config {
            permissions: vec![PermissionRule {
                tool: "fs.read".to_string(),
                action: "auto_approve".to_string(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn enforcing_is_the_default() {
        // Observe must always be something a person asked for. A governance layer that
        // quietly stopped governing is worse than one never installed, because the
        // customer believes they are covered.
        let pm = PermissionManager::new(strict_config());
        assert_eq!(pm.mode(), EnforcementMode::Enforce);
        assert_eq!(pm.decide("fs.write", None).effective, PermissionLevel::Deny);
    }

    #[test]
    fn observe_mode_enforces_nothing() {
        let pm = PermissionManager::with_mode(strict_config(), EnforcementMode::Observe);

        for tool in ["fs.write", "run_command", "delete_file"] {
            let d = pm.decide(tool, None);
            assert_eq!(d.effective, PermissionLevel::AutoApprove, "{}", tool);
        }
    }

    #[test]
    fn observe_mode_still_records_what_the_policy_decided() {
        // The reason the two fields are not collapsed into one. An audit trail that
        // recorded the ENFORCED outcome would report a clean run in observe mode and
        // tell the customer nothing -- which is the finding they installed this to get.
        let pm = PermissionManager::with_mode(strict_config(), EnforcementMode::Observe);

        let d = pm.decide("fs.write", None);
        assert_eq!(d.policy, PermissionLevel::Deny);
        assert_eq!(d.recorded(), "Deny");
        assert!(d.observed_only);
        assert!(d.policy_would_intervene());
    }

    #[test]
    fn approval_prompts_are_suppressed_too() {
        // Rung 1 promises the customer risks nothing, and a prompt is still an
        // interruption -- a wall made of questions instead of denials.
        let config = Config {
            permissions: vec![PermissionRule {
                tool: "delete_file".to_string(),
                action: "auto_approve".to_string(),
            }],
            ..Default::default()
        };
        // The built-in safety default makes this RequireConfirmation even when granted.
        assert_eq!(
            PermissionManager::new(config.clone()).check("delete_file", None),
            PermissionLevel::RequireConfirmation
        );

        let pm = PermissionManager::with_mode(config, EnforcementMode::Observe);
        let d = pm.decide("delete_file", None);
        assert_eq!(d.effective, PermissionLevel::AutoApprove);
        assert_eq!(d.policy, PermissionLevel::RequireConfirmation);
        assert!(d.observed_only);
    }

    #[test]
    fn an_allowed_action_is_not_marked_as_observed() {
        // observed_only must mean "policy was overridden", not "observe mode is on".
        // If it meant the latter, every row would be flagged and the flag would stop
        // being read -- the report needs the ones that would have been stopped.
        let pm = PermissionManager::with_mode(strict_config(), EnforcementMode::Observe);

        let d = pm.decide("fs.read", None);
        assert_eq!(d.policy, PermissionLevel::AutoApprove);
        assert!(!d.observed_only);
        assert!(!d.policy_would_intervene());
    }

    #[test]
    fn enforcing_mode_never_reports_an_override() {
        let pm = PermissionManager::with_mode(strict_config(), EnforcementMode::Enforce);

        for tool in ["fs.read", "fs.write", "run_command"] {
            let d = pm.decide(tool, None);
            assert_eq!(d.policy, d.effective, "{}", tool);
            assert!(!d.observed_only, "{}", tool);
        }
    }

    #[test]
    fn the_mode_is_recorded_on_every_decision() {
        // It lands in the audit row. A trail that does not say which mode produced it
        // cannot be read six months later, and mixing modes in one report is exactly
        // the pooling error cohort labels exist to prevent.
        let observe = PermissionManager::with_mode(strict_config(), EnforcementMode::Observe);
        let enforce = PermissionManager::with_mode(strict_config(), EnforcementMode::Enforce);

        assert_eq!(observe.decide("fs.write", None).mode.as_str(), "observe");
        assert_eq!(enforce.decide("fs.write", None).mode.as_str(), "enforce");
    }
}
