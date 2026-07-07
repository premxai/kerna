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

    /// Prompt the user for confirmation in the terminal.
    pub fn prompt_approval(tool_name: &str, args_display: &str) -> Result<bool> {
        println!();
        println!("  ┌──────────────────────────────────────────────────────────┐");
        println!("  │  ⚠️  PERMISSION REQUIRED                                 │");
        println!("  ├──────────────────────────────────────────────────────────┤");
        println!("  │  Tool:  {:<50} │", tool_name);
        let args_short = if args_display.chars().count() > 50 {
            let truncated: String = args_display.chars().take(47).collect();
            format!("{}...", truncated)
        } else {
            args_display.to_string()
        };
        println!("  │  Args:  {:<50} │", args_short);
        println!("  └──────────────────────────────────────────────────────────┘");
        print!("  Allow this action? [y/n]: ");
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
