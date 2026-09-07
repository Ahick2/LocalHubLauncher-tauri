mod app_state;
mod autostart;
mod commands;
mod config;
mod decoder;
mod desktop;
mod job;
mod launch;
mod logs;
mod model;
mod native_process;
mod process;

pub fn run() {
    use std::sync::Arc;
    use tauri::Manager;
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            desktop::show_window(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(desktop::setup)
        .on_window_event(desktop::window_event)
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::save_item,
            commands::delete_items,
            commands::service_action,
            commands::stop_all,
            commands::start_auto,
            commands::set_auto_start,
            commands::reload_config,
            commands::save_settings,
            commands::get_logs,
            commands::clear_logs,
            commands::open_url,
            commands::open_directory,
            commands::prepare_dropped_items,
            commands::exit_app,
        ])
        .build(tauri::generate_context!())
        .expect("无法启动 Local Hub")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<Arc<app_state::AppState>>() {
                    state.processes.kill_all();
                }
            }
        });
}
