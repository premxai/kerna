use async_trait::async_trait;
use anyhow::Result;
use crate::config::Config;
use crate::memory::MemoryEngine;
use crate::mcp_registry::McpRegistry;
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait Gateway {
    async fn start(&self, config: Config, memory: Arc<MemoryEngine>, mcp_registry: Arc<Mutex<McpRegistry>>) -> Result<()>;
}

pub struct TelegramGateway;

#[async_trait]
impl Gateway for TelegramGateway {
    async fn start(&self, _config: Config, _memory: Arc<MemoryEngine>, _mcp_registry: Arc<Mutex<McpRegistry>>) -> Result<()> {
        println!("[+] Telegram Gateway starting (Stub)");
        // In a real implementation, we'd initialize teloxide here and listen for messages.
        // We would spawn TaskSchedulers for valid incoming commands.
        Ok(())
    }
}
