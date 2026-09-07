use crate::model::{now_ms, LauncherConfig, CONFIG_VERSION};
use serde_json::{Map, Value};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use uuid::Uuid;

const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct LoadedConfig {
    pub config: LauncherConfig,
    pub stamp: Option<Vec<u8>>,
}

pub struct ConfigStore {
    pub directory: PathBuf,
    pub path: PathBuf,
}

impl ConfigStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            path: directory.join("config.json"),
            directory,
        }
    }

    /// Loading never commits a candidate. The application adopts it only after
    /// removed/changed running services have actually stopped.
    pub fn load(&self) -> Result<LoadedConfig, String> {
        let bytes = self.read_with_retry()?;
        let Some(bytes) = bytes else {
            return Ok(LoadedConfig {
                config: LauncherConfig::default(),
                stamp: None,
            });
        };
        match parse_config(&bytes) {
            Ok(mut config) => {
                config.version = CONFIG_VERSION;
                Ok(LoadedConfig {
                    config,
                    stamp: Some(bytes),
                })
            }
            Err(error) => {
                let backup = self.back_up(&bytes);
                let note = match backup {
                    Ok(path) => format!("已备份到：{}", path.display()),
                    Err(reason) => format!("创建备份失败：{reason}。请手动复制原文件"),
                };
                Err(format!(
                    "配置文件无效：{error}\n{}\n{note}\n原文件未被覆盖；修复后点击重新读取。",
                    self.path.display()
                ))
            }
        }
    }

    fn read_with_retry(&self) -> Result<Option<Vec<u8>>, String> {
        for attempt in 0..3 {
            match self.read_bytes() {
                Ok(value) => return Ok(value),
                Err(error)
                    if attempt < 2
                        && matches!(
                            error.kind(),
                            io::ErrorKind::PermissionDenied
                                | io::ErrorKind::WouldBlock
                                | io::ErrorKind::Other
                        ) =>
                {
                    thread::sleep(Duration::from_millis(200 * (attempt + 1)));
                }
                Err(error) => return Err(format!("无法读取配置 {}：{error}", self.path.display())),
            }
        }
        unreachable!()
    }

    fn read_bytes(&self) -> io::Result<Option<Vec<u8>>> {
        match fs::metadata(&self.path) {
            Ok(meta) if meta.len() > MAX_CONFIG_BYTES => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "配置文件超过 4 MB",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            _ => {}
        }
        fs::read(&self.path).map(Some)
    }

    /// Compare with the last accepted disk content before replacing a file.
    /// This also prevents window-placement saves from erasing external edits.
    pub fn save(
        &self,
        config: &LauncherConfig,
        expected: Option<&[u8]>,
    ) -> Result<Vec<u8>, String> {
        config.validate()?;
        fs::create_dir_all(&self.directory).map_err(|e| format!("无法创建配置目录：{e}"))?;
        let actual = self
            .read_bytes()
            .map_err(|e| format!("无法检查配置文件：{e}"))?;
        if actual.as_deref() != expected {
            return Err("配置已被其他程序修改。请先重新读取配置，再保存更改。".into());
        }
        let mut bytes = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let temporary = self
            .directory
            .join(format!(".config-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            // Check once more after preparing the temporary file.
            if self.read_bytes()?.as_deref() != expected {
                return Err(io::Error::other("配置在保存过程中被外部修改，请重新读取"));
            }
            replace_file(&temporary, &self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|e| format!("配置保存失败：{e}"))?;
        Ok(bytes)
    }

    fn back_up(&self, bytes: &[u8]) -> io::Result<PathBuf> {
        let path = self.directory.join(format!(
            "config.json.broken-{}-{}",
            now_ms(),
            Uuid::new_v4()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(path)
    }
}

pub fn parse_config(bytes: &[u8]) -> Result<LauncherConfig, String> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("JSON 解析失败：{e}"))?;
    let value = normalize_keys(value)?;
    let config: LauncherConfig =
        serde_json::from_value(value).map_err(|e| format!("配置字段不正确：{e}"))?;
    config.validate()?;
    Ok(config)
}

/// The WinForms format used PascalCase and accepted arbitrary property casing.
/// Known names are normalized; unknown fields survive a load/save round trip.
fn normalize_keys(value: Value) -> Result<Value, String> {
    const NAMES: &[&str] = &[
        "version",
        "items",
        "settings",
        "mainWindowBounds",
        "settingsWindowBounds",
        "id",
        "name",
        "category",
        "target",
        "arguments",
        "workingDirectory",
        "url",
        "launchType",
        "autoStart",
        "hideWindow",
        "enabled",
        "startWithWindows",
        "startMinimizedToTray",
        "minimizeToTray",
        "closeToTray",
        "confirmBeforeStopAll",
        "autoStartIntervalMs",
        "x",
        "y",
        "width",
        "height",
    ];
    match value {
        Value::Object(object) => {
            let mut normalized = Map::new();
            for (key, value) in object {
                let name = NAMES
                    .iter()
                    .find(|name| name.eq_ignore_ascii_case(&key))
                    .map(|name| (*name).to_owned())
                    .unwrap_or(key);
                if normalized.contains_key(&name) {
                    return Err(format!("配置中存在大小写重复的属性：{name}"));
                }
                normalized.insert(name, normalize_keys(value)?);
            }
            Ok(Value::Object(normalized))
        }
        Value::Array(values) => values
            .into_iter()
            .map(normalize_keys)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        other => Ok(other),
    }
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Both paths are in the same directory; replacement never crosses volumes.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LaunchItem, LaunchType};

    #[test]
    fn imports_case_insensitive_winforms_config_and_preserves_extensions() {
        let json = br#"{"VERSION":1,"Items":[{"Id":"11111111-1111-1111-1111-111111111111","Name":"demo","Target":"run.py","LaunchType":"pYtHoN","CustomFlag":7}],"Settings":{"AutoStartIntervalMs":500},"MainWindowBounds":{"X":20,"Y":30,"Width":1280,"Height":800,"Location":{"X":20,"Y":30}},"customRoot":"keep"}"#;
        let config = parse_config(json).unwrap();
        assert_eq!(config.items[0].launch_type, LaunchType::Python);
        assert_eq!(config.settings.auto_start_interval_ms, 500);
        assert_eq!(config.items[0].extra["CustomFlag"], 7);
        assert_eq!(config.extra["customRoot"], "keep");
    }

    #[test]
    fn rejects_nulls_unknown_types_duplicate_ids_and_future_versions() {
        for json in [
            r#"null"#,
            r#"{"items":[null]}"#,
            r#"{"settings":null}"#,
            r#"{"version":99}"#,
            r#"{"version":1,"VERSION":2}"#,
            r#"{"items":[{"id":"11111111-1111-1111-1111-111111111111","name":"x","target":"x","launchType":999}]}"#,
            r#"{"items":[{"id":"11111111-1111-1111-1111-111111111111","name":"x","target":"x"},{"id":"11111111-1111-1111-1111-111111111111","name":"y","target":"y"}]}"#,
        ] {
            assert!(parse_config(json.as_bytes()).is_err(), "accepted {json}");
        }
    }

    #[test]
    fn invalid_config_is_backed_up_without_overwriting_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().into());
        let original = b"{broken json";
        fs::write(&store.path, original).unwrap();
        assert!(store.load().is_err());
        assert_eq!(fs::read(&store.path).unwrap(), original);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn saves_atomically_and_refuses_to_overwrite_external_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path().into());
        let mut config = LauncherConfig::default();
        config.items.push(LaunchItem::new("demo", "demo.exe"));
        let stamp = store.save(&config, None).unwrap();
        assert_eq!(store.load().unwrap().config.items.len(), 1);
        fs::write(&store.path, b"{}").unwrap();
        assert!(store.save(&config, Some(&stamp)).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), b"{}");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
