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

pub fn resolve(node: &NodeConfig) -> Result<(NodeConfig, Option<Core>), String> {
    if node.kind != "clash" { return Ok((node.clone(), None)); }
    let mut proxy: Value = serde_json::from_str(&node.password).map_err(|_| "节点配置无效。")?;
    proxy["name"] = json!("HYDRA-NODE");
    if proxy.get("dialer-proxy").is_some() { return Err("暂不支持依赖其他节点的代理链。".into()); }
    let port = TcpListener::bind(("127.0.0.1", 0)).and_then(|s| s.local_addr()).map_err(|_| "无法分配代理端口。")?.port();
    let secret = uuid::Uuid::new_v4().to_string();
    let config = json!({"socks-port":port,"allow-lan":false,"mode":"rule","authentication":[format!("hydra:{secret}")],
        "proxies":[proxy],"rules":["MATCH,HYDRA-NODE"],"log-level":"silent"});
    // Per-run working dirs live apart from the installed binary path, otherwise the first run
    // turns `<root>/mihomo` into a directory and the installed kernel can never be found.
    let home = paths::storage_root().join("mihomo-run").join(uuid::Uuid::new_v4().to_string());
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
            return Ok((NodeConfig {kind:"socks5".into(),host:"127.0.0.1".into(),port,username:"hydra".into(),password:secret,..node.clone()},Some(core)));
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
            .get(url).header("User-Agent","clash.meta").send().and_then(|r|r.error_for_status()).map_err(|_| "订阅下载失败。")?
            .take(4_194_305).read_to_string(&mut text).map_err(|_| "订阅不是有效文本。")?;
        if text.len()>4_194_304 { return Err("订阅超过 4 MB。".into()); }
        Ok(text)
    }).await.map_err(|_|"订阅任务失败。".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

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
