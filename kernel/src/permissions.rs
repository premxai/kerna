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
