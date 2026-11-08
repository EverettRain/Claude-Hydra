//! Local per-account relay: 127.0.0.1 SOCKS5 + HTTP proxy that forwards to the account's node.
//!
//! - Claude and its child processes only ever see `127.0.0.1:<port>`; the node's credentials
//!   stay inside Hydra.
//! - Hostnames are passed to the node unresolved, so DNS happens at the node.
//! - UDP (SOCKS5 UDP ASSOCIATE) and BIND are refused. There is no direct fallback: if the node
//!   is unreachable, or Hydra is not running, the account has no network.
//! - Relays live in the long-running Hydra tray process. Other Hydra processes (CLI, watchdog)
//!   start the tray process when they need a relay.
use crate::node::NodeConfig;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

struct Relay {
    port: u16,
    node: Arc<RwLock<NodeConfig>>,
    stop: Arc<AtomicBool>,
}

static HOST: AtomicBool = AtomicBool::new(false);
fn relays() -> &'static Mutex<HashMap<String, Relay>> {
    static R: OnceLock<Mutex<HashMap<String, Relay>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Marks this process as the relay host (the GUI / tray process).
pub fn set_host() { HOST.store(true, Ordering::SeqCst); }

/// Starts or updates the relay for an account in this process and returns its port.
fn ensure(id: &str, node: &NodeConfig, preferred: Option<u16>) -> Result<u16, String> {
    let mut map = relays().lock().map_err(|_| "中继状态错误。")?;
    if let Some(r) = map.get(id) {
        if let Ok(mut n) = r.node.write() { *n = node.clone(); }
        return Ok(r.port);
    }
    let listener = preferred
        .and_then(|p| TcpListener::bind((Ipv4Addr::LOCALHOST, p)).ok())
        .map(Ok)
        .unwrap_or_else(|| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
        .map_err(|e| format!("无法启动本地代理中继：{e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let relay = Relay { port, node: Arc::new(RwLock::new(node.clone())), stop: Arc::new(AtomicBool::new(false)) };
    let (cfg, stop) = (relay.node.clone(), relay.stop.clone());
    std::thread::Builder::new().name(format!("relay-{port}")).spawn(move || serve(listener, cfg, stop)).map_err(|e| e.to_string())?;
    map.insert(id.to_string(), relay);
    Ok(port)
}

fn stop(id: &str) {
    if let Some(r) = relays().lock().ok().and_then(|mut m| m.remove(id)) {
        r.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, r.port)), Duration::from_millis(200));
    }
}

/// Host process: makes the running relays match `settings.json` and persists their ports.
pub fn sync_all() {
    if !HOST.load(Ordering::SeqCst) { return; }
    let s = crate::settings::load();
    let mut wanted = Vec::new();
    let mut ports = Vec::new();
    for (id, acc) in &s.accounts {
        let Some(node) = acc.node() else { continue };
        wanted.push(id.clone());
        match ensure(id, &node, acc.relay_port) {
            Ok(port) if acc.relay_port != Some(port) => ports.push((id.clone(), port)),
            Ok(_) => {}
            Err(e) => crate::paths::append_log("hydra-error.log", &format!("relay {id}: {e}")),
        }
    }
    let running: Vec<String> = relays().lock().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    for id in running.iter().filter(|id| !wanted.contains(id)) { stop(id); }
    if !ports.is_empty() {
        let _ = crate::settings::update(|s| for (id, port) in &ports { s.account_mut(id).relay_port = Some(*port); });
    }
}

pub fn reachable(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_millis(300)).is_ok()
}

/// Returns the relay port for launching an account, starting the Hydra tray process if needed.
pub fn port_for_launch(id: &str, node: &NodeConfig) -> Result<u16, String> {
    if HOST.load(Ordering::SeqCst) {
        let preferred = crate::settings::load().account(id).relay_port;
        let port = ensure(id, node, preferred)?;
        if preferred != Some(port) { crate::settings::update(|s| s.account_mut(id).relay_port = Some(port))?; }
        return Ok(port);
    }
    let current = || crate::settings::load().account(id).relay_port;
    if let Some(port) = current().filter(|p| reachable(*p)) { return Ok(port); }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe).arg("--tray")
        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
        .spawn().map_err(|e| format!("无法启动 Claude-Hydra 后台：{e}"))?;
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(port) = current().filter(|p| reachable(*p)) { return Ok(port); }
    }
    Err("Claude-Hydra 后台的本地代理中继未就绪，账号未启动。请先打开 Claude-Hydra。".into())
}

// ---- server ---------------------------------------------------------------------------

fn serve(listener: TcpListener, node: Arc<RwLock<NodeConfig>>, stop: Arc<AtomicBool>) {
    for conn in listener.incoming() {
        if stop.load(Ordering::SeqCst) { break; }
        let Ok(client) = conn else { continue };
        // Only loopback clients; the listener is bound to 127.0.0.1 but be explicit.
        if !client.peer_addr().map(|a| a.ip().is_loopback()).unwrap_or(false) { continue; }
        let node = node.read().map(|n| n.clone()).unwrap_or_default();
        let _ = std::thread::Builder::new().name("relay-conn".into()).spawn(move || { let _ = handle(client, &node); });
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Host { Name(String), Ip(IpAddr) }

#[derive(Clone, Debug, PartialEq)]
struct Target { host: Host, port: u16 }

impl Target {
    fn authority(&self) -> String {
        match &self.host {
            Host::Name(n) => format!("{n}:{}", self.port),
            Host::Ip(IpAddr::V6(ip)) => format!("[{ip}]:{}", self.port),
            Host::Ip(ip) => format!("{ip}:{}", self.port),
        }
    }
    fn parse(authority: &str, default_port: u16) -> Option<Target> {
        let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
            let (h, tail) = rest.split_once(']')?;
            (h, tail.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(default_port))?)
        } else if let Some((h, p)) = authority.rsplit_once(':') {
            (h, p.parse().ok()?)
        } else {
            (authority, default_port)
        };
        if host.is_empty() || port == 0 { return None; }
        let host = match host.parse::<IpAddr>() { Ok(ip) => Host::Ip(ip), Err(_) => Host::Name(host.to_ascii_lowercase()) };
        Some(Target { host, port })
    }
}

fn handle(client: TcpStream, node: &NodeConfig) -> io::Result<()> {
    client.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    client.set_nodelay(true)?;
    let mut first = [0u8; 1];
    if client.peek(&mut first)? == 0 { return Ok(()); }
    if first[0] == 5 { socks_server(client, node) } else { http_server(client, node) }
}

fn socks_server(mut client: TcpStream, node: &NodeConfig) -> io::Result<()> {
    let mut head = [0u8; 2];
    client.read_exact(&mut head)?;
    let mut methods = vec![0u8; head[1] as usize];
    client.read_exact(&mut methods)?;
    if !methods.contains(&0) { return client.write_all(&[5, 0xFF]); }
    client.write_all(&[5, 0])?;
    let mut req = [0u8; 4];
    client.read_exact(&mut req)?;
    let host = read_socks_addr(&mut client, req[3])?;
    let mut port = [0u8; 2];
    client.read_exact(&mut port)?;
    let reply = |client: &mut TcpStream, code: u8| client.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]);
    if req[1] != 1 {
        // UDP ASSOCIATE / BIND: refused so UDP never leaves outside the node.
        return reply(&mut client, 7);
    }
    let target = Target { host, port: u16::from_be_bytes(port) };
    match open_tunnel(node, &target) {
        Ok(upstream) => { reply(&mut client, 0)?; pipe(client, upstream) }
        Err(_) => reply(&mut client, 5),
    }
}

fn read_socks_addr(s: &mut impl Read, atyp: u8) -> io::Result<Host> {
    Ok(match atyp {
        1 => { let mut b = [0u8; 4]; s.read_exact(&mut b)?; Host::Ip(IpAddr::from(b)) }
        4 => { let mut b = [0u8; 16]; s.read_exact(&mut b)?; Host::Ip(IpAddr::from(b)) }
        3 => {
            let mut len = [0u8; 1]; s.read_exact(&mut len)?;
            let mut b = vec![0u8; len[0] as usize]; s.read_exact(&mut b)?;
            Host::Name(String::from_utf8(b).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad host"))?)
        }
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad address type")),
    })
}

/// Reads an HTTP head (up to the blank line). Returns the head and any bytes read past it.
fn read_head(s: &mut impl Read) -> io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    loop {
        let n = s.read(&mut chunk)?;
        if n == 0 { return Err(io::ErrorKind::UnexpectedEof.into()); }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(i + 4);
            return Ok((String::from_utf8_lossy(&buf).into_owned(), rest));
        }
        if buf.len() > 64 * 1024 { return Err(io::Error::new(io::ErrorKind::InvalidData, "head too large")); }
    }
}

fn http_server(mut client: TcpStream, node: &NodeConfig) -> io::Result<()> {
    let (head, rest) = read_head(&mut client)?;
    let mut lines = head.split("\r\n");
    let request = lines.next().unwrap_or_default();
    let mut parts = request.split(' ');
    let (method, uri, version) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default(), parts.next().unwrap_or("HTTP/1.1"));
    let bad = |client: &mut TcpStream, code: &str| client.write_all(format!("HTTP/1.1 {code}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").as_bytes());
    if method.eq_ignore_ascii_case("CONNECT") {
        let Some(target) = Target::parse(uri, 443) else { return bad(&mut client, "400 Bad Request") };
        return match open_tunnel(node, &target) {
            Ok(mut upstream) => {
                client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
                if !rest.is_empty() { upstream.write_all(&rest)?; }
                pipe(client, upstream)
            }
            Err(_) => bad(&mut client, "502 Bad Gateway"),
        };
    }
    // Absolute-form plain HTTP request: tunnel to the origin and rewrite to origin-form.
    let Some(after) = uri.strip_prefix("http://") else { return bad(&mut client, "400 Bad Request") };
    let (authority, path) = after.split_at(after.find('/').unwrap_or(after.len()));
    let Some(target) = Target::parse(authority, 80) else { return bad(&mut client, "400 Bad Request") };
    let mut out = format!("{method} {} {version}\r\n", if path.is_empty() { "/" } else { path });
    for line in lines.filter(|l| !l.is_empty()) {
        let name = line.split(':').next().unwrap_or_default().trim().to_ascii_lowercase();
        if !matches!(name.as_str(), "proxy-connection" | "proxy-authorization" | "connection" | "keep-alive") {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    out.push_str("Connection: close\r\n\r\n");
    match open_tunnel(node, &target) {
        Ok(mut upstream) => {
            upstream.write_all(out.as_bytes())?;
            if !rest.is_empty() { upstream.write_all(&rest)?; }
            pipe(client, upstream)
        }
        Err(_) => bad(&mut client, "502 Bad Gateway"),
    }
}

// ---- upstream -------------------------------------------------------------------------

enum Upstream {
    Tcp(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Read for Upstream {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> { match self { Upstream::Tcp(s) => s.read(b), Upstream::Tls(s) => s.read(b) } }
}
impl Write for Upstream {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> { match self { Upstream::Tcp(s) => s.write(b), Upstream::Tls(s) => s.write(b) } }
    fn flush(&mut self) -> io::Result<()> { match self { Upstream::Tcp(s) => s.flush(), Upstream::Tls(s) => s.flush() } }
}
impl Upstream {
    fn socket(&self) -> &TcpStream { match self { Upstream::Tcp(s) => s, Upstream::Tls(s) => s.get_ref() } }
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    static C: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    C.get_or_init(|| {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("tls versions")
            .with_root_certificates(roots).with_no_client_auth())
    }).clone()
}

fn connect_node(node: &NodeConfig) -> io::Result<TcpStream> {
    let addrs: Vec<SocketAddr> = (node.host.as_str(), node.port).to_socket_addrs()?.collect();
    let mut last = io::Error::new(io::ErrorKind::NotFound, "node address not found");
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => { s.set_nodelay(true)?; return Ok(s); }
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn open_tunnel(node: &NodeConfig, target: &Target) -> io::Result<Upstream> {
    let tcp = connect_node(node)?;
    tcp.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    tcp.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let mut up = match node.kind.as_str() {
        "https" => {
            let name = rustls::pki_types::ServerName::try_from(node.host.clone()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "bad TLS name"))?;
            let conn = rustls::ClientConnection::new(tls_config(), name).map_err(io::Error::other)?;
            Upstream::Tls(Box::new(rustls::StreamOwned::new(conn, tcp)))
        }
        _ => Upstream::Tcp(tcp),
    };
    if node.kind == "socks5" { socks_client(&mut up, node, target)? } else { connect_client(&mut up, node, target)? }
    up.socket().set_read_timeout(None)?;
    up.socket().set_write_timeout(None)?;
    Ok(up)
}

fn socks_client(s: &mut Upstream, node: &NodeConfig, target: &Target) -> io::Result<()> {
    let auth = !node.username.is_empty();
    s.write_all(if auth { &[5, 2, 0, 2] } else { &[5, 1, 0] })?;
    let mut sel = [0u8; 2];
    s.read_exact(&mut sel)?;
    let fail = |m: &str| io::Error::new(io::ErrorKind::PermissionDenied, m.to_string());
    match sel[1] {
        0 => {}
        2 if auth => {
            let mut msg = vec![1, node.username.len() as u8];
            msg.extend_from_slice(node.username.as_bytes());
            msg.push(node.password.len() as u8);
            msg.extend_from_slice(node.password.as_bytes());
            s.write_all(&msg)?;
            let mut r = [0u8; 2];
            s.read_exact(&mut r)?;
            if r[1] != 0 { return Err(fail("socks auth rejected")); }
        }
        _ => return Err(fail("socks method rejected")),
    }
    let mut req = vec![5, 1, 0];
    match &target.host {
        Host::Ip(IpAddr::V4(ip)) => { req.push(1); req.extend_from_slice(&ip.octets()); }
        Host::Ip(IpAddr::V6(ip)) => { req.push(4); req.extend_from_slice(&ip.octets()); }
        Host::Name(n) => {
            if n.len() > 255 { return Err(fail("host too long")); }
            req.push(3); req.push(n.len() as u8); req.extend_from_slice(n.as_bytes());
        }
    }
    req.extend_from_slice(&target.port.to_be_bytes());
    s.write_all(&req)?;
    let mut rep = [0u8; 4];
    s.read_exact(&mut rep)?;
    if rep[1] != 0 { return Err(fail("socks connect rejected")); }
    read_socks_addr(s, rep[3])?;
    let mut port = [0u8; 2];
    s.read_exact(&mut port)
}

fn connect_client(s: &mut Upstream, node: &NodeConfig, target: &Target) -> io::Result<()> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let authority = target.authority();
    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if !node.username.is_empty() {
        req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", STANDARD.encode(format!("{}:{}", node.username, node.password))));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    s.flush()?;
    let (head, _) = read_head(s)?;
    let ok = head.split(' ').nth(1).map(|c| c == "200").unwrap_or(false);
    if !ok { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "proxy refused CONNECT")); }
    Ok(())
}

// ---- piping ---------------------------------------------------------------------------

fn copy_half(mut from: impl Read, mut to: impl Write) -> io::Result<u64> {
    let mut buf = [0u8; 16 * 1024];
    let mut total = 0;
    loop {
        let n = from.read(&mut buf)?;
        if n == 0 { return Ok(total); }
        to.write_all(&buf[..n])?;
        to.flush()?;
        total += n as u64;
    }
}

fn pipe(client: TcpStream, upstream: Upstream) -> io::Result<()> {
    client.set_read_timeout(None)?;
    match upstream {
        Upstream::Tcp(up) => {
            let (c2, u2) = (client.try_clone()?, up.try_clone()?);
            let t = std::thread::spawn(move || { let _ = copy_half(&c2, &u2); let _ = u2.shutdown(Shutdown::Write); });
            let _ = copy_half(&up, &client);
            let _ = client.shutdown(Shutdown::Write);
            let _ = t.join();
            Ok(())
        }
        Upstream::Tls(mut tls) => {
            // A TLS stream cannot be split; poll both sides with short timeouts.
            let tick = Some(Duration::from_millis(25));
            client.set_read_timeout(tick)?;
            tls.get_ref().set_read_timeout(tick)?;
            let mut client = client;
            let mut buf = [0u8; 16 * 1024];
            let would_block = |e: &io::Error| matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut);
            loop {
                match client.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => { tls.write_all(&buf[..n])?; tls.flush()?; }
                    Err(e) if would_block(&e) => {}
                    Err(e) => return Err(e),
                }
                match tls.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => client.write_all(&buf[..n])?,
                    Err(e) if would_block(&e) => {}
                    Err(e) => return Err(e),
                }
            }
            let _ = client.shutdown(Shutdown::Both);
            tls.conn.send_close_notify();
            let _ = tls.flush();
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv6Addr, TcpListener};

    /// Fake upstream SOCKS5 server requiring user/pass; echoes the requested target back.
    fn fake_socks(user: &'static str, pass: &'static str) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().take(4) {
                let mut s = s.unwrap();
                std::thread::spawn(move || {
                    let mut h = [0u8; 2]; s.read_exact(&mut h).unwrap();
                    let mut m = vec![0u8; h[1] as usize]; s.read_exact(&mut m).unwrap();
                    assert!(m.contains(&2));
                    s.write_all(&[5, 2]).unwrap();
                    let mut v = [0u8; 2]; s.read_exact(&mut v).unwrap();
                    let mut u = vec![0u8; v[1] as usize]; s.read_exact(&mut u).unwrap();
                    let mut pl = [0u8; 1]; s.read_exact(&mut pl).unwrap();
                    let mut p = vec![0u8; pl[0] as usize]; s.read_exact(&mut p).unwrap();
                    let ok = u == user.as_bytes() && p == pass.as_bytes();
                    s.write_all(&[1, if ok { 0 } else { 1 }]).unwrap();
                    if !ok { return; }
                    let mut r = [0u8; 4]; s.read_exact(&mut r).unwrap();
                    let host = read_socks_addr(&mut s, r[3]).unwrap();
                    let mut port = [0u8; 2]; s.read_exact(&mut port).unwrap();
                    s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
                    let line = format!("{host:?}:{}\n", u16::from_be_bytes(port));
                    s.write_all(line.as_bytes()).unwrap();
                    let mut b = [0u8; 64];
                    let n = s.read(&mut b).unwrap_or(0);
                    let _ = s.write_all(&b[..n]);
                });
            }
        });
        port
    }

    /// Fake upstream HTTP proxy requiring Basic auth.
    fn fake_http() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming().take(2) {
                let mut s = s.unwrap();
                std::thread::spawn(move || {
                    let (head, _) = read_head(&mut s).unwrap();
                    // "u:p w" base64
                    if !head.contains("Proxy-Authorization: Basic dTpwIHc=") { s.write_all(b"HTTP/1.1 407 Auth\r\n\r\n").unwrap(); return; }
                    let target = head.split(' ').nth(1).unwrap().to_string();
                    s.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
                    let mut b = [0u8; 512];
                    let n = s.read(&mut b).unwrap();
                    s.write_all(format!("{target}|{}", String::from_utf8_lossy(&b[..n])).as_bytes()).unwrap();
                });
            }
        });
        port
    }

    fn node(kind: &str, port: u16, user: &str, pass: &str) -> NodeConfig {
        NodeConfig { kind: kind.into(), host: "127.0.0.1".into(), port, username: user.into(), password: pass.into(), ..Default::default() }
    }

    fn start(node: NodeConfig) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || serve(l, Arc::new(RwLock::new(node)), Arc::new(AtomicBool::new(false))));
        port
    }

    fn read_all(s: &mut TcpStream) -> String {
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn socks_client_hostnames_reach_node_unresolved_with_auth() {
        let relay = start(node("socks5", fake_socks("user", "p@ss"), "user", "p@ss"));
        let mut c = TcpStream::connect(("127.0.0.1", relay)).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut sel = [0u8; 2]; c.read_exact(&mut sel).unwrap();
        assert_eq!(sel, [5, 0]);
        let host = b"claude.ai";
        let mut req = vec![5, 1, 0, 3, host.len() as u8];
        req.extend_from_slice(host); req.extend_from_slice(&443u16.to_be_bytes());
        c.write_all(&req).unwrap();
        let mut rep = [0u8; 10]; c.read_exact(&mut rep).unwrap();
        assert_eq!(rep[1], 0);
        c.write_all(b"ping").unwrap();
        let out = read_all(&mut c);
        assert!(out.starts_with("Name(\"claude.ai\"):443\n"), "{out}");
        assert!(out.ends_with("ping"));
    }

    #[test]
    fn socks_udp_associate_is_refused() {
        let relay = start(node("socks5", 1, "", ""));
        let mut c = TcpStream::connect(("127.0.0.1", relay)).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut sel = [0u8; 2]; c.read_exact(&mut sel).unwrap();
        c.write_all(&[5, 3, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
        let mut rep = [0u8; 10]; c.read_exact(&mut rep).unwrap();
        assert_eq!(rep[1], 7, "UDP ASSOCIATE must be refused");
    }

    #[test]
    fn wrong_credentials_fail_closed() {
        let relay = start(node("socks5", fake_socks("user", "right"), "user", "wrong"));
        let mut c = TcpStream::connect(("127.0.0.1", relay)).unwrap();
        c.write_all(&[5, 1, 0]).unwrap();
        let mut sel = [0u8; 2]; c.read_exact(&mut sel).unwrap();
        c.write_all(&[5, 1, 0, 1, 1, 1, 1, 1, 0, 80]).unwrap();
        let mut rep = [0u8; 10]; c.read_exact(&mut rep).unwrap();
        assert_ne!(rep[1], 0);
    }

    #[test]
    fn http_connect_and_plain_requests_use_basic_auth() {
        let relay = start(node("http", fake_http(), "u", "p w"));
        let mut c = TcpStream::connect(("127.0.0.1", relay)).unwrap();
        c.write_all(b"CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: api.anthropic.com:443\r\n\r\n").unwrap();
        let (head, _) = read_head(&mut c).unwrap();
        assert!(head.starts_with("HTTP/1.1 200"));
        c.write_all(b"hello").unwrap();
        assert_eq!(read_all(&mut c), "api.anthropic.com:443|hello");

        let mut c = TcpStream::connect(("127.0.0.1", relay)).unwrap();
        c.write_all(b"GET http://example.com/a?b HTTP/1.1\r\nHost: example.com\r\nProxy-Connection: keep-alive\r\n\r\n").unwrap();
        let out = read_all(&mut c);
        assert!(out.starts_with("example.com:80|GET /a?b HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n"), "{out}");
    }

    /// Real forwarding SOCKS5 node with user/pass auth (for the network test below).
    fn forwarding_socks() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || for s in l.incoming() {
            let mut s = s.unwrap();
            std::thread::spawn(move || {
                let mut h = [0u8; 2]; s.read_exact(&mut h).unwrap();
                let mut m = vec![0u8; h[1] as usize]; s.read_exact(&mut m).unwrap();
                s.write_all(&[5, 2]).unwrap();
                let mut v = [0u8; 2]; s.read_exact(&mut v).unwrap();
                let mut u = vec![0u8; v[1] as usize]; s.read_exact(&mut u).unwrap();
                let mut pl = [0u8; 1]; s.read_exact(&mut pl).unwrap();
                let mut p = vec![0u8; pl[0] as usize]; s.read_exact(&mut p).unwrap();
                s.write_all(&[1, 0]).unwrap();
                let mut r = [0u8; 4]; s.read_exact(&mut r).unwrap();
                let host = match read_socks_addr(&mut s, r[3]).unwrap() { Host::Name(n) => n, Host::Ip(ip) => ip.to_string() };
                let mut port = [0u8; 2]; s.read_exact(&mut port).unwrap();
                let up = TcpStream::connect((host.as_str(), u16::from_be_bytes(port))).unwrap();
                s.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
                let _ = pipe(s, Upstream::Tcp(up));
            });
        });
        port
    }

    #[test]
    #[ignore = "needs internet"]
    fn real_https_through_relay_and_authenticated_node() {
        let n = node("socks5", forwarding_socks(), "user", "pass");
        let relay = start(n.clone());
        let client = crate::node::relay_client(relay, Duration::from_secs(20)).unwrap();
        let info = crate::node::inspect(&client, &NodeConfig { ip_version: "ipv4".into(), geo_channel: "ipinfo".into(), ..n }).unwrap();
        println!("exit via relay: {info:?}");
        assert!(info.ip.parse::<std::net::Ipv4Addr>().is_ok());
        // Plain-HTTP child-process style request through the relay's HTTP side.
        let http = reqwest::blocking::Client::builder().no_proxy().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{relay}")).unwrap()).build().unwrap();
        assert!(http.get("http://ip-api.com/json/?fields=status").send().unwrap().text().unwrap().contains("success"));
        assert!(http.get("https://api4.ipify.org").send().unwrap().status().is_success());
    }

    fn chrome(args: &[String], url: &str) -> String {
        let exe = std::env::var("HYDRA_TEST_CHROME").expect("set HYDRA_TEST_CHROME to a Chromium binary");
        let profile = std::env::temp_dir().join(format!("hydra-chrome-{}-{}", std::process::id(), crate::paths::now_millis()));
        let out = std::process::Command::new(exe)
            .args(["--headless=new", "--disable-gpu", "--no-first-run", "--timeout=9000"])
            .arg(format!("--user-data-dir={}", profile.display()))
            .args(args).arg("--dump-dom").arg(url).output().unwrap();
        let _ = std::fs::remove_dir_all(profile);
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Runs the real Chromium switches Hydra passes to Claude.
    #[test]
    #[ignore = "needs internet and HYDRA_TEST_CHROME"]
    fn chromium_switches_route_everything_through_relay() {
        let relay = start(node("socks5", forwarding_socks(), "user", "pass"));
        let args = crate::node::chromium_args(relay);
        let direct = chrome(&[], "https://api4.ipify.org");
        let proxied = chrome(&args, "https://api4.ipify.org");
        assert!(proxied.contains("<body>"), "proxied page should load: {proxied}");
        println!("direct and proxied loaded: {} / {}", direct.contains("<body>"), proxied.contains("<body>"));

        // Relay down: nothing loads (no direct fallback).
        let dead = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let closed = chrome(&crate::node::chromium_args(dead), "https://api4.ipify.org");
        let ip_like = |s: &str| s.split(|c: char| !(c.is_ascii_digit() || c == '.')).any(|t| t.parse::<std::net::Ipv4Addr>().is_ok());
        assert!(!ip_like(&closed), "must not load without the relay: {closed}");

        // WebRTC: no host/srflx candidates may be gathered over UDP. The page is served from
        // loopback (never proxied) and holds its load event open while ICE gathering runs.
        let page = TcpListener::bind("127.0.0.1:0").unwrap();
        let page_port = page.local_addr().unwrap().port();
        std::thread::spawn(move || for s in page.incoming() {
            let mut s = s.unwrap();
            std::thread::spawn(move || {
                let (head, _) = read_head(&mut s).unwrap_or_default();
                if head.starts_with("GET /slow") { std::thread::sleep(Duration::from_secs(5)); let _ = s.write_all(b"HTTP/1.1 404 x\r\nContent-Length: 0\r\n\r\n"); return; }
                let body = "<html><body><pre id=o>CANDS:[]</pre><script>const o=document.getElementById('o');const seen=[];\
                    const pc=new RTCPeerConnection({iceServers:[{urls:'stun:stun.l.google.com:19302'}]});\
                    pc.onicecandidate=e=>{if(e.candidate){seen.push(e.candidate.candidate);o.textContent='CANDS:'+JSON.stringify(seen)}};\
                    pc.createDataChannel('x');pc.createOffer().then(x=>pc.setLocalDescription(x));</script><img src=/slow></body></html>";
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes());
            });
        });
        let url = format!("http://127.0.0.1:{page_port}/rtc");
        let count = |out: &str| out.matches("candidate:").count();
        let baseline = chrome(&[], &url);
        let rtc = chrome(&args, &url);
        println!("webrtc candidates: without switches {}, with switches {}", count(&baseline), count(&rtc));
        assert!(rtc.contains("CANDS:["), "candidate list was not written: {rtc}");
        assert!(!rtc.contains(" udp ") && !rtc.contains(" UDP ") && !rtc.contains("typ srflx"), "WebRTC leaked: {rtc}");

        // DNS: names are resolved at the node; a name the local resolver can't see still loads through it.
        let dns = chrome(&args, "https://ipinfo.io/json");
        assert!(dns.contains("\"ip\""), "{dns}");
    }

    #[test]
    fn parses_authorities() {
        assert_eq!(Target::parse("[::1]:8443", 443), Some(Target { host: Host::Ip(IpAddr::V6(Ipv6Addr::LOCALHOST)), port: 8443 }));
        assert_eq!(Target::parse("Example.COM", 80).unwrap().authority(), "example.com:80");
        assert!(Target::parse(":80", 80).is_none());
    }
}
