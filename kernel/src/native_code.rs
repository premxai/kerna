use crate::scheduler::ChatMessage;
use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

const TRACKED_FILE_LIMIT: usize = 160;
const STATUS_LINE_LIMIT: usize = 80;

#[derive(Debug, Clone)]
pub struct CodeDryRunContext {
    pub repo_root: PathBuf,
    pub head: String,
    pub status_digest: String,
    pub tracked_file_count: usize,
    pub status_line_count: usize,
    pub prompt: String,
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
         4. what approval/receipt/containment would be required before execution.\n\n\
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
}
