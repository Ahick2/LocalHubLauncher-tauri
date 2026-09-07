use crate::{app_state::AppState, model::WindowBounds};
use std::{
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

pub fn application_directory() -> Result<PathBuf, String> {
    // An explicit override is useful for isolated tests and managed deployments.
    // Normal portable use always keeps data next to this executable.
    if let Some(directory) = std::env::var_os("LOCALHUB_CONFIG_DIR") {
        let directory = PathBuf::from(directory);
        if !directory.is_absolute() {
            return Err("LOCALHUB_CONFIG_DIR 必须是绝对路径".into());
        }
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        return Ok(directory);
    }
    std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| "无法获取程序目录".into())
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState::new(application_directory()?);
    app.manage(state.clone());
    state.attach(app.handle().clone());
    let window = app.get_webview_window("main").ok_or("主窗口不存在")?;
    let snapshot = state.snapshot();
    if let Some(bounds) = snapshot.config.main_window_bounds {
        let visible = window.available_monitors()?.iter().any(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            i64::from(bounds.x) + i64::from(bounds.width) > i64::from(position.x)
                && i64::from(bounds.y) + i64::from(bounds.height) > i64::from(position.y)
                && i64::from(bounds.x) < i64::from(position.x) + i64::from(size.width)
                && i64::from(bounds.y) < i64::from(position.y) + i64::from(size.height)
        });
        if visible {
            let _ = window.set_position(tauri::PhysicalPosition::new(bounds.x, bounds.y));
            let _ = window.set_size(tauri::PhysicalSize::new(
                bounds.width.max(980),
                bounds.height.max(650),
            ));
        }
    }
    install_tray(app)?;
    let autostart = std::env::args().any(|arg| arg.eq_ignore_ascii_case("--autostart"));
    if snapshot.load_error.is_some()
        || !autostart
        || !snapshot.config.settings.start_minimized_to_tray
    {
        window.show()?;
    }
    if snapshot.load_error.is_none() {
        if let Err(error) = state.start_auto() {
            state.notice("error", error);
        }
    }
    Ok(())
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "auto", "启动所有自动项", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止全部", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit = MenuItem::with_id(app, "exit", "退出 Local Hub", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &start, &stop, &separator, &exit])?;
    let mut tray = TrayIconBuilder::with_id("local-hub")
        .tooltip("Local Hub · 本地服务控制中心")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_window(app),
            "auto" => {
                let state = app.state::<Arc<AppState>>();
                if let Err(error) = state.inner().start_auto() {
                    state.notice("error", error);
                }
            }
            "stop" => request_stop_all(app.clone()),
            "exit" => request_exit(app.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn window_event(window: &tauri::Window, event: &WindowEvent) {
    let Some(state) = window.try_state::<Arc<AppState>>() else {
        return;
    };
    if state.exiting.load(Ordering::Acquire) {
        return;
    }
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if state
                .config()
                .map(|config| config.settings.close_to_tray)
                .unwrap_or(false)
            {
                let _ = window.hide();
            } else {
                request_exit(window.app_handle().clone());
            }
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            if window.is_minimized().unwrap_or(false) {
                if state
                    .config()
                    .map(|config| config.settings.minimize_to_tray)
                    .unwrap_or(false)
                {
                    let _ = window.hide();
                }
            } else if !window.is_maximized().unwrap_or(false) {
                // set_size restores the client area, so save the same dimensions
                // instead of adding the title bar and borders on every launch.
                if let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) {
                    if size.width >= 100 && size.height >= 100 {
                        state.remember_placement(WindowBounds {
                            x: position.x,
                            y: position.y,
                            width: size.width,
                            height: size.height,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

fn request_stop_all(app: AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    let confirm = state
        .config()
        .map(|config| config.settings.confirm_before_stop_all)
        .unwrap_or(false)
        && !state.processes.active_ids().is_empty();
    let stop = move || {
        tauri::async_runtime::spawn(async move {
            match state.stop_all().await {
                Ok(result) if result.failures.is_empty() => {
                    state.notice("success", "全部服务已停止，待启动队列已取消")
                }
                Ok(result) => state.notice(
                    "error",
                    result
                        .failures
                        .iter()
                        .map(|failure| format!("{}：{}", failure.name, failure.error))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                Err(error) => state.notice("error", error),
            }
        });
    };
    if confirm {
        app.dialog()
            .message("确定停止全部服务，并取消待启动的自动项？")
            .title("停止全部")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancel)
            .show(move |accepted| {
                if accepted {
                    stop();
                }
            });
    } else {
        stop();
    }
}

pub fn request_exit(app: AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    if state.exiting.load(Ordering::Acquire) {
        return;
    }
    let has_running = !state.processes.active_ids().is_empty();
    let exit_app = app.clone();
    let exit = move || {
        if state.exiting.swap(true, Ordering::AcqRel) {
            return;
        }
        state.cancel_auto();
        tauri::async_runtime::spawn(async move {
            let _ = state.stop_all().await;
            if let Err(error) = state.save_placement().await {
                state.notice("error", error);
            }
            state.processes.kill_all();
            exit_app.exit(0);
        });
    };
    if has_running {
        app.dialog()
            .message("退出 Local Hub 会停止它启动的全部服务。确定退出？")
            .title("退出 Local Hub")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancel)
            .show(move |accepted| {
                if accepted {
                    exit();
                }
            });
    } else {
        exit();
    }
}
