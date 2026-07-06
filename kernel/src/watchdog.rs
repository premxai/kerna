use crate::config::Config;
use crate::memory::MemoryEngine;
use anyhow::Result;
use reqwest::Client;
use sha2::{Sha256, Digest};
use std::sync::Arc;
use std::time::Duration;

pub struct WatchdogEngine {
    memory: Arc<MemoryEngine>,
    _config: Config,
}

impl WatchdogEngine {
    pub fn new(memory: Arc<MemoryEngine>, config: Config) -> Self {
        WatchdogEngine { memory, _config: config }
    }

    pub async fn start(&self) -> Result<()> {
        let memory = self.memory.clone();
        
        tokio::spawn(async move {
            let client = Client::new();
            
            loop {
                // Get all watching tasks
                if let Ok(tasks) = memory.get_tasks() {
                    for (id, goal, status) in tasks {
                        if status == "watching" && goal.starts_with("Watch ") {
                            // Parse "Watch <url> every <interval>"
                            let parts: Vec<&str> = goal.split_whitespace().collect();
                            if parts.len() == 4 && parts[0] == "Watch" && parts[2] == "every" {
                                let url = parts[1];
                                // We check it periodically. In a real system, we'd sleep per task based on interval.
                                // For simplicity, we just fetch it and check if it changed since last fetch.
                                
                                match client.get(url).send().await {
                                    Ok(resp) => {
                                        if let Ok(text) = resp.text().await {
                                            let mut hasher = Sha256::new();
                                            hasher.update(text.as_bytes());
                                            let current_hash = format!("{:x}", hasher.finalize());
                                            
                                            let pref_key = format!("watchdog_{}", id);
                                            let last_hash = memory.get_preference(&pref_key).unwrap_or(None);
                                            
                                            if let Some(lh) = last_hash {
                                                if lh != current_hash {
                                                    println!("\n[Watchdog] 🔔 Alert! Content changed for URL: {}", url);
                                                    if let Ok(parsed_id) = uuid::Uuid::parse_str(&id) {
                                                        let _ = memory.log_message(
                                                            parsed_id, 
                                                            "INFO", 
                                                            &format!("Content changed for URL: {}", url)
                                                        );
                                                    }
                                                    let _ = memory.set_preference(&pref_key, &current_hash);
                                                }
                                            } else {
                                                // First time checking
                                                let _ = memory.set_preference(&pref_key, &current_hash);
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        eprintln!("[Watchdog] Failed to fetch {}: {}", url, e);
                                    }
                                }
                            }
                        }
                    }
                }
                
                // Check every 60 seconds (in a real app, parse the interval)
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        });
        
        Ok(())
    }
}
