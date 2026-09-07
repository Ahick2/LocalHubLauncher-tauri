use crate::{
    app_state::{ActionResult, AppState, DroppedItems, ServiceAction, Snapshot},
    logs::LogPage,
    model::{normalized_url, LaunchItem, LauncherSettings},
};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

#[tauri::command]
pub async fn get_snapshot(state: State<'_, Arc<AppState>>) -> Result<Snapshot, String> {
    Ok(state.snapshot())
}

#[tauri::command]
pub async fn save_item(
    state: State<'_, Arc<AppState>>,
    item: LaunchItem,
) -> Result<Snapshot, String> {
    state.save_item(item).await
}

#[tauri::command]
pub async fn delete_items(
    state: State<'_, Arc<AppState>>,
    ids: Vec<Uuid>,
) -> Result<ActionResult, String> {
    state.inner().delete_items(ids).await
}

#[tauri::command]
pub async fn service_action(
    state: State<'_, Arc<AppState>>,
    ids: Vec<Uuid>,
    action: ServiceAction,
) -> Result<ActionResult, String> {
    state.inner().action(ids, action).await
}

#[tauri::command]
pub async fn stop_all(state: State<'_, Arc<AppState>>) -> Result<ActionResult, String> {
    state.inner().stop_all().await
}

#[tauri::command]
pub async fn start_auto(state: State<'_, Arc<AppState>>) -> Result<Snapshot, String> {
    state.inner().start_auto()
}

#[tauri::command]
pub async fn set_auto_start(
    state: State<'_, Arc<AppState>>,
    ids: Vec<Uuid>,
    enabled: bool,
) -> Result<Snapshot, String> {
    state.set_auto_start(ids, enabled).await
}

#[tauri::command]
pub async fn reload_config(state: State<'_, Arc<AppState>>) -> Result<Snapshot, String> {
    state.inner().reload().await
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, Arc<AppState>>,
    settings: LauncherSettings,
) -> Result<Snapshot, String> {
    state.save_settings(settings).await
}

#[tauri::command]
pub async fn get_logs(
    state: State<'_, Arc<AppState>>,
    id: Option<Uuid>,
) -> Result<LogPage, String> {
    if let Some(id) = id {
        let config = state.config()?;
        let item = config
            .items
            .iter()
            .find(|item| item.id == id)
            .ok_or("启动项不存在")?;
        if !state.logs.tabs().iter().any(|tab| tab.id == id) {
            state.logs.ensure(id, &item.name);
            state.publish();
        }
    }
    Ok(state.logs.page(id))
}

#[tauri::command]
pub async fn clear_logs(
    state: State<'_, Arc<AppState>>,
    id: Option<Uuid>,
) -> Result<LogPage, String> {
    state.logs.clear(id);
    state.publish();
    Ok(state.logs.page(id))
}

#[tauri::command]
pub async fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    let url = normalized_url(&url)?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| format!("无法打开网址：{e}"))
}

#[tauri::command]
pub async fn open_directory(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: Option<Uuid>,
) -> Result<(), String> {
    let directory = state.directory_for(id)?;
    app.opener()
        .open_path(directory.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("无法打开目录：{e}"))
}

#[tauri::command]
pub async fn prepare_dropped_items(
    state: State<'_, Arc<AppState>>,
    paths: Vec<String>,
) -> Result<DroppedItems, String> {
    state.prepare_dropped(paths)
}

#[tauri::command]
pub fn exit_app(app: AppHandle) {
    crate::desktop::request_exit(app);
}
