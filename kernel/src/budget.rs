use anyhow::{anyhow, Result};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct BudgetConfig {
    pub max_runtime_seconds: u64,
    pub max_tool_calls: u64,
    pub max_llm_calls: u64,
    pub max_cost_usd: f64,
    pub max_output_bytes: u64,
    pub max_memory_writes: u64,
}

#[derive(Debug)]
pub struct BudgetTracker {
    config: BudgetConfig,
    start_time: Instant,
    tool_calls: u64,
    llm_calls: u64,
    cost_usd: f64,
    output_bytes: u64,
    memory_writes: u64,
}

impl BudgetTracker {
    pub fn new(config: BudgetConfig) -> Self {
        Self {
            config,
            start_time: Instant::now(),
            tool_calls: 0,
            llm_calls: 0,
            cost_usd: 0.0,
            output_bytes: 0,
            memory_writes: 0,
        }
    }

    pub fn check_runtime(&self) -> Result<()> {
        let elapsed = self.start_time.elapsed().as_secs();
        if elapsed > self.config.max_runtime_seconds {
            return Err(anyhow!(
                "BUDGET_EXCEEDED: max runtime of {}s exceeded (used {}s)",
                self.config.max_runtime_seconds,
                elapsed
            ));
        }
        Ok(())
    }

    pub fn record_llm_call(&mut self, cost_increment: f64) -> Result<()> {
        self.check_runtime()?;
        if self.llm_calls >= self.config.max_llm_calls {
            return Err(anyhow!(
                "BUDGET_EXCEEDED: max LLM calls of {} exceeded",
                self.config.max_llm_calls
            ));
        }
        self.llm_calls += 1;

        self.cost_usd += cost_increment;
        if self.cost_usd > self.config.max_cost_usd {
            return Err(anyhow!(
                "BUDGET_EXCEEDED: max cost of ${:.2} exceeded (used ${:.2})",
                self.config.max_cost_usd,
                self.cost_usd
            ));
        }

        Ok(())
    }

    pub fn record_tool_call(&mut self) -> Result<()> {
        self.check_runtime()?;
        if self.tool_calls >= self.config.max_tool_calls {
            return Err(anyhow!(
                "BUDGET_EXCEEDED: max tool calls of {} exceeded",
                self.config.max_tool_calls
            ));
        }
        self.tool_calls += 1;
        Ok(())
    }

    pub fn record_output_bytes(&mut self, bytes: u64) -> Result<()> {
        if self.output_bytes + bytes > self.config.max_output_bytes {
            return Err(anyhow!(
                "BUDGET_EXCEEDED: max output bytes of {} exceeded",
                self.config.max_output_bytes
            ));
        }
        self.output_bytes += bytes;
        Ok(())
    }

    pub fn record_memory_write(&mut self) -> Result<()> {
        if self.memory_writes >= self.config.max_memory_writes {
            return Err(anyhow!(
                "BUDGET_EXCEEDED: max memory writes of {} exceeded",
                self.config.max_memory_writes
            ));
        }
        self.memory_writes += 1;
        Ok(())
    }

    pub fn get_snapshot_json(&self) -> serde_json::Value {
        serde_json::json!({
            "runtime_seconds_used": self.start_time.elapsed().as_secs(),
            "tool_calls_used": self.tool_calls,
            "llm_calls_used": self.llm_calls,
            "cost_usd_used": self.cost_usd,
            "output_bytes_used": self.output_bytes,
            "memory_writes_used": self.memory_writes,
        })
    }
}
