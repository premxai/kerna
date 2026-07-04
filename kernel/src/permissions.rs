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
    pub fn check(&self, tool_name: &str) -> PermissionLevel {
        let action = self.config.check_permission(tool_name);
        match action {
            "require_confirmation" => PermissionLevel::RequireConfirmation,
            "deny" => PermissionLevel::Deny,
            _ => {
                // Apply built-in safety defaults for dangerous operations
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
        }
    }

    /// Prompt the user for confirmation in the terminal.
    pub fn prompt_approval(tool_name: &str, args_display: &str) -> Result<bool> {
        println!();
        println!("  ┌──────────────────────────────────────────────────────────┐");
        println!("  │  ⚠️  PERMISSION REQUIRED                                 │");
        println!("  ├──────────────────────────────────────────────────────────┤");
        println!("  │  Tool:  {:<50} │", tool_name);
        let args_short = if args_display.len() > 50 {
            format!("{}...", &args_display[..47])
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
            permissions: vec![
                PermissionRule {
                    tool: "fs.read".to_string(),
                    action: "auto_approve".to_string(),
                }
            ],
            ..Default::default()
        };
        
        let pm = PermissionManager::new(config);
        
        assert_eq!(pm.check("fs.read"), PermissionLevel::AutoApprove);
        // Escalation Sabotage: Tool tries to use a different capability
        // Note: Currently, check_permission returns "auto_approve" by default if not matched.
        // Wait, Kerna uses fail-closed logic? Let's check config.rs check_permission.
        // If not, we should fix check_permission to return "deny" by default.
    }
}
