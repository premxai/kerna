use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("kernel must be inside the repository")
        .to_path_buf()
}

fn inventory() -> Value {
    let path = repo_root().join("contracts/security-action-inventory.json");
    serde_json::from_str(&fs::read_to_string(path).expect("inventory must be readable"))
        .expect("inventory must be valid JSON")
}

fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    for item in fs::read_dir(root).expect("source root must be readable") {
        let path = item.expect("source entry must be readable").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn inventory_contract_is_complete_and_uses_stable_ids() {
    let inventory = inventory();
    assert_eq!(inventory["version"], 1);
    assert_eq!(
        inventory["statuses"],
        serde_json::json!([
            "governed",
            "contained",
            "trusted-host-only",
            "intentionally-excluded",
            "open-bypass"
        ])
    );

    let allowed_statuses: BTreeSet<&str> = inventory["statuses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    let allowed_risks = BTreeSet::from(["P0", "P1", "P2", "accepted"]);
    let required = [
        "title",
        "entrypoint",
        "initiator",
        "controlled_inputs",
        "operation",
        "privileges",
        "enforcement_point",
        "containment_boundary",
        "receipt_coverage",
        "failure_behavior",
        "platforms",
        "tests",
        "source_files",
        "status",
        "risk",
    ];

    let mut ids = BTreeSet::new();
    for entry in inventory["entries"].as_array().unwrap() {
        let id = entry["id"].as_str().expect("entry needs an id");
        assert!(
            id.starts_with("SB-")
                && id.len() == 6
                && id[3..].chars().all(|character| character.is_ascii_digit()),
            "{id} is not a stable SB-NNN identifier"
        );
        assert!(ids.insert(id), "duplicate inventory id {id}");
        for field in required {
            assert!(!entry[field].is_null(), "{id} is missing {field}");
        }
        assert!(
            allowed_statuses.contains(entry["status"].as_str().unwrap()),
            "{id} has an unknown status"
        );
        assert!(
            allowed_risks.contains(entry["risk"].as_str().unwrap()),
            "{id} has an unknown risk"
        );
        assert!(
            !entry["tests"].as_array().unwrap().is_empty(),
            "{id} must cite verification"
        );
        if entry["status"] == "open-bypass" {
            assert!(
                entry["remediation_task"].as_u64().is_some(),
                "{id} open bypass needs a remediation task"
            );
        }
    }
}

#[test]
fn every_sensitive_rust_surface_is_classified() {
    let root = repo_root();
    let inventory = inventory();
    let patterns: Vec<&str> = inventory["sensitive_patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();

    let mut classified = BTreeMap::<String, Vec<String>>::new();
    for entry in inventory["entries"].as_array().unwrap() {
        let id = entry["id"].as_str().unwrap().to_owned();
        for source in entry["source_files"].as_array().unwrap() {
            let source = source.as_str().unwrap().replace('\\', "/");
            assert!(root.join(&source).is_file(), "{id} cites missing {source}");
            classified.entry(source).or_default().push(id.clone());
        }
    }

    let mut files = Vec::new();
    rust_files(&root.join("kernel/src"), &mut files);
    rust_files(&root.join("ui/src-tauri/src"), &mut files);

    let mut missing = Vec::new();
    for path in files {
        let contents = fs::read_to_string(&path).expect("Rust source must be readable");
        if patterns.iter().any(|pattern| contents.contains(pattern)) {
            let relative = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if !classified.contains_key(&relative) {
                missing.push(relative);
            }
        }
    }
    assert!(
        missing.is_empty(),
        "sensitive Rust surfaces need inventory entries: {}",
        missing.join(", ")
    );
}

#[test]
fn model_controlled_surfaces_fail_closed_or_are_explicit_bypasses() {
    let inventory = inventory();
    for entry in inventory["entries"].as_array().unwrap() {
        if matches!(entry["initiator"].as_str(), Some("model" | "agent")) {
            let id = entry["id"].as_str().unwrap();
            let status = entry["status"].as_str().unwrap();
            assert!(
                matches!(
                    status,
                    "governed" | "contained" | "intentionally-excluded" | "open-bypass"
                ),
                "{id} model-controlled path has ambiguous status {status}"
            );
            let failure = entry["failure_behavior"].as_str().unwrap();
            assert!(!failure.trim().is_empty(), "{id} needs a failure behavior");
            if status == "open-bypass" {
                assert_eq!(entry["risk"], "P0", "model/agent bypass {id} must be P0");
            }
        }
    }
}
