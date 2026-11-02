//! RedPilot 核心库：桌面端（Tauri）与 MCP 服务共用的全部业务逻辑。

pub mod ai;
pub mod analysis;
pub mod config;
pub mod db;
pub mod mask;
pub mod mcp;
pub mod models;
pub mod project;
pub mod prompts;
pub mod redmine;
pub mod secrets;
pub mod service;

pub use config::{AppConfig, Paths};
pub use service::{AppEvent, Service};

/// 带错误码的业务错误，前端/MCP 客户端可据此给出针对性提示。
#[derive(Debug)]
pub struct CodedError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for CodedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for CodedError {}

/// 本地日期，格式 YYYY-MM-DD
pub fn local_today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn coded(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(CodedError {
        code,
        message: message.into(),
    })
}
