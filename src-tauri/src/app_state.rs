use crate::{
    autostart,
    config::{ConfigStore, LoadedConfig},
    launch::resolve_path,
    logs::{LogStore, LogTab},
    model::{LaunchItem, LauncherConfig, LauncherSettings, RuntimeStatus, WindowBounds},
    process::ProcessManager,
};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub config: LauncherConfig,
    pub statuses: Vec<RuntimeStatus>,
    pub log_tabs: Vec<LogTab>,
    pub config_path: String,
    pub load_error: Option<String>,
    pub auto_start_registered: bool,
    pub auto_starting: bool,
    pub platform: &'static str,
    pub version: &'static str,
}

#[derive(Clone, Serialize)]
pub struct Notice {
    pub level: &'static str,
    pub message: String,
}

#[derive(Serialize)]
pub struct ActionFailure {
    pub id: Uuid,
    pub name: String,
    pub error: String,
}

#[derive(Serialize)]
pub struct ActionResult {
    pub snapshot: Snapshot,
    pub failures: Vec<ActionFailure>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

#[derive(Serialize)]
pub struct DroppedItems {
    pub items: Vec<LaunchItem>,
    pub failures: Vec<String>,
}

pub struct AppState {
    pub store: ConfigStore,
    loaded: RwLock<Option<LoadedConfig>>,
    load_error: RwLock<Option<String>>,
    pub logs: Arc<LogStore>,
    pub processes: Arc<ProcessManager>,
    control: Mutex<()>,
    revision: AtomicU64,
    auto_epoch: AtomicU64,
    auto_starting: AtomicBool,
    registered: AtomicBool,
    pub exiting: AtomicBool,
    app: RwLock<Option<AppHandle>>,
    placement: RwLock<Option<WindowBounds>>,
    placement_dirty: AtomicBool,
}

impl AppState {
    pub fn new(directory: PathBuf) -> Arc<Self> {
        let store = ConfigStore::new(directory.clone());
        let (loaded, error) = match store.load() {
            Ok(loaded) => (Some(loaded), None),
            Err(error) => (None, Some(error)),
        };
        let logs = Arc::new(LogStore::default());
        let processes = ProcessManager::new(directory, logs.clone());
        let state = Arc::new(Self {
            store,
            loaded: RwLock::new(loaded),
            load_error: RwLock::new(error),
            logs,
            processes,
            control: Mutex::new(()),
            revision: AtomicU64::new(1),
            auto_epoch: AtomicU64::new(0),
            auto_starting: AtomicBool::new(false),
            registered: AtomicBool::new(autostart::enabled()),
            exiting: AtomicBool::new(false),
            app: RwLock::new(None),
            placement: RwLock::new(None),
            placement_dirty: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&state);
        state.processes.set_change_handler(Arc::new(move || {
            if let Some(state) = weak.upgrade() {
                state.publish();
            }
        }));
        state
    }

    pub fn attach(self: &Arc<Self>, app: AppHandle) {
        *self.app.write() = Some(app);
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            while !state.exiting.load(Ordering::Acquire) {
                interval.tick().await;
                if let Some(batch) = state.logs.drain_batch() {
                    if let Some(app) = state.app.read().as_ref() {
                        let _ = app.emit("log-batch", batch);
                    }
                }
            }
        });
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            while !state.exiting.load(Ordering::Acquire) {
                interval.tick().await;
                if state.placement_dirty.swap(false, Ordering::AcqRel) {
                    if let Err(error) = state.save_placement().await {
                        state.notice("error", error);
                    }
                }
            }
        });
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            revision: self.revision.load(Ordering::Acquire),
            config: self
                .loaded
                .read()
                .as_ref()
                .map(|loaded| loaded.config.clone())
                .unwrap_or_default(),
            statuses: self.processes.statuses(),
            log_tabs: self.logs.tabs(),
            config_path: self.store.path.to_string_lossy().into(),
            load_error: self.load_error.read().clone(),
            auto_start_registered: self.registered.load(Ordering::Acquire),
            auto_starting: self.auto_starting.load(Ordering::Acquire),
            platform: std::env::consts::OS,
            version: env!("CARGO_PKG_VERSION"),
        }
    }

    pub fn publish(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
        let app = self.app.read().clone();
        if let Some(app) = app {
            let _ = app.emit("snapshot-changed", self.snapshot());
        }
    }

    pub fn notice(&self, level: &'static str, message: impl Into<String>) {
        if let Some(app) = self.app.read().as_ref() {
            let _ = app.emit(
                "app-notice",
                Notice {
                    level,
                    message: message.into(),
                },
            );
        }
    }

    pub fn config(&self) -> Result<LauncherConfig, String> {
        self.loaded
            .read()
            .as_ref()
            .map(|loaded| loaded.config.clone())
            .ok_or_else(|| "请先修复配置文件并重新读取，启动器不会覆盖原文件".into())
    }

    fn ready(&self) -> Result<LauncherConfig, String> {
        if self.exiting.load(Ordering::Acquire) {
            return Err("启动器正在退出".into());
        }
        self.config()
    }

    fn commit(&self, mut config: LauncherConfig) -> Result<(), String> {
        if let Some(bounds) = *self.placement.read() {
            config.main_window_bounds = Some(bounds);
        }
        let current = self.loaded.read().clone().ok_or("配置尚未成功加载")?;
        let stamp = self.store.save(&config, current.stamp.as_deref())?;
        *self.loaded.write() = Some(LoadedConfig {
            config,
            stamp: Some(stamp),
        });
        self.publish();
        Ok(())
    }

    pub async fn save_item(&self, mut item: LaunchItem) -> Result<Snapshot, String> {
        let _guard = self.control.lock().await;
        let mut config = self.ready()?;
        item.name = item.name.trim().into();
        item.target = item.target.trim().into();
        if item.category.trim().is_empty() {
            item.category = "本地服务".into();
        }
        item.validate()?;
        if let Some(index) = config.items.iter().position(|old| old.id == item.id) {
            if self.processes.is_running(item.id) {
                return Err("请先停止此服务，再修改启动配置".into());
            }
            config.items[index] = item;
        } else {
            config.items.push(item);
        }
        self.commit(config)?;
        Ok(self.snapshot())
    }

    pub async fn set_auto_start(&self, ids: Vec<Uuid>, enabled: bool) -> Result<Snapshot, String> {
        let _guard = self.control.lock().await;
        let mut config = self.ready()?;
        let ids: HashSet<_> = ids.into_iter().collect();
        for item in &mut config.items {
            if ids.contains(&item.id) {
                item.auto_start = enabled;
            }
        }
        self.commit(config)?;
        Ok(self.snapshot())
    }

    pub async fn action(
        self: &Arc<Self>,
        ids: Vec<Uuid>,
        action: ServiceAction,
    ) -> Result<ActionResult, String> {
        let _guard = self.control.lock().await;
        let config = self.ready()?;
        let mut failures = Vec::new();
        let mut visited = HashSet::new();
        for id in ids {
            if !visited.insert(id) {
                continue;
            }
            let Some(item) = config.items.iter().find(|item| item.id == id) else {
                failures.push(ActionFailure {
                    id,
                    name: id.to_string(),
                    error: "启动项已不存在，请刷新列表".into(),
                });
                continue;
            };
            let result = match action {
                ServiceAction::Start if !item.enabled => Err("此启动项已禁用".into()),
                ServiceAction::Restart if !item.enabled && !self.processes.is_running(id) => {
                    Err("此启动项已禁用".into())
                }
                ServiceAction::Start => self.processes.start(item).await,
                ServiceAction::Stop => self.processes.stop(id).await,
                ServiceAction::Restart => self.processes.restart(item).await,
            };
            if let Err(error) = result {
                failures.push(ActionFailure {
                    id,
                    name: item.name.clone(),
                    error,
                });
            }
        }
        Ok(ActionResult {
            snapshot: self.snapshot(),
            failures,
        })
    }

    pub fn cancel_auto(&self) {
        self.auto_epoch.fetch_add(1, Ordering::AcqRel);
        self.auto_starting.store(false, Ordering::Release);
        self.publish();
    }

    pub async fn stop_all(self: &Arc<Self>) -> Result<ActionResult, String> {
        self.cancel_auto();
        let _guard = self.control.lock().await;
        let names = self.config().map(|config| config.items).unwrap_or_default();
        let mut tasks = tokio::task::JoinSet::new();
        for id in self.processes.active_ids() {
            let manager = self.processes.clone();
            tasks.spawn(async move { (id, manager.stop(id).await) });
        }
        let mut failures = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok((id, Err(error))) => failures.push(ActionFailure {
                    id,
                    name: names
                        .iter()
                        .find(|item| item.id == id)
                        .map(|item| item.name.clone())
                        .unwrap_or_else(|| id.to_string()),
                    error,
                }),
                Err(error) => failures.push(ActionFailure {
                    id: Uuid::nil(),
                    name: "停止服务".into(),
                    error: error.to_string(),
                }),
                _ => {}
            }
        }
        Ok(ActionResult {
            snapshot: self.snapshot(),
            failures,
        })
    }

    pub async fn delete_items(self: &Arc<Self>, ids: Vec<Uuid>) -> Result<ActionResult, String> {
        self.cancel_auto();
        let _guard = self.control.lock().await;
        let mut config = self.ready()?;
        let ids: HashSet<_> = ids.into_iter().collect();
        let selected: Vec<_> = config
            .items
            .iter()
            .filter(|item| ids.contains(&item.id))
            .cloned()
            .collect();
        let mut removed = Vec::new();
        let mut failures = Vec::new();
        for item in selected {
            if let Err(error) = self.processes.stop(item.id).await {
                failures.push(ActionFailure {
                    id: item.id,
                    name: item.name,
                    error,
                });
                continue;
            }
            config.items.retain(|entry| entry.id != item.id);
            removed.push(item.id);
        }
        // Commit before dropping logs/state. A write failure retains the entry.
        self.commit(config)?;
        for id in removed {
            self.processes.forget(id)?;
        }
        self.publish();
        Ok(ActionResult {
            snapshot: self.snapshot(),
            failures,
        })
    }

    pub async fn reload(self: &Arc<Self>) -> Result<Snapshot, String> {
        self.cancel_auto();
        let _guard = self.control.lock().await;
        let candidate = self.store.load()?;
        let previous = self.loaded.read().clone();
        if previous
            .as_ref()
            .is_some_and(|loaded| loaded.stamp.is_some())
            && candidate.stamp.is_none()
        {
            return Err("配置文件暂时不存在，已保留当前启动项。请恢复文件后重新读取。".into());
        }
        for id in self.processes.active_ids() {
            let old = previous
                .as_ref()
                .and_then(|loaded| loaded.config.items.iter().find(|item| item.id == id));
            let new = candidate.config.items.iter().find(|item| item.id == id);
            let changed = match (old, new) {
                (Some(old), Some(new)) => {
                    old.target != new.target
                        || old.arguments != new.arguments
                        || old.working_directory != new.working_directory
                        || old.launch_type != new.launch_type
                        || old.hide_window != new.hide_window
                }
                _ => true,
            };
            if changed {
                self.processes.stop(id).await.map_err(|error| {
                    format!(
                        "配置尚未切换：服务 {} 未能停止。{error}",
                        old.map(|item| item.name.as_str()).unwrap_or("未知")
                    )
                })?;
            }
        }
        let valid_ids: HashSet<_> = candidate.config.items.iter().map(|item| item.id).collect();
        if let Some(previous) = previous {
            for item in previous.config.items {
                if !valid_ids.contains(&item.id) {
                    self.processes.forget(item.id)?;
                }
            }
        }
        for tab in self.logs.tabs() {
            if let Some(item) = candidate.config.items.iter().find(|item| item.id == tab.id) {
                self.logs.ensure(tab.id, &item.name);
            }
        }
        *self.loaded.write() = Some(candidate);
        *self.load_error.write() = None;
        self.registered
            .store(autostart::enabled(), Ordering::Release);
        self.publish();
        Ok(self.snapshot())
    }

    pub async fn save_settings(&self, settings: LauncherSettings) -> Result<Snapshot, String> {
        let _guard = self.control.lock().await;
        let mut config = self.ready()?;
        config.settings = settings;
        config.settings.start_minimized_to_tray &= config.settings.start_with_windows;
        config.validate()?;
        let should_register = config.settings.start_with_windows;
        let current_registered = autostart::enabled();
        let change_registry = should_register != current_registered;
        let previous = if change_registry {
            autostart::read()?
        } else {
            None
        };
        if change_registry {
            let command = should_register
                .then(autostart::registered_command)
                .transpose()?;
            autostart::write(command.as_deref())
                .map_err(|error| format!("开机自启动设置失败，配置未保存：{error}"))?;
        }
        if let Err(error) = self.commit(config) {
            if change_registry {
                if let Err(rollback) = autostart::write(previous.as_deref()) {
                    return Err(format!("{error}\n开机自启动恢复失败：{rollback}"));
                }
            }
            return Err(error);
        }
        self.registered
            .store(autostart::enabled(), Ordering::Release);
        self.publish();
        Ok(self.snapshot())
    }

    pub fn start_auto(self: &Arc<Self>) -> Result<Snapshot, String> {
        let config = self.ready()?;
        if self.auto_starting.swap(true, Ordering::AcqRel) {
            return Ok(self.snapshot());
        }
        let epoch = self.auto_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let ids: Vec<_> = config
            .items
            .iter()
            .filter(|item| item.enabled && item.auto_start)
            .map(|item| item.id)
            .collect();
        let state = self.clone();
        self.publish();
        tauri::async_runtime::spawn(async move {
            for (index, id) in ids.iter().enumerate() {
                let interval = {
                    let _guard = state.control.lock().await;
                    if state.auto_epoch.load(Ordering::Acquire) != epoch
                        || state.exiting.load(Ordering::Acquire)
                    {
                        break;
                    }
                    let Ok(config) = state.config() else {
                        break;
                    };
                    if let Some(item) = config
                        .items
                        .iter()
                        .find(|item| item.id == *id && item.enabled && item.auto_start)
                    {
                        if let Err(error) = state.processes.start(item).await {
                            state
                                .notice("error", format!("「{}」自动启动失败：{error}", item.name));
                        }
                    }
                    config.settings.auto_start_interval_ms
                };
                if index + 1 < ids.len() {
                    tokio::time::sleep(Duration::from_millis(interval.into())).await;
                }
            }
            if state.auto_epoch.load(Ordering::Acquire) == epoch {
                state.auto_starting.store(false, Ordering::Release);
                state.publish();
            }
        });
        Ok(self.snapshot())
    }

    pub fn remember_placement(&self, bounds: WindowBounds) {
        *self.placement.write() = Some(bounds);
        self.placement_dirty.store(true, Ordering::Release);
    }

    pub async fn save_placement(&self) -> Result<(), String> {
        let _guard = self.control.lock().await;
        if self.loaded.read().is_none() || self.placement.read().is_none() {
            return Ok(());
        }
        self.commit(self.config()?)
    }

    pub fn directory_for(&self, id: Option<Uuid>) -> Result<PathBuf, String> {
        let Some(id) = id else {
            return Ok(self.store.directory.clone());
        };
        let config = self.config()?;
        let item = config
            .items
            .iter()
            .find(|item| item.id == id)
            .ok_or("启动项不存在")?;
        let directory = if !item.working_directory.trim().is_empty() {
            resolve_path(&item.working_directory, &self.store.directory)
        } else {
            let file = resolve_path(&item.target, &self.store.directory);
            if file.is_file() {
                file.parent().unwrap_or(&self.store.directory).to_path_buf()
            } else {
                self.store.directory.clone()
            }
        };
        if !directory.is_dir() {
            return Err(format!("工作目录不存在：{}", directory.display()));
        }
        Ok(directory)
    }

    pub fn prepare_dropped(&self, paths: Vec<String>) -> Result<DroppedItems, String> {
        self.ready()?;
        if paths.len() > 100 {
            return Err("一次最多添加 100 个文件或文件夹".into());
        }
        let mut result = DroppedItems {
            items: Vec::new(),
            failures: Vec::new(),
        };
        for path in paths {
            let path = resolve_path(&path, &self.store.directory);
            match item_from_path(&path) {
                Ok(item) => result.items.push(item),
                Err(error) => result.failures.push(error),
            }
        }
        Ok(result)
    }
}

fn item_from_path(path: &Path) -> Result<LaunchItem, String> {
    let (target, directory, name) = if path.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && file_priority(path).is_some())
            .collect();
        files.sort_by_key(|path| {
            (
                file_priority(path),
                path.file_name().map(|name| name.len()),
                path.clone(),
            )
        });
        let target = files
            .into_iter()
            .next()
            .ok_or_else(|| format!("{} 中没有找到常见启动文件，请手动添加", path.display()))?;
        (
            target,
            path.to_path_buf(),
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        )
    } else if path.is_file() {
        (
            path.to_path_buf(),
            path.parent().unwrap_or(path).to_path_buf(),
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        return Err(format!("文件不存在：{}", path.display()));
    };
    let mut item = LaunchItem::new(name, target.to_string_lossy());
    item.working_directory = directory.to_string_lossy().into();
    Ok(item)
}

fn file_priority(path: &Path) -> Option<u8> {
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "bat" if name.starts_with("start") => Some(0),
        "bat" if name.starts_with("run") => Some(1),
        "bat" | "cmd" => Some(2),
        "exe" | "com" => Some(3),
        "ps1" => Some(4),
        "py" | "pyw" => Some(5),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LaunchType;

    fn seed(directory: &Path, config: &LauncherConfig) {
        std::fs::write(
            directory.join("config.json"),
            serde_json::to_vec_pretty(config).unwrap(),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn bad_refresh_keeps_the_last_accepted_config() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = LauncherConfig::default();
        let item = LaunchItem::new("keep", "keep.exe");
        config.items.push(item.clone());
        seed(directory.path(), &config);
        let state = AppState::new(directory.path().into());
        std::fs::write(state.store.path.clone(), b"null").unwrap();
        assert!(state.reload().await.is_err());
        assert_eq!(state.config().unwrap().items[0].id, item.id);
        assert_eq!(std::fs::read(&state.store.path).unwrap(), b"null");
        assert!(state
            .save_item(LaunchItem::new("new", "new.exe"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn missing_file_during_refresh_does_not_delete_the_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = LauncherConfig::default();
        config.items.push(LaunchItem::new("keep", "keep.exe"));
        seed(directory.path(), &config);
        let state = AppState::new(directory.path().into());
        std::fs::remove_file(&state.store.path).unwrap();
        assert!(state.reload().await.is_err());
        assert_eq!(state.config().unwrap().items.len(), 1);
    }

    #[tokio::test]
    async fn invalid_initial_config_cannot_be_overwritten_by_an_edit() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("config.json"), b"{broken").unwrap();
        let state = AppState::new(directory.path().into());
        assert!(state.snapshot().load_error.is_some());
        assert!(state
            .save_item(LaunchItem::new("new", "new.exe"))
            .await
            .is_err());
        assert_eq!(std::fs::read(&state.store.path).unwrap(), b"{broken");
    }

    #[test]
    fn folder_detection_respects_start_script_priority() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a.exe"), "").unwrap();
        std::fs::write(directory.path().join("start-service.bat"), "").unwrap();
        assert!(item_from_path(directory.path())
            .unwrap()
            .target
            .ends_with("start-service.bat"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn refreshed_command_stops_the_old_process_before_adoption() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = LauncherConfig::default();
        let mut item = LaunchItem::new("sleep", "Start-Sleep -Seconds 30");
        item.launch_type = LaunchType::WindowsPowerShell;
        config.items.push(item.clone());
        seed(directory.path(), &config);
        let state = AppState::new(directory.path().into());
        state
            .action(vec![item.id], ServiceAction::Start)
            .await
            .unwrap();
        assert!(state.processes.is_running(item.id));
        config.items[0].target = "Write-Output 'changed'".into();
        seed(directory.path(), &config);
        state.reload().await.unwrap();
        assert!(!state.processes.is_running(item.id));
        assert_eq!(
            state.config().unwrap().items[0].target,
            "Write-Output 'changed'"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn stop_all_cancels_items_waiting_in_the_auto_start_queue() {
        let directory = tempfile::tempdir().unwrap();
        let mut config = LauncherConfig::default();
        config.settings.auto_start_interval_ms = 250;
        for name in ["first", "second"] {
            let mut item = LaunchItem::new(name, "Start-Sleep -Seconds 30");
            item.auto_start = true;
            item.launch_type = LaunchType::WindowsPowerShell;
            config.items.push(item);
        }
        seed(directory.path(), &config);
        let state = AppState::new(directory.path().into());
        state.start_auto().unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !state.processes.is_running(config.items[0].id) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let result = state.stop_all().await.unwrap();
        assert!(result.failures.is_empty());
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(state.processes.active_ids().is_empty());
        assert!(!state.snapshot().auto_starting);
    }
}
