mod commands;

use commands::AppState;
use redpilot_core::models::{IssueChange, SyncReport};
use redpilot_core::{AppEvent, Service};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;
use tracing_subscriber::EnvFilter;

const TRAY_ID: &str = "main";

pub fn run_mcp_stdio() {
    // stdout 是 MCP 协议通道，日志只能写 stderr
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let rt = tokio::runtime::Runtime::new().expect("创建运行时失败");
    let res = rt.block_on(async {
        let svc = Service::open_default()?;
        redpilot_core::mcp::serve_stdio(svc).await
    });
    if let Err(e) = res {
        eprintln!("RedPilot MCP 退出: {e:#}");
        std::process::exit(1);
    }
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let svc = match Service::open_default() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("初始化失败: {e:#}");
            std::process::exit(1);
        }
    };

    // 开机自启时带 --minimized，只在托盘中运行
    let minimized = std::env::args().any(|a| a == "--minimized");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .manage(AppState::new(Arc::clone(&svc)))
        .setup(move |app| {
            if !minimized {
                show_main(app.handle());
            }
            build_tray(app.handle())?;
            forward_events(app.handle().clone(), Arc::clone(&svc));
            spawn_scheduler(Arc::clone(&svc));
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                state.restart_mcp().await;
                update_tray(&handle);
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭窗口只是隐藏到托盘，后台继续同步并提供 MCP 服务
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::set_secret,
            commands::secret_status,
            commands::test_redmine,
            commands::test_ai,
            commands::sync_now,
            commands::list_issues,
            commands::issue_detail,
            commands::mark_read,
            commands::analyze,
            commands::set_classification,
            commands::suggest_batches,
            commands::latest_plan,
            commands::draft_note,
            commands::resolve_issue,
            commands::batch_resolve,
            commands::request_more_info,
            commands::start_progress,
            commands::link_issues,
            commands::change_status,
            commands::stage_attachment,
            commands::recent_changes,
            commands::attachment_bytes,
            commands::get_autostart,
            commands::set_autostart,
            commands::statuses,
            commands::activities,
            commands::stats,
            commands::work_report,
            commands::recent_resolutions,
            commands::mcp_status,
            commands::restart_mcp,
            commands::regenerate_mcp_token,
            commands::open_url,
        ])
        .build(tauri::generate_context!())
        .expect("启动 RedPilot 失败");

    app.run(|app, event| {
        // macOS：点击 Dock 图标时重新显示窗口
        #[cfg(target_os = "macos")]
        if let RunEvent::Reopen { .. } = event {
            show_main(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开 RedPilot", true, None::<&str>)?;
    let sync = MenuItem::with_id(app, "sync", "立即同步", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &sync, &sep, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("RedPilot")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "sync" => {
                let svc = Arc::clone(&app.state::<AppState>().svc);
                tauri::async_runtime::spawn(async move {
                    let _ = svc.sync().await;
                });
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?)
            .icon_as_template(true);
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(icon) = app.default_window_icon() {
            builder = builder.icon(icon.clone());
        }
    }
    builder.build(app)?;
    Ok(())
}

/// 托盘显示待办数量（macOS 在菜单栏图标旁显示数字）
fn update_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let stats = app.state::<AppState>().svc.stats().ok();
    let open = stats.as_ref().map(|s| s.open_total).unwrap_or(0);
    let unread = stats.as_ref().map(|s| s.unread).unwrap_or(0);
    let _ = tray.set_tooltip(Some(format!("RedPilot · {open} 个待办，{unread} 个未读")));
    #[cfg(target_os = "macos")]
    let _ = tray.set_title(if unread > 0 { Some(unread.to_string()) } else { None });
}

fn notify_new(app: &AppHandle, svc: &Service, report: &SyncReport) {
    if !svc.config().sync.notify || report.new_ids.is_empty() {
        return;
    }
    let subjects: Vec<String> = report
        .new_ids
        .iter()
        .take(3)
        .filter_map(|id| svc.db.get_view(*id).ok().flatten())
        .map(|v| format!("#{} {}", v.id, v.subject))
        .collect();
    let _ = app
        .notification()
        .builder()
        .title(format!("收到 {} 个新任务", report.new_ids.len()))
        .body(subjects.join("\n"))
        .show();
}

/// 提出人回复、任务被转回、调为高优先级时通知
fn notify_changes(app: &AppHandle, svc: &Service, changes: &[IssueChange]) {
    if !svc.config().sync.notify_changes {
        return;
    }
    let important: Vec<&IssueChange> = changes.iter().filter(|c| c.important).collect();
    let mut issues: Vec<u64> = important.iter().map(|c| c.issue_id).collect();
    issues.dedup();
    let (title, body) = match issues.as_slice() {
        [] => return,
        [id] => {
            let subject = important.first().map(|c| c.subject.as_str()).unwrap_or("");
            let lines: Vec<&str> = important.iter().map(|c| c.summary.as_str()).collect();
            (format!("#{id} {subject}"), lines.join("\n"))
        }
        _ => {
            let lines: Vec<String> = important.iter().rev().take(3).map(|c| format!("#{} {}", c.issue_id, c.summary)).collect();
            (format!("{} 个任务有新动态", issues.len()), lines.join("\n"))
        }
    };
    let _ = app.notification().builder().title(title).body(body).show();
}

fn notify_due(app: &AppHandle, svc: &Service, report: &SyncReport) {
    if !svc.config().sync.notify_due || report.due_soon.is_empty() {
        return;
    }
    let today = redpilot_core::local_today();
    let lines: Vec<String> = report
        .due_soon
        .iter()
        .take(3)
        .filter_map(|id| svc.db.get_view(*id).ok().flatten())
        .map(|v| {
            let when = if v.due_date.as_deref() == Some(today.as_str()) { "今天" } else { "明天" };
            format!("#{} {}（{when}到期）", v.id, v.subject)
        })
        .collect();
    let _ = app
        .notification()
        .builder()
        .title(format!("{} 个任务即将到期", report.due_soon.len()))
        .body(lines.join("\n"))
        .show();
}

fn forward_events(app: AppHandle, svc: Arc<Service>) {
    // 首次使用时第一次同步会把所有任务都当成“新任务”，此时不弹通知
    let mut skip_first_notify = svc.db.kv_get("last_sync_at").ok().flatten().is_none();
    let mut rx = svc.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let _ = app.emit("redpilot://event", &ev);
                    match &ev {
                        AppEvent::SyncFinished { report } => {
                            if !skip_first_notify {
                                notify_new(&app, &svc, report);
                                notify_changes(&app, &svc, &report.changes);
                            }
                            notify_due(&app, &svc, report);
                            skip_first_notify = false;
                            update_tray(&app);
                        }
                        AppEvent::IssuesChanged | AppEvent::AnalysisFinished { .. } => update_tray(&app),
                        _ => {}
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });
}

/// 后台定时同步：每 20 秒检查一次是否到达配置的同步间隔
fn spawn_scheduler(svc: Arc<Service>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let mut last: Option<Instant> = None;
        loop {
            let cfg = svc.config();
            let minutes = cfg.sync.interval_minutes as u64;
            let configured = !cfg.redmine.base_url.trim().is_empty() && svc.secret_status().redmine;
            let due = minutes > 0 && last.is_none_or(|t| t.elapsed() >= Duration::from_secs(minutes * 60));
            if configured && due {
                if let Err(e) = svc.sync().await {
                    tracing::warn!("定时同步失败: {e:#}");
                }
                last = Some(Instant::now());
            }
            tokio::time::sleep(Duration::from_secs(20)).await;
        }
    });
}
