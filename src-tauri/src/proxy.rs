use crate::{
    launch::find_program,
    model::{validate_proxy_url, validate_test_site, LaunchItem, LauncherSettings, ProxyMode},
};
use serde::Serialize;
use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

/// Variables every mainstream runtime consults before opening a socket.
/// Windows environment lookups are case-insensitive, so upper case alone works
/// for Node, Python, Go, curl and git.
const PROXY_VARIABLES: [&str; 4] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"];

/// The environment a child process should receive, plus the keys to remove
/// from the inherited block. Removal matters: a launcher started from a shell
/// that already exports `HTTP_PROXY` would otherwise leak it into services
/// that were explicitly marked as direct.
///
/// The bypass list is entirely driven by configuration. Nothing is injected
/// implicitly, so leaving it empty sends every host, loopback included,
/// through the proxy.
#[derive(Clone, Debug, Default)]
pub struct ProxyPlan {
    pub set: Vec<(String, String)>,
    pub unset: Vec<String>,
    pub note: Option<String>,
}

impl ProxyPlan {
    fn direct(note: Option<String>) -> Self {
        Self {
            set: Vec::new(),
            unset: PROXY_VARIABLES
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            note,
        }
    }
}

pub struct SystemProxy {
    pub url: String,
    pub bypass: String,
}

pub fn resolve(item: &LaunchItem, settings: &LauncherSettings) -> Result<ProxyPlan, String> {
    let (mode, url, no_proxy) = if item.proxy_mode == ProxyMode::Custom {
        (
            ProxyMode::Custom,
            item.proxy_url.as_str(),
            item.no_proxy.as_str(),
        )
    } else {
        let mode = match item.proxy_mode {
            ProxyMode::Inherit => settings.proxy.mode,
            other => other,
        };
        (
            mode,
            settings.proxy.url.as_str(),
            settings.proxy.no_proxy.as_str(),
        )
    };
    resolve_plan(mode, url, no_proxy)
}

/// Shared by service launch and the connection test so both paths agree on
/// which variables a child process would receive.
pub fn resolve_plan(mode: ProxyMode, url: &str, no_proxy: &str) -> Result<ProxyPlan, String> {
    let mode = if mode == ProxyMode::Inherit {
        ProxyMode::None
    } else {
        mode
    };
    let (proxy_url, source_bypass, source_label) = match mode {
        ProxyMode::None => return Ok(ProxyPlan::direct(Some("代理已禁用，连接将直接发出".into()))),
        ProxyMode::System => match system_proxy() {
            Ok(Some(proxy)) => (proxy.url, proxy.bypass, "Windows 系统代理"),
            Ok(None) => {
                return Ok(ProxyPlan::direct(Some(
                    "Windows 系统代理未启用，连接将直接发出".into(),
                )))
            }
            Err(error) => return Err(format!("读取 Windows 系统代理失败：{error}")),
        },
        ProxyMode::Custom => (validate_proxy_url(url)?, String::new(), "自定义代理"),
        // Normalized to None above.
        ProxyMode::Inherit => unreachable!(),
    };

    let mut bypass: Vec<String> = Vec::new();
    for source in [source_bypass.as_str(), no_proxy] {
        for part in source.split([',', ';']) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if !bypass
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(part))
            {
                bypass.push(part.to_owned());
            }
        }
    }

    let scheme = url::Url::parse(&proxy_url)
        .map(|parsed| parsed.scheme().to_owned())
        .unwrap_or_default();
    let mut plan = ProxyPlan::direct(None);
    let note = if matches!(scheme.as_str(), "socks5" | "socks5h") {
        // Most clients reject a socks scheme inside HTTP_PROXY instead of
        // falling back, so socks is offered through ALL_PROXY only.
        plan.set.push(("ALL_PROXY".to_owned(), proxy_url.clone()));
        format!("已注入{source_label} {proxy_url}（仅 ALL_PROXY，部分工具不支持 socks）")
    } else {
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            plan.set.push((name.to_owned(), proxy_url.clone()));
        }
        format!("已通过{source_label}注入 {proxy_url}")
    };
    plan.note = Some(note);
    if !bypass.is_empty() {
        plan.set.push(("NO_PROXY".to_owned(), bypass.join(",")));
    }
    // An empty list leaves `NO_PROXY` in `plan.unset`, so a value inherited
    // from the launcher's own terminal cannot silently reintroduce bypasses.
    Ok(plan)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTestResult {
    pub ok: bool,
    pub status: u16,
    pub elapsed_ms: u64,
    pub effective_proxy: Option<String>,
    pub message: String,
}

/// Uses the system `curl` rather than adding an HTTP stack to the binary. The
/// plan is applied as environment variables, so the test exercises exactly the
/// same variables a launched service would receive.
pub async fn test_site(site: &str, plan: &ProxyPlan) -> Result<ProxyTestResult, String> {
    let site = validate_test_site(site)?;
    let curl =
        curl_path().ok_or("未找到 curl，无法测试连接。Windows 10 1803 及以上版本自带 curl.exe")?;
    let started = Instant::now();
    let mut command = tokio::process::Command::new(curl);
    command
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--max-time")
        .arg("8")
        .arg("--output")
        .arg(if cfg!(windows) { "NUL" } else { "/dev/null" })
        .arg("--write-out")
        .arg("%{http_code}")
        .arg(&site)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // The launcher has no console of its own; without this flag Windows
    // allocates a fresh console window for curl on every test.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command
            .as_std_mut()
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    for name in &plan.unset {
        command.env_remove(name);
    }
    command.envs(plan.set.iter().map(|(key, value)| (key, value)));
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .map_err(|_| "测试超时：15 秒内没有收到响应".to_string())?
        .map_err(|error| format!("无法运行 curl：{error}"))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .unwrap_or(0);
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let ok = output.status.success() && status > 0;
    let message = if ok {
        format!("连接成功，HTTP {status}")
    } else if detail.is_empty() {
        format!("连接失败：{}", curl_hint(output.status.code()))
    } else {
        format!("连接失败：{detail}")
    };
    Ok(ProxyTestResult {
        ok,
        status,
        elapsed_ms,
        effective_proxy: plan
            .set
            .iter()
            .find(|(key, _)| key == "HTTP_PROXY" || key == "ALL_PROXY")
            .map(|(_, value)| value.clone()),
        message,
    })
}

fn curl_path() -> Option<PathBuf> {
    find_program(if cfg!(windows) { "curl.exe" } else { "curl" }).or_else(|| {
        std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join("System32/curl.exe"))
            .filter(|path| path.is_file())
    })
}

fn curl_hint(code: Option<i32>) -> String {
    match code {
        Some(5) => "无法解析代理地址".into(),
        Some(6) => "无法解析测试网站的主机名".into(),
        Some(7) => "无法连接到代理或目标服务器".into(),
        Some(28) => "连接超时".into(),
        Some(35) => "TLS 握手失败，代理可能不支持 HTTPS".into(),
        Some(56) => "接收数据失败，连接可能被中断".into(),
        Some(60) => "TLS 证书校验失败".into(),
        Some(code) => format!("curl 退出码 {code}"),
        None => "curl 被异常终止".into(),
    }
}

#[cfg(windows)]
pub fn system_proxy() -> Result<Option<SystemProxy>, String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY) {
        Ok(key) => key,
        // A missing key means Windows has never stored proxy settings, which is
        // equivalent to "direct" rather than a reason to refuse the launch.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    // A PAC script (AutoConfigURL) cannot be flattened into a static address,
    // so it is intentionally ignored rather than guessed at.
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    if enabled == 0 {
        return Ok(None);
    }
    let server: String = key
        .get_value("ProxyServer")
        .map_err(|error| format!("无法读取 ProxyServer：{error}"))?;
    if server.trim().is_empty() {
        return Ok(None);
    }
    let url = parse_proxy_server(&server)?;
    let bypass: String = key.get_value("ProxyOverride").unwrap_or_default();
    Ok(Some(SystemProxy {
        url,
        bypass: normalize_bypass(&bypass),
    }))
}

#[cfg(not(windows))]
pub fn system_proxy() -> Result<Option<SystemProxy>, String> {
    Err("仅 Windows 支持读取系统代理".into())
}

/// `ProxyServer` is either `host:port` or a protocol map such as
/// `http=h:p;https=h:p;ftp=h:p`.
fn parse_proxy_server(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let (scheme, value) = if raw.contains('=') {
        let mut http = None;
        let mut https = None;
        let mut socks = None;
        for entry in raw.split(';') {
            let Some((key, value)) = entry.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match key.trim().to_ascii_lowercase().as_str() {
                "http" => http = Some(value.to_owned()),
                // The `https=` entry names an HTTP proxy used for HTTPS
                // requests; its address is not an `https://` proxy itself.
                "https" => https = Some(value.to_owned()),
                "socks" | "socks5" => socks = Some(value.to_owned()),
                _ => {}
            }
        }
        if let Some(value) = http {
            ("http", value)
        } else if let Some(value) = https {
            ("http", value)
        } else if let Some(value) = socks {
            ("socks5", value)
        } else {
            return Err("Windows 系统代理配置中没有可用的地址".into());
        }
    } else {
        ("http", raw.to_owned())
    };
    let value = if value.contains("://") {
        value
    } else {
        format!("{scheme}://{value}")
    };
    validate_proxy_url(&value).map_err(|error| format!("Windows 系统代理地址无效：{error}"))
}

/// `ProxyOverride` uses semicolons and a `<local>` token that has no
/// equivalent in `NO_PROXY`.
fn normalize_bypass(raw: &str) -> String {
    raw.split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty() && !part.eq_ignore_ascii_case("<local>"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(mode: ProxyMode, url: &str, no_proxy: &str) -> LaunchItem {
        let mut item = LaunchItem::new("demo", "demo.exe");
        item.proxy_mode = mode;
        item.proxy_url = url.into();
        item.no_proxy = no_proxy.into();
        item
    }

    fn custom(url: &str) -> LauncherSettings {
        let mut settings = LauncherSettings::default();
        settings.proxy.mode = ProxyMode::Custom;
        settings.proxy.url = url.into();
        settings
    }

    fn value<'a>(plan: &'a ProxyPlan, key: &str) -> Option<&'a str> {
        plan.set
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn the_default_bypass_list_keeps_loopback_out_of_the_proxy() {
        let settings = custom("127.0.0.1:7890");
        assert_eq!(settings.proxy.no_proxy, crate::model::DEFAULT_NO_PROXY);
        let plan = resolve(&item(ProxyMode::Inherit, "", ""), &settings).unwrap();
        assert_eq!(value(&plan, "HTTP_PROXY"), Some("http://127.0.0.1:7890"));
        assert_eq!(value(&plan, "HTTPS_PROXY"), Some("http://127.0.0.1:7890"));
        let bypass = value(&plan, "NO_PROXY").unwrap();
        assert!(bypass.contains("localhost"), "{bypass}");
        assert!(bypass.contains("127.0.0.1"), "{bypass}");
        assert!(bypass.contains("::1"), "{bypass}");
    }

    #[test]
    fn an_empty_bypass_list_sends_everything_through_the_proxy() {
        let mut settings = custom("http://127.0.0.1:7890");
        settings.proxy.no_proxy.clear();
        let plan = resolve(&item(ProxyMode::Inherit, "", ""), &settings).unwrap();
        assert_eq!(value(&plan, "NO_PROXY"), None);
        assert!(plan.unset.iter().any(|name| name == "NO_PROXY"));
    }

    #[test]
    fn an_item_custom_address_wins_over_the_global_default() {
        let plan = resolve(
            &item(ProxyMode::Custom, "http://local:9090", ""),
            &custom("http://global:8080"),
        )
        .unwrap();
        assert_eq!(value(&plan, "HTTP_PROXY"), Some("http://local:9090"));
    }

    #[test]
    fn disabling_the_proxy_removes_inherited_variables() {
        let plan = resolve(
            &item(ProxyMode::None, "", ""),
            &custom("http://global:8080"),
        )
        .unwrap();
        assert!(plan.set.is_empty());
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"] {
            assert!(plan.unset.iter().any(|entry| entry == name), "{name}");
        }
    }

    #[test]
    fn socks_is_only_exposed_through_all_proxy() {
        let plan = resolve(
            &item(ProxyMode::Inherit, "", ""),
            &custom("socks5://user:pass@127.0.0.1:1080"),
        )
        .unwrap();
        assert_eq!(
            value(&plan, "ALL_PROXY"),
            Some("socks5://user:pass@127.0.0.1:1080")
        );
        assert_eq!(value(&plan, "HTTP_PROXY"), None);
        assert_eq!(value(&plan, "HTTPS_PROXY"), None);
    }

    #[test]
    fn bypass_entries_merge_without_duplicates() {
        let mut settings = custom("http://127.0.0.1:7890");
        settings.proxy.no_proxy = "localhost, .internal; 10.0.0.0/8".into();
        let plan = resolve(&item(ProxyMode::Inherit, "", ""), &settings).unwrap();
        let bypass = value(&plan, "NO_PROXY").unwrap();
        assert_eq!(bypass.matches("localhost").count(), 1, "{bypass}");
        assert!(bypass.contains(".internal"), "{bypass}");
        assert!(bypass.contains("10.0.0.0/8"), "{bypass}");
    }

    #[test]
    fn windows_proxy_server_forms_are_parsed() {
        assert_eq!(
            parse_proxy_server("127.0.0.1:7890").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            parse_proxy_server("http=127.0.0.1:7890;https=127.0.0.1:7891").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert_eq!(
            parse_proxy_server("socks=127.0.0.1:1080").unwrap(),
            "socks5://127.0.0.1:1080"
        );
        assert_eq!(
            normalize_bypass("localhost;127.*;<local>"),
            "localhost,127.*"
        );
    }

    #[test]
    fn invalid_proxy_addresses_are_rejected() {
        for value in [
            "",
            "ftp://127.0.0.1:21",
            "http://",
            "http://127.0.0.1:7890/path",
        ] {
            assert!(validate_proxy_url(value).is_err(), "accepted {value}");
        }
        assert_eq!(
            validate_proxy_url("127.0.0.1:7890").unwrap(),
            "http://127.0.0.1:7890"
        );
        assert!(validate_proxy_url("socks5://user:pass@127.0.0.1:1080").is_ok());
    }

    #[test]
    fn test_sites_default_to_https_and_reject_other_schemes() {
        assert_eq!(
            validate_test_site("www.google.com").unwrap(),
            "https://www.google.com/"
        );
        assert!(validate_test_site("ftp://example.com").is_err());
        assert!(validate_test_site("").is_err());
    }

    #[test]
    fn the_test_plan_matches_the_service_plan() {
        let plan = resolve_plan(ProxyMode::Custom, "127.0.0.1:7890", "").unwrap();
        assert_eq!(value(&plan, "HTTP_PROXY"), Some("http://127.0.0.1:7890"));
        assert_eq!(value(&plan, "HTTPS_PROXY"), Some("http://127.0.0.1:7890"));
    }

    #[tokio::test]
    async fn a_local_site_reports_status_and_timing() {
        use std::io::{Read, Write};
        if curl_path().is_none() {
            return;
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut buffer = [0u8; 1024];
                let _ = socket.read(&mut buffer);
                let _ = socket.write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let plan = resolve_plan(ProxyMode::None, "", "").unwrap();
        let result = test_site(&format!("http://127.0.0.1:{port}/"), &plan)
            .await
            .unwrap();
        server.join().unwrap();
        assert!(result.ok, "{}", result.message);
        assert_eq!(result.status, 204);
        assert!(result.effective_proxy.is_none());
    }
}
