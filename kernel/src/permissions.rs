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

/// Manages permission policies for tool calls.
pub struct PermissionManager {
    config: Config,
}

impl PermissionManager {
    pub fn new(config: Config) -> Self {
        PermissionManager { config }
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
}
