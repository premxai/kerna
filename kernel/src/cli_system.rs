//! The minimal machine check for `kerna init`: five human-legible rows and a
//! verdict. Deep diagnostics stay in `kerna doctor`.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SystemReport {
    pub os: String,
    pub cpu: String,
    pub ram_gb: u64,
    pub gpu: String,
    pub disk_free_gb: u64,
}

impl SystemReport {
    pub fn detect() -> Self {
        let mut system = sysinfo::System::new_all();
        system.refresh_all();
        let os = sysinfo::System::long_os_version()
            .or_else(|| Some(std::env::consts::OS.to_string()))
            .unwrap_or_else(|| "Unknown OS".to_string());
        let cpu = system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().trim().to_string())
            .filter(|brand| !brand.is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string());
        let ram_gb = system.total_memory() / 1_073_741_824;
        let hardware = crate::models::detect_hardware();
        let gpu = if hardware.detected {
            match hardware.memory_gb {
                Some(vram) => format!("{} · {vram} GB", hardware.name),
                None => hardware.name.clone(),
            }
        } else {
            "Not detected (CPU inference)".to_string()
        };
        let disk_free_gb = sysinfo::Disks::new_with_refreshed_list()
            .iter()
            .max_by_key(|disk| disk.total_space())
            .map(|disk| disk.available_space() / 1_073_741_824)
            .unwrap_or(0);
        Self {
            os,
            cpu,
            ram_gb,
            gpu,
            disk_free_gb,
        }
    }

    pub fn local_ai_supported(&self) -> bool {
        self.gpu != "Not detected (CPU inference)" || self.ram_gb >= 16
    }

    /// (label, value, ok) rows in the plan's fixed display order.
    pub fn rows(&self) -> Vec<(&'static str, String, bool)> {
        vec![
            ("OS", self.os.clone(), true),
            ("CPU", self.cpu.clone(), self.cpu != "Unknown CPU"),
            ("Memory", format!("{} GB", self.ram_gb), self.ram_gb > 0),
            (
                "GPU",
                self.gpu.clone(),
                self.gpu != "Not detected (CPU inference)",
            ),
            (
                "Storage",
                format!("{} GB available", self.disk_free_gb),
                self.disk_free_gb > 10,
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(gpu: &str, ram: u64) -> SystemReport {
        SystemReport {
            os: "Windows 11".to_string(),
            cpu: "Ryzen 7".to_string(),
            ram_gb: ram,
            gpu: gpu.to_string(),
            disk_free_gb: 186,
        }
    }

    #[test]
    fn rows_are_the_five_plan_fields_in_order() {
        let rows = report("RTX 4060 · 8 GB", 32).rows();
        let labels: Vec<_> = rows.iter().map(|(label, _, _)| *label).collect();
        assert_eq!(labels, vec!["OS", "CPU", "Memory", "GPU", "Storage"]);
        assert!(rows.iter().all(|(_, _, ok)| *ok));
    }

    #[test]
    fn local_ai_support_is_honest_about_missing_accelerators() {
        assert!(report("RTX 4060 · 8 GB", 16).local_ai_supported());
        assert!(!report("Not detected (CPU inference)", 8).local_ai_supported());
        // Large-RAM CPU-only machines can still run small local models.
        assert!(report("Not detected (CPU inference)", 32).local_ai_supported());
    }

    #[test]
    fn detection_never_panics_on_an_arbitrary_machine() {
        let report = SystemReport::detect();
        assert!(!report.os.is_empty());
        assert!(!report.cpu.is_empty());
    }
}
