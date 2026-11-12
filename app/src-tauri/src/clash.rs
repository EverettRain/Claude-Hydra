use crate::{node::NodeConfig, paths};
use serde_json::{json, Value};
use std::{io::{Read, Write}, net::{TcpListener, TcpStream}, process::{Child, Command, Stdio}, time::{Duration, Instant}};

pub struct Core(Child, std::path::PathBuf);
impl Drop for Core {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); let _ = std::fs::remove_dir_all(&self.1); }
}
impl Core {
    pub fn alive(&mut self) -> bool { matches!(self.0.try_wait(), Ok(None)) }
}

fn run_root() -> std::path::PathBuf { paths::storage_root().join("mihomo-run") }

/// Host start-up: removes run directories left by cores of a Hydra that crashed or was
/// killed (a clean exit removes its own). Only the host owns long-lived cores, so anything
/// older than a few minutes belongs to nobody.
pub fn sweep_stale_runs() {
    let Ok(entries) = std::fs::read_dir(run_root()) else { return };
    for dir in entries.flatten() {
        let old = dir.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(300));
        if old { let _ = std::fs::remove_dir_all(dir.path()); }
    }
}

/// One hop as a Mihomo proxy entry. Clash nodes carry theirs; custom proxies are converted.
fn hop(node: &NodeConfig, name: &str, dialer: Option<&str>) -> Result<Value, String> {
    let mut proxy = if node.kind == "clash" {
        serde_json::from_str::<Value>(&node.password).ok().filter(Value::is_object).ok_or("节点配置无效。")?
    } else {
        let mut p = json!({"type": if node.kind == "socks5" { "socks5" } else { "http" }, "server": node.host, "port": node.port, "udp": false});
        if node.kind == "https" { p["tls"] = json!(true); }
        if !node.username.is_empty() { p["username"] = json!(node.username); p["password"] = json!(node.password); }
        p
    };
    proxy["name"] = json!(name);
    match dialer {
        Some(d) => proxy["dialer-proxy"] = json!(d),
        // A subscription node that depends on another one needs that node saved with it.
        None => if proxy.get("dialer-proxy").is_some() {
            return Err("这个节点通过另一个节点中转（dialer-proxy），但没有保存它的前置节点。请在环境配置中重新读取订阅并重新选择该节点。".into());
        },
    }
    Ok(proxy)
}

/// Mihomo proxy list for a node and its front proxies; the first entry is the exit.
fn chain(node: &NodeConfig) -> Result<Vec<Value>, String> {
    let mut hops = Vec::new();
    let mut current = Some(node);
    while let Some(n) = current {
        let name = if hops.is_empty() { "HYDRA-NODE".to_string() } else { format!("HYDRA-VIA-{}", hops.len()) };
        let dialer = n.via.as_ref().map(|_| format!("HYDRA-VIA-{}", hops.len() + 1));
        hops.push(hop(n, &name, dialer.as_deref())?);
        current = n.via.as_deref();
    }
    Ok(hops)
}

/// Turns a configured node into the upstream the relay talks to. Plain proxies are used as
/// they are; Clash nodes and proxy chains run in a managed Mihomo core that is reached over
/// an authenticated loopback SOCKS5 port.
pub fn resolve(node: &NodeConfig) -> Result<(NodeConfig, Option<Core>), String> {
    if node.kind != "clash" && node.via.is_none() { return Ok((node.clone(), None)); }
    let proxies = chain(node)?;
    let port = TcpListener::bind(("127.0.0.1", 0)).and_then(|s| s.local_addr()).map_err(|_| "无法分配代理端口。")?.port();
    let secret = uuid::Uuid::new_v4().to_string();
    let config = json!({"socks-port":port,"allow-lan":false,"mode":"rule","authentication":[format!("hydra:{secret}")],
        "proxies":proxies,"rules":["MATCH,HYDRA-NODE"],"log-level":"silent"});
    // Per-run working dirs live apart from the installed binary path, otherwise the first run
    // turns `<root>/mihomo` into a directory and the installed kernel can never be found.
    let home = run_root().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&home).map_err(|_| "无法创建内核目录。")?;
    let mut cmd = Command::new(mihomo_exe());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; cmd.creation_flags(0x08000000); }
    let mut core = Core(cmd.arg("-d").arg(&home).args(["-f","-"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|_| { let _ = std::fs::remove_dir_all(&home); "无法启动内置的 Mihomo 内核，请重新安装 Claude-Hydra，或通过 HYDRA_MIHOMO_PATH 指定内核路径。" })?, home.clone());
    core.0.stdin.take().ok_or("内核输入不可用")?.write_all(config.to_string().as_bytes()).map_err(|_| "无法传递节点配置。")?;
    let end = Instant::now() + Duration::from_secs(8);
    while core.alive() && Instant::now() < end {
        if TcpStream::connect(("127.0.0.1",port)).is_ok() {
            return Ok((NodeConfig {kind:"socks5".into(),host:"127.0.0.1".into(),port,username:"hydra".into(),password:secret,via:None,..node.clone()},Some(core)));
        }
        std::thread::sleep(Duration::from_millis(60));
    }
    Err("Mihomo 未能启动所选节点。请检查节点协议和内核版本。".into())
}

/// HYDRA_MIHOMO_PATH, then the bundled sidecar next to our executable, then
/// `<storage root>/mihomo`, then well-known install locations (apps launched from Finder
/// do not inherit the shell PATH), then plain PATH lookup.
fn mihomo_exe() -> std::ffi::OsString {
    if let Some(p) = std::env::var_os("HYDRA_MIHOMO_PATH") { return p; }
    let name = if cfg!(windows) { "mihomo.exe" } else { "mihomo" };
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        candidates.push(dir.join(name));
    }
    candidates.push(paths::storage_root().join(name));
    if cfg!(target_os = "macos") {
        candidates.extend(["/opt/homebrew/bin/mihomo", "/usr/local/bin/mihomo"].map(std::path::PathBuf::from));
    }
    candidates.into_iter().find(|p| p.is_file()).map(|p| p.into_os_string()).unwrap_or_else(|| name.into())
}

#[tauri::command]
pub async fn clash_subscription(url: String) -> Result<String,String> {
    tauri::async_runtime::spawn_blocking(move || {
        let url = url::Url::parse(url.trim()).map_err(|_| "订阅地址无效。")?;
        if !matches!(url.scheme(),"http"|"https") { return Err("订阅地址须为 HTTP 或 HTTPS。".into()); }
        let mut text = String::new();
        reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build().map_err(|_| "无法初始化网络。")?
            .get(url).header("User-Agent","clash.meta").send().and_then(|r|r.error_for_status()).map_err(|e| {
                if let Some(status) = e.status() { format!("订阅服务器返回 HTTP {}。", status.as_u16()) }
                else if e.is_timeout() { "订阅下载超时。".into() }
                else if e.is_connect() { "无法连接订阅服务器，请检查网络或系统代理。".into() }
                else { "订阅请求失败，请检查地址和网络连接。".into() }
            })?
            .take(4_194_305).read_to_string(&mut text).map_err(|_| "订阅不是有效文本。")?;
        if text.len()>4_194_304 { return Err("订阅超过 4 MB。".into()); }
        Ok(text)
    }).await.map_err(|_|"订阅任务失败。".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_front_proxies_with_dialer_proxy() {
        let front = NodeConfig { kind: "socks5".into(), host: "front.example".into(), port: 1080, username: "u".into(), password: "p".into(), ..Default::default() };
        let middle = NodeConfig { kind: "https".into(), host: "mid.example".into(), port: 443, via: Some(Box::new(front)), ..Default::default() };
        let exit = json!({"type":"trojan","name":"exit","server":"exit.example","port":443,"password":"x","dialer-proxy":"old"});
        let node = NodeConfig { kind: "clash".into(), host: "exit.example".into(), port: 443, password: exit.to_string(), via: Some(Box::new(middle)), ..Default::default() }.validated().unwrap();
        let hops = chain(&node).unwrap();
        assert_eq!(hops.len(), 3);
        assert_eq!((hops[0]["name"].as_str(), hops[0]["dialer-proxy"].as_str()), (Some("HYDRA-NODE"), Some("HYDRA-VIA-1")));
        assert_eq!((hops[1]["type"].as_str(), hops[1]["tls"].as_bool(), hops[1]["dialer-proxy"].as_str()), (Some("http"), Some(true), Some("HYDRA-VIA-2")));
        assert_eq!((hops[2]["type"].as_str(), hops[2]["username"].as_str()), (Some("socks5"), Some("u")));
        assert!(hops[2].get("dialer-proxy").is_none());
        // A dependent subscription node without its front node is refused, not run directly.
        let alone = NodeConfig { via: None, ..node.clone() };
        assert!(chain(&alone).is_err());
        // A plain proxy without a chain needs no core at all.
        let plain = NodeConfig { kind: "socks5".into(), host: "h".into(), port: 1, ..Default::default() };
        assert!(resolve(&plain).unwrap().1.is_none());
        let deep = (0..crate::node::MAX_FRONT + 1).fold(plain.clone(), |n, _| NodeConfig { via: Some(Box::new(n)), ..plain.clone() });
        assert!(deep.validated().is_err());
    }

    #[test]
    #[ignore = "requires Mihomo, HYDRA_TEST_PROXY and internet"]
    fn imported_node_reaches_exit_through_managed_core() {
        let url = url::Url::parse(&std::env::var("HYDRA_TEST_PROXY").unwrap()).unwrap();
        let host = url.host_str().unwrap();
        let port = url.port().unwrap();
        let proxy = json!({"type":"http","name":"test","server":host,"port":port});
        let config = NodeConfig { kind:"clash".into(), host:host.into(), port,
            password:proxy.to_string(), ..Default::default() }.validated().unwrap();
        let (route, _core) = resolve(&config).unwrap();
        let exit = crate::node::inspect(&route.client(Duration::from_secs(20)).unwrap(), &route).unwrap();
        assert!(!exit.country.is_empty());
    }
}
