//! Structured activity log. Every native event is appended as one JSON line;
//! `kerna logs` is the human-facing reader. Normal CLI output stays clean
//! because the machinery goes here instead of to the terminal.

use anyhow::Result;
use std::io::Write;
use std::path::PathBuf;

pub fn log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KERNA_LOG_DIR") {
        return PathBuf::from(dir);
    }
    if cfg!(windows) {
        PathBuf::from(r"C:\KernaData\logs")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
            .unwrap_or_else(std::env::temp_dir)
            .join("kerna")
            .join("logs")
    }
}

pub fn log_path() -> PathBuf {
    log_dir().join("kerna.jsonl")
}

/// Best effort: a broken log must never fail a governed run.
pub fn append_value(value: &serde_json::Value) {
    let path = log_path();
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(line) = serde_json::to_string(value) else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{line}");
    }
}

pub fn append_event(event: &serde_json::Value) {
    let mut entry = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    });
    if let (Some(entry_map), Some(event_map)) = (entry.as_object_mut(), event.as_object()) {
        for (key, value) in event_map {
            entry_map.insert(key.clone(), value.clone());
        }
    }
    append_value(&entry);
}

fn scalar_fields(value: &serde_json::Value) -> Vec<String> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter(|(key, _)| **key != "ts" && **key != "type")
                .filter_map(|(key, value)| {
                    let text = match value {
                        serde_json::Value::String(text) => text.clone(),
                        serde_json::Value::Number(number) => number.to_string(),
                        serde_json::Value::Bool(flag) => flag.to_string(),
                        _ => return None,
                    };
                    let text = if text.chars().count() > 60 {
                        let mut cut: String = text.chars().take(57).collect();
                        cut.push_str("...");
                        cut
                    } else {
                        text
                    };
                    Some(format!("{key}={text}"))
                })
                .take(6)
                .collect()
        })
        .unwrap_or_default()
}

/// One compact log line: local time, event name, and its scalar fields.
pub fn format_entry(value: &serde_json::Value) -> String {
    let ts = value.get("ts").and_then(|v| v.as_str()).unwrap_or("");
    let local = chrono::DateTime::parse_from_rfc3339(ts)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_else(|_| ts.to_string());
    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("?");
    let fields = scalar_fields(value);
    if fields.is_empty() {
        format!("{local} {kind}")
    } else {
        format!("{local} {kind}  {}", fields.join(" "))
    }
}

pub fn read_recent(lines: usize) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(log_path()).unwrap_or_default();
    Ok(text
        .lines()
        .rev()
        .take(lines)
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .map(|value| format_entry(&value))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect())
}

pub fn run(tail: usize, follow: bool) -> Result<()> {
    for line in read_recent(tail)? {
        println!("{line}");
    }
    if !follow {
        return Ok(());
    }
    let path = log_path();
    let mut size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() <= size {
            continue;
        }
        if let Ok(mut file) = std::fs::File::open(&path) {
            use std::io::{Read, Seek, SeekFrom};
            let _ = file.seek(SeekFrom::Start(size));
            let mut chunk = String::new();
            if file.read_to_string(&mut chunk).is_ok() {
                for line in chunk.lines() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                        println!("{}", format_entry(&value));
                    }
                }
            }
        }
        size = meta.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_round_trip_with_field_formatting() {
        let dir = std::env::temp_dir().join(format!("kerna-log-test-{}", uuid::Uuid::new_v4()));
        std::env::set_var("KERNA_LOG_DIR", &dir);
        append_event(&serde_json::json!({
            "type": "action.executed",
            "session_id": "code-1",
            "status": "written",
            "detail": "12 bytes staged",
            "preflight": {"nested": true}
        }));
        let lines = read_recent(10).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("action.executed"));
        assert!(lines[0].contains("status=written"));
        assert!(lines[0].contains("session_id=code-1"));
        // Nested payloads are not printed as scalars.
        assert!(!lines[0].contains("nested"));
        std::env::remove_var("KERNA_LOG_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_entry_truncates_long_values_and_survives_bad_timestamps() {
        let line = format_entry(&serde_json::json!({
            "ts": "not-a-time",
            "type": "assistant.delta",
            "text": "x".repeat(200),
        }));
        assert!(line.starts_with("not-a-time assistant.delta"));
        assert!(line.contains("..."));
        assert!(line.chars().count() < 120);
    }
}
