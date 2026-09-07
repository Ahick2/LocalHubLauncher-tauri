use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

pub const CONFIG_VERSION: u32 = 2;
pub const MAX_ITEMS: usize = 1000;
pub type Extensions = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum LaunchType {
    #[default]
    Auto,
    Executable,
    Batch,
    PowerShell,
    Python,
    Command,
    Pwsh,
    WindowsPowerShell,
    Wsl,
    GitBash,
}

impl<'de> Deserialize<'de> for LaunchType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        const TYPES: [LaunchType; 10] = [
            LaunchType::Auto,
            LaunchType::Executable,
            LaunchType::Batch,
            LaunchType::PowerShell,
            LaunchType::Python,
            LaunchType::Command,
            LaunchType::Pwsh,
            LaunchType::WindowsPowerShell,
            LaunchType::Wsl,
            LaunchType::GitBash,
        ];
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::String(name) => TYPES
                .into_iter()
                .find(|kind| format!("{kind:?}").eq_ignore_ascii_case(&name))
                .ok_or_else(|| de::Error::custom(format!("未知启动方式：{name}"))),
            Value::Number(number) => number
                .as_u64()
                .and_then(|index| TYPES.get(index as usize).copied())
                .ok_or_else(|| de::Error::custom("启动方式编号必须在 0 到 9 之间")),
            _ => Err(de::Error::custom("启动方式必须是名称或有效编号")),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_category() -> String {
    "本地服务".into()
}
fn legacy_version() -> u32 {
    1
}
fn default_interval() -> u32 {
    800
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchItem {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "default_category")]
    pub category: String,
    pub target: String,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub working_directory: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub launch_type: LaunchType,
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default = "default_true")]
    pub hide_window: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, flatten)]
    pub extra: Extensions,
}

impl LaunchItem {
    pub fn new(name: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            category: default_category(),
            target: target.into(),
            arguments: String::new(),
            working_directory: String::new(),
            url: String::new(),
            launch_type: LaunchType::Auto,
            auto_start: false,
            hide_window: true,
            enabled: true,
            extra: Extensions::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_nil() {
            return Err("启动项 ID 不能为空 GUID".into());
        }
        if self.name.trim().is_empty() {
            return Err("请填写启动项名称".into());
        }
        if self.target.trim().is_empty() {
            return Err(format!("「{}」的启动文件或命令不能为空", self.name));
        }
        for (label, value, limit) in [
            ("名称", &self.name, 200),
            ("分类", &self.category, 200),
            ("启动文件或命令", &self.target, 32767),
            ("启动参数", &self.arguments, 32767),
            ("工作目录", &self.working_directory, 32767),
            ("网址", &self.url, 8192),
        ] {
            if value.contains('\0') || value.chars().count() > limit {
                return Err(format!(
                    "「{}」的{label}包含空字符或超过长度限制",
                    self.name
                ));
            }
        }
        if !self.url.trim().is_empty() {
            normalized_url(&self.url)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherSettings {
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default)]
    pub start_minimized_to_tray: bool,
    #[serde(default = "default_true")]
    pub minimize_to_tray: bool,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    #[serde(default = "default_true")]
    pub confirm_before_stop_all: bool,
    #[serde(default = "default_interval")]
    pub auto_start_interval_ms: u32,
    #[serde(default, flatten)]
    pub extra: Extensions,
}

impl Default for LauncherSettings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            start_minimized_to_tray: false,
            minimize_to_tray: true,
            close_to_tray: true,
            confirm_before_stop_all: true,
            auto_start_interval_ms: default_interval(),
            extra: Extensions::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherConfig {
    #[serde(default = "legacy_version")]
    pub version: u32,
    #[serde(default)]
    pub items: Vec<LaunchItem>,
    #[serde(default)]
    pub settings: LauncherSettings,
    #[serde(default)]
    pub main_window_bounds: Option<WindowBounds>,
    #[serde(default)]
    pub settings_window_bounds: Option<WindowBounds>,
    #[serde(default, flatten)]
    pub extra: Extensions,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            items: Vec::new(),
            settings: LauncherSettings::default(),
            main_window_bounds: None,
            settings_window_bounds: None,
            extra: Extensions::new(),
        }
    }
}

impl LauncherConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=CONFIG_VERSION).contains(&self.version) {
            return Err(format!(
                "不支持配置版本 {}（支持 1 到 {CONFIG_VERSION}），原文件不会被覆盖",
                self.version
            ));
        }
        if self.items.len() > MAX_ITEMS {
            return Err(format!("启动项不能超过 {MAX_ITEMS} 个"));
        }
        if self.settings.auto_start_interval_ms > 10000 {
            return Err("自动启动间隔必须在 0 到 10000 毫秒之间".into());
        }
        let mut ids = HashSet::new();
        for item in &self.items {
            item.validate()?;
            if !ids.insert(item.id) {
                return Err(format!(
                    "「{}」使用了重复的启动项 ID：{}",
                    item.name, item.id
                ));
            }
        }
        for bounds in [self.main_window_bounds, self.settings_window_bounds]
            .into_iter()
            .flatten()
        {
            if bounds.width < 100
                || bounds.height < 100
                || bounds.width > 32768
                || bounds.height > 32768
            {
                return Err("保存的窗口尺寸无效".into());
            }
        }
        Ok(())
    }
}

pub fn normalized_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    let value = if input.contains("://") {
        input.to_owned()
    } else {
        format!("http://{input}")
    };
    let url = url::Url::parse(&value).map_err(|_| "请输入有效的 HTTP 或 HTTPS 网址".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("只允许打开 HTTP 或 HTTPS 网址".into());
    }
    Ok(url.into())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub item_id: Uuid,
    pub generation: u64,
    pub state: ProcessState,
    pub pid: Option<u32>,
    pub started_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub requested_stop: bool,
    pub error: Option<String>,
}

impl RuntimeStatus {
    pub fn stopped(id: Uuid) -> Self {
        Self {
            item_id: id,
            generation: 0,
            state: ProcessState::Stopped,
            pid: None,
            started_at: None,
            exit_code: None,
            requested_stop: false,
            error: None,
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
