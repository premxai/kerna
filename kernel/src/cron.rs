use crate::config::Config;
use crate::mcp_registry::McpRegistry;
use crate::memory::MemoryEngine;
use crate::scheduler::TaskScheduler;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};

pub struct CronEngine {
    scheduler: JobScheduler,
    config: Config,
    memory: Arc<MemoryEngine>,
    mcp_registry: Arc<Mutex<McpRegistry>>,
    running_jobs: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
}

impl CronEngine {
    pub async fn new(
        config: Config,
        memory: Arc<MemoryEngine>,
        mcp_registry: Arc<Mutex<McpRegistry>>,
    ) -> Result<Self> {
        let scheduler = JobScheduler::new().await?;
        Ok(CronEngine {
            scheduler,
            config,
            memory,
            mcp_registry,
            running_jobs: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
        })
    }

    pub async fn start(&mut self) -> Result<()> {
        for schedule in &self.config.schedules {
            if !schedule.enabled {
                continue;
            }

            let cron_expr = schedule.cron.clone();
            let goal = schedule.goal.clone();

            let config_clone = self.config.clone();
            let memory_clone = self.memory.clone();
            let mcp_registry_clone = self.mcp_registry.clone();
            let running_jobs_clone = self.running_jobs.clone();

            println!("[Cron] Registering schedule: '{}' -> {}", cron_expr, goal);

            let job = Job::new_async(cron_expr.as_str(), move |_uuid, mut _l| {
                let goal_clone = goal.clone();
                let config_c = config_clone.clone();
                let memory_c = memory_clone.clone();
                let mcp_c = mcp_registry_clone.clone();
                let running_c = running_jobs_clone.clone();

                Box::pin(async move {
                    {
                        let mut running = running_c.lock().await;
                        if running.contains(&goal_clone) {
                            println!(
                                "[Cron] Skipping scheduled goal (already running): {}",
                                goal_clone
                            );
                            return;
                        }
                        running.insert(goal_clone.clone());
                    }

                    println!("\n[Cron] Triggering scheduled goal: {}", goal_clone);
                    match TaskScheduler::new(config_c, memory_c, mcp_c, None) {
                        Ok(task_scheduler) => {
                            if let Err(e) = task_scheduler.run_goal(&goal_clone).await {
                                eprintln!("[Cron] Scheduled goal failed: {}", e);
                            }
                        }
                        Err(e) => eprintln!("[Cron] Failed to initialize TaskScheduler: {}", e),
                    }

                    {
                        let mut running = running_c.lock().await;
                        running.remove(&goal_clone);
                    }
                })
            })?;

            self.scheduler.add(job).await?;
        }

        self.scheduler.start().await?;
        Ok(())
    }
}
