//! 独立的 stdio MCP 服务，供 Claude Desktop / Cursor 等客户端直接拉起。
//! 与桌面端共用同一份配置、钥匙串和本地缓存。

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdout 是 MCP 协议通道，日志只能写到 stderr
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let svc = redpilot_core::Service::open_default()?;
    redpilot_core::mcp::serve_stdio(svc).await
}
