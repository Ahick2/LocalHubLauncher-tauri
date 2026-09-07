use crate::model::{LaunchItem, LaunchType};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{
    env,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// cmd.exe has its own quoting grammar. All other programs receive argv.
    pub raw_args: Option<String>,
    pub directory: PathBuf,
    pub environment: Vec<(String, String)>,
    pub hide_window: bool,
}

pub fn build_spec(item: &LaunchItem, application_directory: &Path) -> Result<LaunchSpec, String> {
    item.validate()?;
    let kind = resolve_type(item.launch_type, &item.target, application_directory);
    let is_script = matches!(
        kind,
        LaunchType::Executable | LaunchType::Batch | LaunchType::PowerShell | LaunchType::Python
    );
    let file = if is_script {
        Some(resolve_target(&item.target, application_directory, kind)?)
    } else {
        None
    };
    let directory = if !item.working_directory.trim().is_empty() {
        resolve_path(&item.working_directory, application_directory)
    } else {
        file.as_ref()
            .and_then(|path| path.parent())
            .unwrap_or(application_directory)
            .to_path_buf()
    };
    if !directory.is_dir() {
        return Err(format!("工作目录不存在：{}", directory.display()));
    }
    let mut spec = LaunchSpec {
        program: PathBuf::new(),
        args: Vec::new(),
        raw_args: None,
        directory,
        environment: Vec::new(),
        hide_window: item.hide_window,
    };
    // Redirected Python is otherwise block-buffered even with UTF-8 enabled.
    if item.hide_window {
        spec.environment.extend([
            ("PYTHONIOENCODING".into(), "utf-8".into()),
            ("PYTHONUTF8".into(), "1".into()),
            ("PYTHONUNBUFFERED".into(), "1".into()),
        ]);
    }
    match kind {
        LaunchType::Executable => {
            spec.program = file.unwrap();
            spec.args = split_windows_arguments(&item.arguments)?;
        }
        LaunchType::Batch => {
            spec.program = cmd_path()?;
            spec.environment.push((
                "LOCALHUB_BATCH_TARGET".into(),
                file.unwrap().to_string_lossy().into(),
            ));
            // No CALL: CALL would expand percent characters in the path twice.
            // Delayed expansion is explicitly disabled so '!' also stays literal.
            spec.raw_args = Some(cmd_arguments(&format!(
                "chcp 65001>nul & \"%LOCALHUB_BATCH_TARGET%\" {}",
                item.arguments
            )));
        }
        LaunchType::PowerShell => {
            spec.program = find_pwsh()
                .or_else(find_windows_powershell)
                .ok_or("未找到 PowerShell，请安装 PowerShell 或检查 PATH")?;
            spec.args = vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-OutputFormat".into(),
                "Text".into(),
                "-File".into(),
                file.unwrap().to_string_lossy().into(),
            ];
            spec.args.extend(split_windows_arguments(&item.arguments)?);
        }
        LaunchType::Python => {
            spec.program = find_program(if cfg!(windows) {
                "python.exe"
            } else {
                "python3"
            })
            .ok_or("未找到 Python。可将虚拟环境中的 python.exe 设为启动文件，把脚本放在参数中")?;
            spec.args = vec!["-u".into(), file.unwrap().to_string_lossy().into()];
            spec.args.extend(split_windows_arguments(&item.arguments)?);
        }
        LaunchType::Command | LaunchType::Auto => {
            spec.program = cmd_path()?;
            spec.raw_args = Some(cmd_arguments(&format!(
                "chcp 65001>nul & {}",
                command_text(item)
            )));
        }
        LaunchType::Pwsh | LaunchType::WindowsPowerShell => {
            spec.program = if kind == LaunchType::Pwsh {
                find_pwsh()
            } else {
                find_windows_powershell()
            }
            .ok_or(if kind == LaunchType::Pwsh {
                "未找到 PowerShell 7 (pwsh)，请安装后重试"
            } else {
                "未找到 Windows PowerShell"
            })?;
            spec.args = vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-OutputFormat".into(),
                "Text".into(),
                "-Command".into(),
                powershell_command(&command_text(item)),
            ];
        }
        LaunchType::Wsl => {
            spec.program = find_program("wsl.exe").ok_or("未找到 WSL，请先安装并初始化发行版")?;
            spec.args = vec![
                "--exec".into(),
                "bash".into(),
                "-lc".into(),
                command_text(item),
            ];
        }
        LaunchType::GitBash => {
            spec.program = find_git_bash().ok_or("未找到 Git Bash，请安装 Git for Windows")?;
            spec.args = vec!["-lc".into(), command_text(item)];
        }
    }
    Ok(spec)
}

fn command_text(item: &LaunchItem) -> String {
    let command = item.target.trim();
    if item.arguments.trim().is_empty() {
        command.to_owned()
    } else {
        format!("{command} {}", item.arguments.trim())
    }
}

pub fn resolve_type(kind: LaunchType, target: &str, application_directory: &Path) -> LaunchType {
    if kind != LaunchType::Auto {
        return kind;
    }
    let input = target.trim();
    let unquoted = unquote_path(input);
    let path = resolve_path(unquoted, application_directory);
    // An extension in the final argument is not the extension of a command.
    // Existing files (including paths with spaces) are recognized first.
    if !path.is_file() && input.chars().any(char::is_whitespace) && unquoted == input {
        return LaunchType::Command;
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "bat" | "cmd" => LaunchType::Batch,
        "ps1" => LaunchType::PowerShell,
        "py" | "pyw" => LaunchType::Python,
        "exe" | "com" => LaunchType::Executable,
        _ if path.is_file() => LaunchType::Executable,
        _ => LaunchType::Command,
    }
}

fn resolve_target(
    target: &str,
    application_directory: &Path,
    kind: LaunchType,
) -> Result<PathBuf, String> {
    let target = expand_path_variables(unquote_path(target.trim()));
    let path = resolve_path(&target, application_directory);
    if path.is_file() {
        return Ok(path);
    }
    if kind == LaunchType::Executable && !target.contains(['/', '\\']) {
        if let Some(path) = find_program(&target) {
            return Ok(path);
        }
    }
    Err(format!(
        "启动文件不存在：{}。若填写的是完整命令，请选择单命令启动方式",
        path.display()
    ))
}

pub fn resolve_path(path: &str, application_directory: &Path) -> PathBuf {
    let expanded = expand_path_variables(unquote_path(path.trim()));
    let path = PathBuf::from(expanded);
    if path.is_absolute() {
        path
    } else {
        application_directory.join(path)
    }
}

fn unquote_path(input: &str) -> &str {
    if input.len() >= 2
        && input.starts_with('"')
        && input.ends_with('"')
        && !input[1..input.len() - 1].contains('"')
    {
        &input[1..input.len() - 1]
    } else {
        input
    }
}

/// Expand Windows path variables only in path fields, never in shell code.
fn expand_path_variables(input: &str) -> String {
    let mut output = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        output.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let Some(end) = tail.find('%') else {
            output.push_str(&rest[start..]);
            return output;
        };
        let variable = &tail[..end];
        match env::var(variable) {
            Ok(value) => output.push_str(&value),
            Err(_) => {
                output.push('%');
                output.push_str(variable);
                output.push('%');
            }
        }
        rest = &tail[end + 1..];
    }
    output.push_str(rest);
    output
}

fn cmd_arguments(command: &str) -> String {
    format!("/d /v:off /s /c \"{command}\"")
}

fn encoded_powershell(command: &str) -> String {
    let script = format!(
        "$OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); {command}"
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    STANDARD.encode(bytes)
}

fn powershell_command(command: &str) -> String {
    // -EncodedCommand makes Windows PowerShell serialize stderr as CLIXML,
    // even with -OutputFormat Text. A constant decoder passed to -Command keeps
    // normal text streams while base64 still protects every user-supplied quote.
    let encoded = encoded_powershell(command);
    format!("& ([ScriptBlock]::Create([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{encoded}'))))")
}

fn cmd_path() -> Result<PathBuf, String> {
    env::var_os("ComSpec")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| find_program("cmd.exe"))
        .ok_or_else(|| "cmd 启动方式仅适用于 Windows，且需要 cmd.exe".into())
}

pub fn find_program(program: &str) -> Option<PathBuf> {
    let program_path = PathBuf::from(program);
    if program_path.is_absolute() && program_path.is_file() {
        return Some(program_path);
    }
    let mut names = vec![program.to_owned()];
    if cfg!(windows) && program_path.extension().is_none() {
        names.extend(["exe", "com"].map(|ext| format!("{program}.{ext}")));
    }
    for directory in env::split_paths(&env::var_os("PATH").unwrap_or_default()) {
        let directory = PathBuf::from(directory.to_string_lossy().trim_matches('"'));
        for name in &names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn find_pwsh() -> Option<PathBuf> {
    find_program(if cfg!(windows) { "pwsh.exe" } else { "pwsh" }).or_else(|| {
        ["ProgramW6432", "ProgramFiles"]
            .into_iter()
            .filter_map(env::var_os)
            .map(|base| PathBuf::from(base).join("PowerShell/7/pwsh.exe"))
            .find(|path| path.is_file())
    })
}

fn find_windows_powershell() -> Option<PathBuf> {
    find_program("powershell.exe").or_else(|| {
        env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"))
            .filter(|path| path.is_file())
    })
}

fn find_git_bash() -> Option<PathBuf> {
    // Probe Git installations before PATH, which can contain the obsolete WSL bash.exe.
    ["ProgramW6432", "ProgramFiles"]
        .into_iter()
        .filter_map(env::var_os)
        .flat_map(|base| {
            [
                PathBuf::from(&base).join("Git/bin/bash.exe"),
                PathBuf::from(base).join("Git/usr/bin/bash.exe"),
            ]
        })
        .find(|path| path.is_file())
        .or_else(|| {
            find_program("git.exe")
                .and_then(|git| {
                    git.parent()
                        .and_then(Path::parent)
                        .map(|base| base.join("bin/bash.exe"))
                })
                .filter(|path| path.is_file())
        })
        .or_else(|| find_program(if cfg!(windows) { "bash.exe" } else { "bash" }))
}

/// Windows argv grammar: whitespace outside quotes separates arguments;
/// backslashes are special only immediately before a double quote.
pub fn split_windows_arguments(input: &str) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    let mut argument = String::new();
    let mut chars = input.chars().peekable();
    let mut quoted = false;
    let mut started = false;
    while let Some(character) = chars.next() {
        match character {
            '\\' => {
                let mut count = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    count += 1;
                }
                if chars.peek() == Some(&'"') {
                    argument.extend(std::iter::repeat_n('\\', count / 2));
                    chars.next();
                    if count % 2 == 1 {
                        argument.push('"');
                    } else {
                        quoted = !quoted;
                    }
                } else {
                    argument.extend(std::iter::repeat_n('\\', count));
                }
                started = true;
            }
            '"' => {
                if quoted && chars.peek() == Some(&'"') {
                    chars.next();
                    argument.push('"');
                } else {
                    quoted = !quoted;
                }
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    result.push(std::mem::take(&mut argument));
                    started = false;
                }
            }
            c => {
                argument.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return Err("启动参数中的双引号没有配对".into());
    }
    if started {
        result.push(argument);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_infer_script_type_from_a_command_argument() {
        let dir = tempfile::tempdir().unwrap();
        for command in [
            "python server.py",
            "powershell -File start.ps1",
            "cmd /c start.cmd",
            "node server.js",
        ] {
            assert_eq!(
                resolve_type(LaunchType::Auto, command, dir.path()),
                LaunchType::Command
            );
        }
        assert_eq!(
            resolve_type(LaunchType::Auto, "server.py", dir.path()),
            LaunchType::Python
        );
    }

    #[test]
    fn existing_paths_with_spaces_remain_file_launches() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my service.py"), "").unwrap();
        assert_eq!(
            resolve_type(LaunchType::Auto, "my service.py", dir.path()),
            LaunchType::Python
        );
    }

    #[test]
    fn powershell_encoding_preserves_quotes_unicode_and_shell_operators() {
        let command = r#"Write-Output ("A" + "B"); Write-Output "中文 $env:TEMP"; 'one two'"#;
        let bytes = STANDARD.decode(encoded_powershell(command)).unwrap();
        let utf16: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let restored = String::from_utf16(&utf16).unwrap();
        assert!(restored.ends_with(command));
    }

    #[test]
    fn argv_preserves_spaces_empty_arguments_backslashes_and_literal_quotes() {
        assert_eq!(
            split_windows_arguments(r#"--name "alpha beta" "" C:\data\file \"hello\""#).unwrap(),
            ["--name", "alpha beta", "", r"C:\data\file", "\"hello\""]
        );
        assert_eq!(
            split_windows_arguments(r#""C:\data\\""#).unwrap(),
            [r"C:\data\"]
        );
        assert!(split_windows_arguments("\"unclosed").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn actually_executes_powershell_double_quotes_correctly() {
        use std::os::windows::process::CommandExt;
        let directory = tempfile::tempdir().unwrap();
        let mut item = LaunchItem::new(
            "quotes",
            r#"Write-Output ("A" + "B"); Write-Output "alpha beta""#,
        );
        item.launch_type = if find_pwsh().is_some() {
            LaunchType::Pwsh
        } else {
            LaunchType::WindowsPowerShell
        };
        let spec = build_spec(&item, directory.path()).unwrap();
        let output = std::process::Command::new(spec.program)
            .args(spec.args)
            .current_dir(spec.directory)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .replace("\r\n", "\n")
                .trim(),
            "AB\nalpha beta"
        );
    }

    #[cfg(windows)]
    #[test]
    fn powershell_errors_are_readable_text_instead_of_clixml() {
        use std::os::windows::process::CommandExt;
        let directory = tempfile::tempdir().unwrap();
        let mut item = LaunchItem::new(
            "errors",
            "Write-Error 'failure 中文'; Write-Output 'still running'",
        );
        item.launch_type = LaunchType::WindowsPowerShell;
        let spec = build_spec(&item, directory.path()).unwrap();
        let output = std::process::Command::new(spec.program)
            .args(spec.args)
            .envs(spec.environment)
            .current_dir(spec.directory)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("failure 中文"), "{error}");
        assert!(
            !error.contains("CLIXML") && !error.contains("<Objs"),
            "{error}"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "still running"
        );
    }
}
