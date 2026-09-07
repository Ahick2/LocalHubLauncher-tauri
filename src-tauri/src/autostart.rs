#[cfg(windows)]
mod platform {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_SET_VALUE},
        RegKey,
    };
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "LocalHubLauncher";

    pub fn read() -> Result<Option<String>, String> {
        let current = RegKey::predef(HKEY_CURRENT_USER);
        match current.open_subkey(KEY) {
            Ok(key) => match key.get_value(NAME) {
                Ok(value) => Ok(Some(value)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.to_string()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn write(value: Option<&str>) -> Result<(), String> {
        let current = RegKey::predef(HKEY_CURRENT_USER);
        if let Some(value) = value {
            let (key, _) = current.create_subkey(KEY).map_err(|e| e.to_string())?;
            key.set_value(NAME, &value).map_err(|e| e.to_string())
        } else {
            match current.open_subkey_with_flags(KEY, KEY_SET_VALUE) {
                Ok(key) => match key.delete_value(NAME) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error.to_string()),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.to_string()),
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn read() -> Result<Option<String>, String> {
        Ok(None)
    }
    pub fn write(_value: Option<&str>) -> Result<(), String> {
        Err("当前仅支持 Windows 开机自启动".into())
    }
}

pub use platform::{read, write};

pub fn registered_command() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(format!("\"{}\" --autostart", path.display()))
}

pub fn enabled() -> bool {
    match (read(), registered_command()) {
        (Ok(Some(value)), Ok(command)) => value.eq_ignore_ascii_case(&command),
        _ => false,
    }
}
