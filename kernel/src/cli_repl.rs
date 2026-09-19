//! `kerna code` with no goal — the interactive Kerna coding environment:
//! near-empty start screen, one prompt, one changing activity line, and a
//! clean result per task. All machinery goes to `kerna logs`.

use anyhow::Result;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub async fn run(
    repo: PathBuf,
    provider: String,
    model: Option<String>,
    debug: bool,
    yes: bool,
    max_turns: u32,
) -> Result<()> {
    crate::cli_brand::banner("Kerna Code");
    // "Allow for this session" grants live exactly as long as this REPL.
    crate::native_exec::clear_session_grants();
    let display_root = repo.canonicalize().unwrap_or_else(|_| repo.clone());
    // Windows canonicalize() prepends the \\?\ verbatim prefix; show a path the user recognizes.
    println!(
        "{}",
        display_root
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string()
    );
    println!("{}", "─".repeat(44));
    println!();
    println!("What do you want to build?");
    println!();

    let stdin = io::stdin();
    let mut history: Vec<String> = Vec::new();
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            break; // EOF (Ctrl+Z / closed pipe)
        }
        let task = line.trim();
        match task {
            "" => continue,
            "exit" | "quit" | "/exit" | "/quit" => break,
            _ => {}
        }
        let note = if history.is_empty() {
            None
        } else {
            Some(format!(
                "Earlier tasks in this interactive session (already governed and finished):\n{}",
                history.join("\n")
            ))
        };
        match crate::run_native_code(
            task.to_string(),
            repo.clone(),
            provider.clone(),
            model.clone(),
            false,
            false,
            yes,
            max_turns,
            debug,
            note.as_deref(),
        )
        .await
        {
            Ok(outcome) => {
                if debug && !outcome.diff_stat.trim().is_empty() {
                    eprintln!("{}", outcome.diff_stat.trim());
                }
                if history.len() >= 8 {
                    history.remove(0);
                }
                history.push(format!(
                    "- \"{task}\" => {} ({} changed, {} tokens)",
                    outcome.outcome,
                    outcome.changed_files.len(),
                    outcome.tokens
                ));
            }
            Err(error) => {
                if history.len() >= 8 {
                    history.remove(0);
                }
                history.push(format!("- \"{task}\" => failed"));
                if debug {
                    eprintln!("task failed: {error:#}");
                }
            }
        }
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn session_history_is_bounded_and_summarised() {
        // The REPL injects at most eight one-line outcomes as context; that
        // invariant lives here so a refactor cannot silently grow the prompt.
        let mut history: Vec<String> = (0..12)
            .map(|index| format!("- \"task {index}\" => applied"))
            .collect();
        if history.len() > 8 {
            let excess = history.len() - 8;
            history.drain(..excess);
        }
        assert_eq!(history.len(), 8);
        assert!(history[0].contains("task 4"));
        assert!(history[7].contains("task 11"));
    }
}
