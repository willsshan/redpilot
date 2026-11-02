// 发布版在 Windows 上不弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `redpilot --mcp-stdio`：不启动界面，作为 stdio MCP 服务供其他 AI 客户端拉起
    if std::env::args().any(|a| a == "--mcp-stdio") {
        redpilot_lib::run_mcp_stdio();
        return;
    }
    redpilot_lib::run();
}
