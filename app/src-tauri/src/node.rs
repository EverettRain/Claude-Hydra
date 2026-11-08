//! Per-account proxy node: configuration, credential sealing, exit/region detection.
//!
//! Claude never sees the node itself. It talks to a local relay (see `relay.rs`) that
//! forwards every connection to the node with its credentials, so the password never
//! appears on a command line or in an environment variable.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io::Read, net::IpAddr, time::Duration};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig {
    /// "http" | "https" | "socks5"
    pub kind: String,
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    /// Plain text in memory and over IPC; sealed in `settings.json` (see `seal`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
    /// Exit IP version to require when checking: "ipv4" | "ipv6".
    #[serde(default)]
    pub ip_version: String,
    /// IP lookup service: "ipinfo" | "ipapi" | "ip-api".
    #[serde(default)]
    pub geo_channel: String,
    /// Provider URL that rotates the node's exit IP (called directly, not through the node).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub refresh_url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExitInfo {
    pub ip: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub timezone: String,
    #[serde(default)]
    pub checked_at: u64,
}

impl NodeConfig {
    /// Normalises and validates user input. Errors are user-facing.
    pub fn validated(&self) -> Result<NodeConfig, String> {
        let mut n = self.clone();
        n.kind = n.kind.trim().to_ascii_lowercase();
        n.host = n.host.trim().trim_start_matches('[').trim_end_matches(']').to_string();
        n.username = n.username.trim().to_string();
        n.refresh_url = n.refresh_url.trim().to_string();
        if !matches!(n.kind.as_str(), "http" | "https" | "socks5") { return Err("代理类型只支持 HTTP、HTTPS 或 SOCKS5。".into()); }
        if n.host.is_empty() || n.host.len() > 253 || !n.host.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c)) {
            return Err("代理主机无效。".into());
        }
        if n.port == 0 { return Err("代理端口无效。".into()); }
        if n.username.len() > 200 || n.password.len() > 200 { return Err("代理账号或密码过长（最多 200 个字符）。".into()); }
        if n.username.is_empty() && !n.password.is_empty() { return Err("填写了代理密码时也需要填写代理账号。".into()); }
        if n.username.chars().chain(n.password.chars()).any(char::is_control) { return Err("代理账号或密码包含无效字符。".into()); }
        if n.kind == "socks5" && (n.username.len() > 255 || n.password.len() > 255) { return Err("SOCKS5 账号或密码过长。".into()); }
        if !matches!(n.ip_version.as_str(), "ipv4" | "ipv6") { n.ip_version = "ipv4".into(); }
        if !matches!(n.geo_channel.as_str(), "ipinfo" | "ipapi" | "ip-api") { n.geo_channel = "ipinfo".into(); }
        if !n.refresh_url.is_empty() {
            let u = url::Url::parse(&n.refresh_url).map_err(|_| "刷新 URL 无效。")?;
            if !matches!(u.scheme(), "http" | "https") { return Err("刷新 URL 只支持 http 或 https。".into()); }
        }
        Ok(n)
    }

    /// Parses a legacy `scheme://host:port` endpoint saved by earlier versions.
    pub fn from_legacy(endpoint: &str) -> Option<NodeConfig> {
        let u = url::Url::parse(endpoint.trim()).ok()?;
        let kind = match u.scheme() { "socks5" | "socks5h" => "socks5", s @ ("http" | "https") => s, _ => return None };
        Some(NodeConfig { kind: kind.into(), host: u.host_str()?.trim_matches(['[', ']']).into(), port: u.port_or_known_default()?, ..Default::default() }
            .validated().ok()?)
    }

    /// Upstream URL for reqwest, with percent-encoded credentials and remote DNS for SOCKS.
    fn reqwest_proxy(&self) -> Result<reqwest::Proxy, String> {
        let scheme = if self.kind == "socks5" { "socks5h" } else { &self.kind };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let mut u = url::Url::parse(&format!("{scheme}://{host}:{}", self.port)).map_err(|_| "代理地址无效。")?;
        if !self.username.is_empty() {
            u.set_username(&self.username).map_err(|_| "代理账号无效。")?;
            u.set_password(Some(&self.password)).map_err(|_| "代理密码无效。")?;
        }
        reqwest::Proxy::all(u.as_str()).map_err(|_| "代理地址无效。".into())
    }

    /// A client that reaches the internet only through this node (never directly).
    pub fn client(&self, timeout: Duration) -> Result<reqwest::blocking::Client, String> {
        reqwest::blocking::Client::builder()
            .no_proxy()
            .proxy(self.reqwest_proxy()?)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .map_err(|_| "无法初始化代理连接。".into())
    }
}

/// A client whose only route is the given local relay port.
pub fn relay_client(port: u16, timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    let proxy = reqwest::Proxy::all(format!("socks5h://127.0.0.1:{port}")).map_err(|_| "本地中继地址无效。")?;
    reqwest::blocking::Client::builder().no_proxy().proxy(proxy).redirect(reqwest::redirect::Policy::none())
        .timeout(timeout).build().map_err(|_| "无法初始化本地中继连接。".into())
}

fn get_text(client: &reqwest::blocking::Client, url: &str, limit: u64) -> Result<String, String> {
    let mut text = String::new();
    client.get(url).header("accept", "application/json, text/plain").send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| if e.is_timeout() { "通过代理节点访问检测服务超时。".to_string() } else { "通过代理节点访问检测服务失败，请检查节点地址、账号和密码。".to_string() })?
        .take(limit).read_to_string(&mut text).map_err(|_| "无法读取检测结果。")?;
    Ok(text)
}

pub fn valid_timezone(tz: &str) -> bool {
    let ok = !tz.is_empty() && tz.len() <= 64 && tz.contains('/') || tz == "UTC";
    ok && tz.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c)) && !tz.contains("..")
        && (cfg!(windows) || !std::path::Path::new("/usr/share/zoneinfo").is_dir() || std::path::Path::new("/usr/share/zoneinfo").join(tz).is_file())
}

fn parse_geo(channel: &str, body: &str) -> Result<ExitInfo, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "IP 查询服务返回了无法识别的数据。")?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let info = match channel {
        "ip-api" => {
            if s("status") != "success" { return Err(format!("IP 查询失败：{}", s("message"))); }
            ExitInfo { ip: s("query"), country: s("countryCode"), region: s("regionName"), city: s("city"), timezone: s("timezone"), checked_at: 0 }
        }
        "ipapi" => {
            if v.get("error").and_then(Value::as_bool) == Some(true) { return Err(format!("IP 查询失败：{}", s("reason"))); }
            ExitInfo { ip: s("ip"), country: s("country_code"), region: s("region"), city: s("city"), timezone: s("timezone"), checked_at: 0 }
        }
        _ => ExitInfo { ip: s("ip"), country: s("country"), region: s("region"), city: s("city"), timezone: s("timezone"), checked_at: 0 },
    };
    if info.ip.parse::<IpAddr>().is_err() { return Err("IP 查询服务未返回有效的出口 IP。".into()); }
    Ok(info)
}

/// Checks a route end to end: exit IP of the required version, then its region and timezone.
pub fn inspect(client: &reqwest::blocking::Client, node: &NodeConfig) -> Result<ExitInfo, String> {
    let ip_url = if node.ip_version == "ipv6" { "https://api6.ipify.org" } else { "https://api4.ipify.org" };
    let ip: IpAddr = get_text(client, ip_url, 64)
        .map_err(|e| if node.ip_version == "ipv6" { format!("{e}（节点可能不支持 IPv6 出口）") } else { e })?
        .trim().parse().map_err(|_| "检测服务未返回有效的出口 IP。")?;
    let geo_url = match node.geo_channel.as_str() {
        "ipapi" => "https://ipapi.co/json/",
        "ip-api" => "http://ip-api.com/json/?fields=status,message,query,countryCode,regionName,city,timezone",
        _ => "https://ipinfo.io/json",
    };
    let mut info = parse_geo(&node.geo_channel, &get_text(client, geo_url, 16 * 1024)?)?;
    // The geo service may see the other IP family; the version-checked address is authoritative.
    info.ip = ip.to_string();
    if !valid_timezone(&info.timezone) { info.timezone.clear(); }
    info.checked_at = crate::paths::now_millis();
    Ok(info)
}

/// Calls the provider's IP rotation URL directly and returns a short status line.
pub fn refresh(url: &str) -> Result<String, String> {
    let u = url::Url::parse(url.trim()).map_err(|_| "刷新 URL 无效。")?;
    if !matches!(u.scheme(), "http" | "https") { return Err("刷新 URL 只支持 http 或 https。".into()); }
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).build().map_err(|_| "无法初始化网络连接。")?;
    let resp = client.get(u).send().map_err(|_| "无法访问刷新 URL。")?;
    let status = resp.status();
    let mut body = String::new();
    let _ = resp.take(300).read_to_string(&mut body);
    let body: String = body.chars().filter(|c| !c.is_control()).take(120).collect();
    if status.is_success() { Ok(format!("已请求刷新（HTTP {}）{}", status.as_u16(), if body.is_empty() { String::new() } else { format!("：{body}") })) }
    else { Err(format!("刷新失败（HTTP {}）{}", status.as_u16(), if body.is_empty() { String::new() } else { format!("：{body}") })) }
}

// ---- credential sealing -------------------------------------------------------------

/// Seals a secret for `settings.json`: DPAPI (current Windows user) on Windows. Elsewhere the
/// settings file lives in a per-user directory and is written with owner-only permissions.
pub fn seal(secret: &str) -> String {
    if secret.is_empty() { return String::new(); }
    #[cfg(windows)]
    if let Some(sealed) = win::protect(secret.as_bytes()) {
        use base64::{engine::general_purpose::STANDARD, Engine};
        return format!("dpapi:{}", STANDARD.encode(sealed));
    }
    format!("plain:{secret}")
}

pub fn unseal(stored: &str) -> String {
    if let Some(rest) = stored.strip_prefix("plain:") { return rest.to_string(); }
    #[cfg(windows)]
    if let Some(rest) = stored.strip_prefix("dpapi:") {
        use base64::{engine::general_purpose::STANDARD, Engine};
        return STANDARD.decode(rest).ok().and_then(|b| win::unprotect(&b)).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
    }
    if stored.starts_with("dpapi:") { return String::new(); }
    stored.to_string()
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    #[repr(C)] struct Blob { len: u32, data: *mut u8 }
    #[link(name = "crypt32")] extern "system" {
        fn CryptProtectData(input: *const Blob, description: *const u16, entropy: *const Blob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32;
        fn CryptUnprotectData(input: *const Blob, description: *mut *mut u16, entropy: *const Blob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32;
    }
    #[link(name = "kernel32")] extern "system" { fn LocalFree(mem: *mut c_void) -> *mut c_void; }
    const UI_FORBIDDEN: u32 = 1;
    fn run(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = Blob { len: data.len() as u32, data: data.as_ptr() as *mut u8 };
        let mut out = Blob { len: 0, data: std::ptr::null_mut() };
        let ok = unsafe {
            if protect { CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), UI_FORBIDDEN, &mut out) }
            else { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), UI_FORBIDDEN, &mut out) }
        };
        if ok == 0 { return None; }
        unsafe { let v = std::slice::from_raw_parts(out.data, out.len as usize).to_vec(); LocalFree(out.data as *mut c_void); Some(v) }
    }
    pub fn protect(data: &[u8]) -> Option<Vec<u8>> { run(data, true) }
    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> { run(data, false) }
}

// ---- launch arguments ---------------------------------------------------------------

/// Chromium switches that would override or bypass the managed proxy.
const PROXY_SWITCHES: [&str; 6] = ["--proxy-server", "--no-proxy-server", "--proxy-pac-url", "--proxy-auto-detect", "--proxy-bypass-list", "--host-resolver-rules"];

/// Whether extra launch arguments already contain a proxy switch (exact flag match only).
pub fn args_conflict(args: &str) -> bool {
    args.split_whitespace()
        .map(|a| a.trim_matches('"').split('=').next().unwrap_or_default().to_ascii_lowercase())
        .any(|flag| PROXY_SWITCHES.contains(&flag.as_str()))
}

/// Chromium switches that route everything through the local relay and close the usual leaks:
/// no local DNS (all names resolve at the node), no QUIC/UDP, WebRTC limited to proxied paths.
pub fn chromium_args(relay_port: u16) -> Vec<String> {
    vec![
        format!("--proxy-server=socks5://127.0.0.1:{relay_port}"),
        "--host-resolver-rules=MAP * ~NOTFOUND , EXCLUDE 127.0.0.1 , EXCLUDE localhost".into(),
        "--disable-quic".into(),
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
        "--webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
        "--enforce-webrtc-ip-permission-check".into(),
    ]
}

/// Proxy environment for Claude's child processes (Claude Code, MCP servers, Node tools).
pub fn child_env(relay_port: u16) -> Vec<(String, String)> {
    let http = format!("http://127.0.0.1:{relay_port}");
    let socks = format!("socks5h://127.0.0.1:{relay_port}");
    let mut env = Vec::new();
    for k in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] { env.push((k.to_string(), http.clone())); }
    for k in ["ALL_PROXY", "all_proxy"] { env.push((k.to_string(), socks.clone())); }
    for k in ["NO_PROXY", "no_proxy"] { env.push((k.to_string(), "localhost,127.0.0.1,::1".to_string())); }
    // Node 24+ only honours proxy variables for fetch/http when asked to.
    env.push(("NODE_USE_ENV_PROXY".into(), "1".into()));
    env
}

pub fn is_proxy_env(key: &str) -> bool {
    ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY", "NODE_USE_ENV_PROXY"].iter().any(|k| k.eq_ignore_ascii_case(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: &str) -> NodeConfig { NodeConfig { kind: kind.into(), host: "proxy.example".into(), port: 1080, ..Default::default() } }

    #[test]
    fn validates_and_normalises() {
        let n = NodeConfig { kind: " SOCKS5 ".into(), host: "[::1]".into(), ..node("x") }.validated().unwrap();
        assert_eq!((n.kind.as_str(), n.host.as_str(), n.ip_version.as_str(), n.geo_channel.as_str()), ("socks5", "::1", "ipv4", "ipinfo"));
        assert!(NodeConfig { port: 0, ..node("http") }.validated().is_err());
        assert!(NodeConfig { host: "a b".into(), ..node("http") }.validated().is_err());
        assert!(NodeConfig { password: "p".into(), ..node("http") }.validated().is_err());
        assert!(node("ftp").validated().is_err());
        assert!(NodeConfig { refresh_url: "file:///x".into(), ..node("http") }.validated().is_err());
    }

    #[test]
    fn legacy_endpoint_becomes_node() {
        let n = NodeConfig::from_legacy("socks5://127.0.0.1:7890").unwrap();
        assert_eq!((n.kind.as_str(), n.host.as_str(), n.port), ("socks5", "127.0.0.1", 7890));
        assert!(NodeConfig::from_legacy("nonsense").is_none());
    }

    #[test]
    fn credentials_are_percent_encoded() {
        let n = NodeConfig { username: "u@x".into(), password: "p:w/#".into(), ..node("socks5") };
        assert!(n.reqwest_proxy().is_ok());
    }

    #[test]
    fn seal_round_trip() {
        assert_eq!(unseal(&seal("s3cr:et")), "s3cr:et");
        assert_eq!(seal(""), "");
        assert_eq!(unseal("legacy"), "legacy");
    }

    #[test]
    fn only_real_proxy_switches_conflict() {
        for args in ["--proxy-server=http://a:1", "--lang=en \"--proxy-pac-url=x\"", "--NO-PROXY-SERVER", "--proxy-bypass-list=*", "--host-resolver-rules=x"] {
            assert!(args_conflict(args), "{args}");
        }
        for args in ["", "--lang=zh-CN", "--no-proxy-xxx", "--proxy-serverless", "--disable-gpu --my-proxy=1"] {
            assert!(!args_conflict(args), "{args}");
        }
    }

    #[test]
    fn parses_each_geo_channel() {
        let a = parse_geo("ipinfo", r#"{"ip":"192.0.2.1","country":"JP","region":"Tokyo","city":"Tokyo","timezone":"Asia/Tokyo"}"#).unwrap();
        assert_eq!((a.country.as_str(), a.timezone.as_str()), ("JP", "Asia/Tokyo"));
        let b = parse_geo("ipapi", r#"{"ip":"2001:db8::1","country_code":"US","timezone":"America/New_York"}"#).unwrap();
        assert_eq!(b.country, "US");
        let c = parse_geo("ip-api", r#"{"status":"success","query":"192.0.2.9","countryCode":"DE","timezone":"Europe/Berlin"}"#).unwrap();
        assert_eq!(c.timezone, "Europe/Berlin");
        assert!(parse_geo("ip-api", r#"{"status":"fail","message":"private range"}"#).is_err());
        assert!(parse_geo("ipinfo", r#"{"ip":"nope"}"#).is_err());
    }

    #[test]
    fn timezone_names_are_checked() {
        assert!(valid_timezone("Asia/Tokyo"));
        assert!(valid_timezone("UTC"));
        assert!(!valid_timezone("../etc/passwd"));
        assert!(!valid_timezone("Asia/Tokyo; rm"));
        assert!(!valid_timezone(""));
    }
}
