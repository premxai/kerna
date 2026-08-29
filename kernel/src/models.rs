//! Curated local-model metadata. The catalog is a pinned, read-only subset of
//! 0xSero/local-ai-registry; Kerna recommends and verifies only, never pulls
//! artifacts or launches a runtime.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

const CATALOG: &str = include_str!("../registry/local-models-v1.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    pub source: CatalogSource,
    pub recipes: Vec<ModelRecipe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSource {
    pub repository: String,
    pub revision: String,
    pub license: String,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRecipe {
    pub id: String,
    pub model_instance_id: String,
    pub hardware_id: String,
    pub hardware_count: u32,
    pub min_vram_gb: u64,
    pub engine: String,
    pub launch_kind: String,
    pub status: String,
    pub chat: bool,
    pub reasoning: bool,
    pub tools: bool,
    pub has_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardwareProfile {
    pub kind: String,
    pub name: String,
    pub memory_gb: Option<u64>,
    pub device_count: u32,
    pub detected: bool,
}

pub fn catalog() -> Result<Catalog> {
    let catalog: Catalog = serde_json::from_str(CATALOG)?;
    if catalog.schema_version != 1 || catalog.source.license != "MIT" {
        return Err(anyhow!("Invalid curated local-model catalog metadata."));
    }
    Ok(catalog)
}

/// Detect only the MVP hardware matrix: NVIDIA VRAM on Windows/Linux and
/// Apple unified memory on macOS. Unknown hardware intentionally receives no
/// validated recommendation until the operator supplies a profile.
pub fn detect_hardware() -> HardwareProfile {
    #[cfg(target_os = "macos")]
    {
        let memory = Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(|bytes| bytes / 1024 / 1024 / 1024);
        return HardwareProfile {
            kind: "apple".to_string(),
            name: "Apple Silicon".to_string(),
            memory_gb: memory,
            device_count: 1,
            detected: memory.is_some(),
        };
    }
    #[cfg(not(target_os = "macos"))]
    {
        let output = Command::new("nvidia-smi")
            .args([
                "--query-gpu=name,memory.total",
                "--format=csv,noheader,nounits",
            ])
            .output();
        if let Some(output) = output.ok().filter(|output| output.status.success()) {
            let rows = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let (name, memory) = line.split_once(',')?;
                    Some((name.trim().to_string(), memory.trim().parse::<u64>().ok()?))
                })
                .collect::<Vec<_>>();
            if let Some((first_name, _)) = rows.first() {
                let memory_mib = rows.iter().map(|(_, memory)| *memory).min().unwrap_or(0);
                let same_model = rows.iter().all(|(name, _)| name == first_name);
                return HardwareProfile {
                    kind: "nvidia".to_string(),
                    name: if same_model {
                        first_name.clone()
                    } else {
                        "Mixed NVIDIA GPUs".to_string()
                    },
                    memory_gb: Some(memory_mib / 1024),
                    device_count: rows.len() as u32,
                    detected: true,
                };
            }
        }
        HardwareProfile {
            kind: "unknown".to_string(),
            name: "Unknown hardware".to_string(),
            memory_gb: None,
            device_count: 0,
            detected: false,
        }
    }
}

/// Load an operator-supplied hardware profile for an unsupported device. This
/// enables transparent inspection, but does not turn an unvalidated device
/// into a validated recommendation.
pub fn load_manual_profile(path: &Path) -> Result<HardwareProfile> {
    let profile: HardwareProfile = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    if profile.kind.trim().is_empty() || profile.name.trim().is_empty() || profile.device_count == 0
    {
        return Err(anyhow!(
            "Manual model profile needs kind, name, and a positive device_count."
        ));
    }
    Ok(HardwareProfile {
        detected: true,
        ..profile
    })
}

fn normalized_hardware_name(value: &str) -> String {
    value
        .to_lowercase()
        .replace("nvidia", "")
        .replace("geforce", "")
        .replace(" ", "-")
        .replace("_", "-")
}

fn hardware_model_id(hardware_id: &str) -> &str {
    hardware_id
        .rsplit_once('-')
        .filter(|(_, suffix)| suffix.ends_with("gb"))
        .map(|(model, _)| model)
        .unwrap_or(hardware_id)
}

pub fn recommend(profile: &HardwareProfile, purpose: &str) -> Result<Vec<ModelRecipe>> {
    let catalog = catalog()?;
    if !profile.detected || profile.kind != "nvidia" {
        return Ok(vec![]);
    }
    let matches = catalog
        .recipes
        .into_iter()
        .filter(|recipe| recipe.status == "validated" && recipe.has_evidence)
        .filter(|recipe| purpose != "coding" || recipe.tools)
        .filter(|recipe| recipe.hardware_id.starts_with("rtx-"))
        .filter(|recipe| {
            normalized_hardware_name(&profile.name).contains(hardware_model_id(&recipe.hardware_id))
        })
        .filter(|recipe| recipe.hardware_count <= profile.device_count)
        .filter(|recipe| {
            profile
                .memory_gb
                .is_some_and(|vram| vram >= recipe.min_vram_gb)
        })
        .collect();
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_pinned_and_only_recommends_validated_tool_recipes() {
        let catalog = catalog().unwrap();
        assert_eq!(catalog.source.license, "MIT");
        assert_eq!(catalog.source.revision.len(), 40);
        let profile = HardwareProfile {
            kind: "nvidia".to_string(),
            name: "Synthetic RTX 3090".to_string(),
            memory_gb: Some(24),
            device_count: 4,
            detected: true,
        };
        let recipes = recommend(&profile, "coding").unwrap();
        assert!(recipes
            .iter()
            .all(|recipe| recipe.status == "validated" && recipe.tools));
    }

    #[test]
    fn unknown_or_apple_profiles_do_not_claim_validated_nvidia_recipes() {
        let unknown = HardwareProfile {
            kind: "unknown".to_string(),
            name: "Unknown".to_string(),
            memory_gb: None,
            device_count: 0,
            detected: false,
        };
        assert!(recommend(&unknown, "coding").unwrap().is_empty());

        let insufficient_vram = HardwareProfile {
            kind: "nvidia".to_string(),
            name: "Synthetic low-memory GPU".to_string(),
            memory_gb: Some(8),
            device_count: 4,
            detected: true,
        };
        assert!(recommend(&insufficient_vram, "coding").unwrap().is_empty());

        let wrong_gpu = HardwareProfile {
            kind: "nvidia".to_string(),
            name: "NVIDIA GeForce RTX 4090".to_string(),
            memory_gb: Some(24),
            device_count: 4,
            detected: true,
        };
        assert!(recommend(&wrong_gpu, "coding").unwrap().is_empty());
    }
}
