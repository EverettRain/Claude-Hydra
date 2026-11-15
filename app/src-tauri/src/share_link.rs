//! Proxy share links (`ss://`, `vmess://`, `vless://`, `trojan://`, `hysteria2://`, `tuic://`,
//! `socks://`, `http://`) turned into Mihomo proxy entries, the form a Clash subscription node is
//! kept in, so a single node runs exactly like one picked from a subscription. Fields map as in
//! Mihomo's own converter (`common/convert`): a link means here what it means in Clash clients.

use crate::node::NodeConfig;
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Nodes read from pasted text, and how many lines could not be read.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parsed {
    pub nodes: Vec<NodeConfig>,
    pub skipped: usize,
    /// Why the first unreadable line was refused, to show when nothing could be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_error: Option<String>,
}

/// Reads one link, several (one per line), or a base64 subscription of links.
pub fn parse(text: &str) -> Parsed {
    let text = text.trim();
    let decoded = if text.contains("://") { None } else { base64_text(text) };
    let mut out = Parsed { nodes: Vec::new(), skipped: 0, first_error: None };
    if decoded.is_none() && !text.contains("://") {
        out.first_error = Some("内容既不是 Clash 配置，也不是节点链接。".into());
        return out;
    }
    for line in decoded.as_deref().unwrap_or(text).lines().map(str::trim).filter(|l| !l.is_empty()) {
        match link(line) {
            Ok(proxy) => out.nodes.push(node(proxy)),
            Err(e) => {
                out.skipped += 1;
                out.first_error.get_or_insert(e);
            }
        }
    }
    out
}

#[tauri::command]
pub fn parse_node_links(text: String) -> Result<Parsed, String> {
    if text.len() > 4_194_304 { return Err("内容超过 4 MB。".into()); }
    let parsed = parse(&text);
    if parsed.nodes.is_empty() {
        return Err(parsed.first_error.unwrap_or_else(|| "没有可识别的节点链接。".into()));
    }
    Ok(parsed)
}

fn node(proxy: Value) -> NodeConfig {
    NodeConfig {
        kind: "clash".into(),
        host: proxy["server"].as_str().unwrap_or_default().to_string(),
        port: proxy["port"].as_u64().unwrap_or_default() as u16,
        password: proxy.to_string(),
        ip_version: "ipv4".into(),
        geo_channel: "ipinfo".into(),
        ..Default::default()
    }
}

/// One link as a Mihomo proxy entry.
fn link(line: &str) -> Result<Value, String> {
    let (scheme, rest) = line.split_once("://").ok_or("不是节点链接。")?;
    let mut proxy = match scheme.to_ascii_lowercase().as_str() {
        "ss" => shadowsocks(rest)?,
        "vmess" => vmess(rest)?,
        "vless" => vless(&Link::read(rest)?)?,
        "trojan" => trojan(&Link::read(rest)?)?,
        "hysteria2" | "hy2" => hysteria2(&Link::read(rest)?)?,
        "tuic" => tuic(&Link::read(rest)?)?,
        "socks" | "socks5" | "socks5h" => plain(&Link::read(rest)?, "socks5", false)?,
        "http" => plain(&Link::read(rest)?, "http", false)?,
        "https" => plain(&Link::read(rest)?, "http", true)?,
        other => return Err(format!("不支持 {other}:// 链接。支持 ss、vmess、vless、trojan、hysteria2、tuic、socks 与 http(s)。")),
    };
    let server = proxy["server"].as_str().unwrap_or_default();
    let port = proxy["port"].as_u64().unwrap_or_default();
    if server.is_empty() || !(1..=65535).contains(&port) { return Err("节点链接缺少服务器或端口。".into()); }
    if proxy["name"].as_str().is_none_or(str::is_empty) {
        proxy["name"] = json!(format!("{} {server}:{port}", proxy["type"].as_str().unwrap_or_default()));
    }
    Ok(proxy)
}

/// The parts every URL-shaped link shares: `user@host:port/path?query#name`.
struct Link {
    user: String,
    host: String,
    port: u16,
    query: HashMap<String, String>,
    name: String,
}

impl Link {
    fn read(rest: &str) -> Result<Link, String> {
        let (rest, name) = rest.split_once('#').unwrap_or((rest, ""));
        let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
        let authority = rest.split('/').next().unwrap_or_default();
        let (user, host_port) = authority.rsplit_once('@').unwrap_or(("", authority));
        let (host, port) = host_port_of(host_port)?;
        Ok(Link {
            user: percent_decode(user),
            host,
            port,
            query: url::form_urlencoded::parse(query.as_bytes()).into_owned().collect(),
            name: percent_decode(name),
        })
    }

    fn q(&self, key: &str) -> Option<&str> {
        self.query.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    fn flag(&self, key: &str) -> bool {
        matches!(self.q(key), Some("1" | "true"))
    }

    fn base(&self, kind: &str) -> Map<String, Value> {
        let mut p = Map::new();
        p.insert("name".into(), json!(self.name));
        p.insert("type".into(), json!(kind));
        p.insert("server".into(), json!(self.host));
        p.insert("port".into(), json!(self.port));
        p
    }
}

fn host_port_of(text: &str) -> Result<(String, u16), String> {
    let (host, port) = match text.strip_prefix('[') {
        Some(v6) => v6.split_once("]:").ok_or("节点地址无效。")?,
        None => text.rsplit_once(':').ok_or("节点链接缺少端口。")?,
    };
    let port = port.trim_end_matches('/').parse::<u16>().ok().filter(|p| *p > 0).ok_or("节点端口无效。")?;
    if host.is_empty() { return Err("节点链接缺少服务器。".into()); }
    Ok((host.to_string(), port))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[i], bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(hi), Some(lo)) => { out.push((hi * 16 + lo) as u8); i += 3; }
            (b, _, _) => { out.push(b); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Base64 in any of the alphabets and paddings links are written with, as text.
fn base64_text(text: &str) -> Option<String> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let unpadded = compact.trim_end_matches('=');
    [STANDARD.decode(&compact), STANDARD_NO_PAD.decode(unpadded), URL_SAFE.decode(&compact), URL_SAFE_NO_PAD.decode(unpadded)]
        .into_iter().flatten().find_map(|b| String::from_utf8(b).ok())
}

fn alpn(value: Option<&str>) -> Option<Value> {
    value.map(|v| json!(v.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>()))
}

/// The transport of a vless / trojan link (`type`, with `path`, `host`, `serviceName`).
fn transport(l: &Link, p: &mut Map<String, Value>) -> Result<(), String> {
    let host = l.q("host");
    let path = l.q("path").unwrap_or("/");
    match l.q("type").unwrap_or("tcp") {
        "tcp" if l.q("headerType") == Some("http") => {
            p.insert("network".into(), json!("http"));
            p.insert("http-opts".into(), json!({"path": [path], "headers": host.map(|h| json!({"Host": [h]})).unwrap_or(json!({}))}));
        }
        "tcp" => {}
        net @ ("ws" | "httpupgrade") => {
            let mut opts = json!({"path": path});
            if let Some(h) = host { opts["headers"] = json!({"Host": h}); }
            if net == "httpupgrade" { opts["v2ray-http-upgrade"] = json!(true); }
            p.insert("network".into(), json!("ws"));
            p.insert("ws-opts".into(), opts);
        }
        "grpc" => {
            p.insert("network".into(), json!("grpc"));
            p.insert("grpc-opts".into(), json!({"grpc-service-name": l.q("serviceName").unwrap_or_default()}));
        }
        "h2" | "http" => {
            p.insert("network".into(), json!("h2"));
            p.insert("h2-opts".into(), json!({"path": path, "host": host.map(|h| vec![h]).unwrap_or_default()}));
        }
        other => return Err(format!("不支持 {other} 传输方式。")),
    }
    Ok(())
}

/// TLS options shared by vless and trojan.
fn tls_options(l: &Link, p: &mut Map<String, Value>) {
    if let Some(sni) = l.q("sni").or(l.q("peer")) { p.insert(if p["type"] == "trojan" { "sni" } else { "servername" }.into(), json!(sni)); }
    if let Some(fp) = l.q("fp") { p.insert("client-fingerprint".into(), json!(fp)); }
    if let Some(a) = alpn(l.q("alpn")) { p.insert("alpn".into(), a); }
    if l.flag("allowInsecure") || l.flag("insecure") { p.insert("skip-cert-verify".into(), json!(true)); }
}

fn vless(l: &Link) -> Result<Value, String> {
    if l.user.is_empty() { return Err("vless 链接缺少 UUID。".into()); }
    let mut p = l.base("vless");
    p.insert("uuid".into(), json!(l.user));
    p.insert("udp".into(), json!(true));
    if let Some(flow) = l.q("flow") { p.insert("flow".into(), json!(flow)); }
    if let Some(enc) = l.q("encryption").filter(|e| *e != "none") { p.insert("encryption".into(), json!(enc)); }
    match l.q("security").unwrap_or("none") {
        "none" => {}
        "tls" => { p.insert("tls".into(), json!(true)); tls_options(l, &mut p); }
        "reality" => {
            p.insert("tls".into(), json!(true));
            tls_options(l, &mut p);
            let key = l.q("pbk").ok_or("reality 链接缺少公钥（pbk）。")?;
            p.insert("reality-opts".into(), json!({"public-key": key, "short-id": l.q("sid").unwrap_or_default()}));
            // Mihomo needs a fingerprint for REALITY; clients default to Chrome's.
            p.entry("client-fingerprint").or_insert(json!("chrome"));
        }
        other => return Err(format!("不支持 {other} 加密方式。")),
    }
    transport(l, &mut p)?;
    Ok(Value::Object(p))
}

fn trojan(l: &Link) -> Result<Value, String> {
    if l.user.is_empty() { return Err("trojan 链接缺少密码。".into()); }
    let mut p = l.base("trojan");
    p.insert("password".into(), json!(l.user));
    p.insert("udp".into(), json!(true));
    tls_options(l, &mut p);
    transport(l, &mut p)?;
    Ok(Value::Object(p))
}

fn hysteria2(l: &Link) -> Result<Value, String> {
    let mut p = l.base("hysteria2");
    p.insert("password".into(), json!(l.user));
    if let Some(sni) = l.q("sni") { p.insert("sni".into(), json!(sni)); }
    if let Some(obfs) = l.q("obfs") {
        p.insert("obfs".into(), json!(obfs));
        p.insert("obfs-password".into(), json!(l.q("obfs-password").unwrap_or_default()));
    }
    if let Some(ports) = l.q("mport") { p.insert("ports".into(), json!(ports)); }
    if let Some(a) = alpn(l.q("alpn")) { p.insert("alpn".into(), a); }
    if l.flag("insecure") { p.insert("skip-cert-verify".into(), json!(true)); }
    if let Some(pin) = l.q("pinSHA256") { p.insert("fingerprint".into(), json!(pin)); }
    Ok(Value::Object(p))
}

fn tuic(l: &Link) -> Result<Value, String> {
    let (uuid, password) = l.user.split_once(':').ok_or("tuic 链接缺少 UUID 或密码。")?;
    let mut p = l.base("tuic");
    p.insert("uuid".into(), json!(uuid));
    p.insert("password".into(), json!(password));
    p.insert("udp".into(), json!(true));
    if let Some(cc) = l.q("congestion_control") { p.insert("congestion-controller".into(), json!(cc)); }
    if let Some(mode) = l.q("udp_relay_mode") { p.insert("udp-relay-mode".into(), json!(mode)); }
    if let Some(sni) = l.q("sni") { p.insert("sni".into(), json!(sni)); }
    if let Some(a) = alpn(l.q("alpn")) { p.insert("alpn".into(), a); }
    if l.flag("allow_insecure") || l.flag("insecure") { p.insert("skip-cert-verify".into(), json!(true)); }
    if l.flag("disable_sni") { p.insert("disable-sni".into(), json!(true)); }
    Ok(Value::Object(p))
}

/// `socks://` and `http(s)://`; v2rayN writes the socks credentials as base64.
fn plain(l: &Link, kind: &str, tls: bool) -> Result<Value, String> {
    let mut p = l.base(kind);
    let user = if kind == "socks5" { base64_text(&l.user).filter(|u| u.contains(':')).unwrap_or_else(|| l.user.clone()) } else { l.user.clone() };
    if let Some((name, password)) = user.split_once(':') {
        p.insert("username".into(), json!(name));
        p.insert("password".into(), json!(password));
    }
    if tls { p.insert("tls".into(), json!(true)); }
    Ok(Value::Object(p))
}

/// `ss://` in both forms: SIP002 (`base64(method:password)@host:port`, the userinfo also seen
/// percent-encoded for 2022 ciphers) and the legacy `base64(method:password@host:port)`.
fn shadowsocks(rest: &str) -> Result<Value, String> {
    let (rest, name) = rest.split_once('#').unwrap_or((rest, ""));
    let (body, query) = rest.split_once('?').unwrap_or((rest, ""));
    let body = body.trim_end_matches('/');
    let (userinfo, host_port) = match body.rsplit_once('@') {
        Some((user, hp)) => (base64_text(user).filter(|u| u.contains(':')).unwrap_or_else(|| percent_decode(user)), hp.to_string()),
        None => {
            let plain = base64_text(body).ok_or("ss 链接无法解码。")?;
            let (user, hp) = plain.rsplit_once('@').ok_or("ss 链接缺少服务器。")?;
            (user.to_string(), hp.to_string())
        }
    };
    let (cipher, password) = userinfo.split_once(':').ok_or("ss 链接缺少加密方式或密码。")?;
    let (host, port) = host_port_of(&host_port)?;
    let mut p = json!({"name": percent_decode(name), "type": "ss", "server": host, "port": port, "cipher": cipher, "password": password, "udp": true});
    let query: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
    if let Some(plugin) = query.get("plugin").filter(|v| !v.is_empty()) {
        let mut parts = plugin.split(';');
        let kind = parts.next().unwrap_or_default();
        let opts: HashMap<&str, &str> = parts.map(|kv| kv.split_once('=').unwrap_or((kv, ""))).collect();
        match kind {
            "obfs-local" | "simple-obfs" => {
                p["plugin"] = json!("obfs");
                p["plugin-opts"] = json!({"mode": opts.get("obfs").copied().unwrap_or("http"), "host": opts.get("obfs-host").copied().unwrap_or_default()});
            }
            "v2ray-plugin" => {
                p["plugin"] = json!("v2ray-plugin");
                p["plugin-opts"] = json!({"mode": "websocket", "host": opts.get("host").copied().unwrap_or_default(),
                    "path": opts.get("path").copied().unwrap_or("/"), "tls": opts.contains_key("tls")});
            }
            other => return Err(format!("不支持 ss 插件 {other}。")),
        }
    }
    Ok(p)
}

/// `vmess://` as v2rayN writes it: base64 of a JSON object.
fn vmess(rest: &str) -> Result<Value, String> {
    let text = base64_text(rest.split('#').next().unwrap_or_default()).ok_or("vmess 链接无法解码。")?;
    let v: Value = serde_json::from_str(&text).map_err(|_| "vmess 链接不是 v2rayN 格式。")?;
    let s = |k: &str| match &v[k] { Value::String(s) => s.trim().to_string(), Value::Number(n) => n.to_string(), _ => String::new() };
    let port = s("port").parse::<u16>().map_err(|_| "vmess 节点端口无效。")?;
    if s("id").is_empty() { return Err("vmess 链接缺少 UUID。".into()); }
    let mut p = json!({"name": s("ps"), "type": "vmess", "server": s("add"), "port": port, "uuid": s("id"),
        "alterId": s("aid").parse::<u32>().unwrap_or(0), "cipher": if s("scy").is_empty() { "auto".into() } else { s("scy") }, "udp": true});
    let host = s("host");
    let path = if s("path").is_empty() { "/".to_string() } else { s("path") };
    if s("tls") == "tls" {
        p["tls"] = json!(true);
        let sni = if s("sni").is_empty() { host.clone() } else { s("sni") };
        if !sni.is_empty() { p["servername"] = json!(sni); }
        let protocols = s("alpn");
        if let Some(a) = alpn(Some(protocols.as_str()).filter(|a| !a.is_empty())) { p["alpn"] = a; }
        if !s("fp").is_empty() { p["client-fingerprint"] = json!(s("fp")); }
    }
    match s("net").as_str() {
        "" | "tcp" if s("type") == "http" => {
            p["network"] = json!("http");
            p["http-opts"] = json!({"path": [path], "headers": if host.is_empty() { json!({}) } else { json!({"Host": [host]}) }});
        }
        "" | "tcp" => {}
        net @ ("ws" | "httpupgrade") => {
            p["network"] = json!("ws");
            p["ws-opts"] = json!({"path": path});
            if !host.is_empty() { p["ws-opts"]["headers"] = json!({"Host": host}); }
            if net == "httpupgrade" { p["ws-opts"]["v2ray-http-upgrade"] = json!(true); }
        }
        "grpc" => { p["network"] = json!("grpc"); p["grpc-opts"] = json!({"grpc-service-name": s("path")}); }
        "h2" | "http" => { p["network"] = json!("h2"); p["h2-opts"] = json!({"path": path, "host": if host.is_empty() { vec![] } else { vec![host] }}); }
        other => return Err(format!("不支持 vmess 的 {other} 传输方式。")),
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;

    fn one(link: &str) -> Value {
        let parsed = parse(link);
        assert_eq!(parsed.skipped, 0, "{link}: {:?}", parsed.first_error);
        serde_json::from_str(&parsed.nodes[0].password).unwrap()
    }

    #[test]
    fn vless_reality_keeps_its_key_flow_and_name() {
        // Arrange
        let link = "vless://8b6f0b5e-1111-4222-8333-944455556666@203.0.113.9:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.example.com&fp=chrome&pbk=PUBKEY&sid=ab12&type=tcp#%E4%B8%9C%E4%BA%AC%20VPS";

        // Act
        let p = one(link);

        // Assert
        assert_eq!(p["type"], "vless");
        assert_eq!((p["server"].as_str(), p["port"].as_u64()), (Some("203.0.113.9"), Some(443)));
        assert_eq!(p["name"], "东京 VPS");
        assert_eq!(p["uuid"], "8b6f0b5e-1111-4222-8333-944455556666");
        assert_eq!(p["flow"], "xtls-rprx-vision");
        assert_eq!(p["tls"], true);
        assert_eq!(p["servername"], "www.example.com");
        assert_eq!(p["reality-opts"], json!({"public-key": "PUBKEY", "short-id": "ab12"}));
        assert!(p.get("network").is_none());
    }

    #[test]
    fn trojan_over_websocket_carries_path_host_and_sni() {
        // Arrange
        let link = "trojan://p%40ss@[2001:db8::1]:8443?type=ws&path=%2Fws&host=cdn.example.com&sni=cdn.example.com&allowInsecure=1#ws";

        // Act
        let p = one(link);

        // Assert
        assert_eq!((p["server"].as_str(), p["port"].as_u64()), (Some("2001:db8::1"), Some(8443)));
        assert_eq!(p["password"], "p@ss");
        assert_eq!(p["sni"], "cdn.example.com");
        assert_eq!(p["skip-cert-verify"], true);
        assert_eq!(p["network"], "ws");
        assert_eq!(p["ws-opts"], json!({"path": "/ws", "headers": {"Host": "cdn.example.com"}}));
    }

    #[test]
    fn shadowsocks_reads_both_link_forms_and_obfs() {
        // Arrange
        let user = STANDARD.encode("aes-256-gcm:secret");
        let sip002 = format!("ss://{user}@198.51.100.4:8388/?plugin=obfs-local%3Bobfs%3Dhttp%3Bobfs-host%3Dbing.com#SIP002");
        let legacy = format!("ss://{}#legacy", STANDARD.encode("chacha20-ietf-poly1305:pw@198.51.100.5:8389"));
        let plain2022 = "ss://2022-blake3-aes-128-gcm:a2V5%3D%3D@198.51.100.6:443#2022";

        // Act
        let (a, b, c) = (one(&sip002), one(&legacy), one(plain2022));

        // Assert
        assert_eq!((a["cipher"].as_str(), a["password"].as_str(), a["port"].as_u64()), (Some("aes-256-gcm"), Some("secret"), Some(8388)));
        assert_eq!(a["plugin"], "obfs");
        assert_eq!(a["plugin-opts"], json!({"mode": "http", "host": "bing.com"}));
        assert_eq!((b["cipher"].as_str(), b["password"].as_str(), b["server"].as_str()), (Some("chacha20-ietf-poly1305"), Some("pw"), Some("198.51.100.5")));
        assert_eq!((c["cipher"].as_str(), c["password"].as_str()), (Some("2022-blake3-aes-128-gcm"), Some("a2V5==")));
    }

    #[test]
    fn vmess_json_link_with_websocket_and_tls() {
        // Arrange
        let body = json!({"v": "2", "ps": "美西", "add": "us.example.net", "port": "443", "id": "11111111-2222-4333-8444-555555555555",
            "aid": 0, "scy": "auto", "net": "ws", "host": "cdn.example.net", "path": "/v", "tls": "tls"});
        let link = format!("vmess://{}", STANDARD.encode(body.to_string()));

        // Act
        let p = one(&link);

        // Assert
        assert_eq!((p["name"].as_str(), p["server"].as_str(), p["port"].as_u64()), (Some("美西"), Some("us.example.net"), Some(443)));
        assert_eq!(p["tls"], true);
        assert_eq!(p["servername"], "cdn.example.net");
        assert_eq!(p["ws-opts"], json!({"path": "/v", "headers": {"Host": "cdn.example.net"}}));
    }

    #[test]
    fn hysteria2_and_tuic_options() {
        // Arrange
        let hy2 = "hy2://auth@203.0.113.7:443/?sni=hy.example.com&obfs=salamander&obfs-password=ob&insecure=1&mport=20000-30000";
        let tuic = "tuic://uuid-1:pw@203.0.113.8:443?congestion_control=bbr&udp_relay_mode=native&alpn=h3&sni=t.example.com";

        // Act
        let (h, t) = (one(hy2), one(tuic));

        // Assert
        assert_eq!((h["type"].as_str(), h["password"].as_str(), h["sni"].as_str()), (Some("hysteria2"), Some("auth"), Some("hy.example.com")));
        assert_eq!((h["obfs"].as_str(), h["obfs-password"].as_str(), h["ports"].as_str()), (Some("salamander"), Some("ob"), Some("20000-30000")));
        assert_eq!(h["skip-cert-verify"], true);
        assert_eq!(h["name"], "hysteria2 203.0.113.7:443");
        assert_eq!((t["uuid"].as_str(), t["password"].as_str(), t["congestion-controller"].as_str()), (Some("uuid-1"), Some("pw"), Some("bbr")));
        assert_eq!(t["alpn"], json!(["h3"]));
    }

    #[test]
    fn a_base64_subscription_reads_every_link_and_counts_the_rest() {
        // Arrange
        let list = format!("{}\nvless://u@203.0.113.9:443?security=tls&type=grpc&serviceName=svc#g\nwireguard://x@1.2.3.4:51820\n",
            "trojan://pw@203.0.113.10:443#t");
        let subscription = STANDARD.encode(list);

        // Act
        let parsed = parse(&subscription);

        // Assert
        assert_eq!(parsed.nodes.len(), 2);
        assert_eq!(parsed.skipped, 1);
        let grpc: Value = serde_json::from_str(&parsed.nodes[1].password).unwrap();
        assert_eq!(grpc["grpc-opts"], json!({"grpc-service-name": "svc"}));
        assert!(parsed.first_error.unwrap().contains("wireguard"));
    }

    #[test]
    fn a_node_from_a_link_passes_the_checks_a_subscription_node_does() {
        // Arrange
        let parsed = parse("trojan://pw@203.0.113.10:443#t");

        // Act
        let checked = parsed.nodes[0].validated();

        // Assert
        assert!(checked.is_ok(), "{checked:?}");
        assert_eq!(checked.unwrap().kind, "clash");
    }

    #[test]
    fn links_without_a_server_or_port_are_refused() {
        // Arrange
        let bad = ["vless://uuid@host-only?security=tls", "trojan://@203.0.113.1:443", "ss://bm90YmFzZTY0", "vless://u@h:0"];

        // Act
        let results: Vec<Parsed> = bad.iter().map(|l| parse(l)).collect();

        // Assert
        for (link, r) in bad.iter().zip(&results) {
            assert!(r.nodes.is_empty() && r.skipped == 1, "{link} should be refused");
        }
    }

    /// What the links turn into is a configuration Mihomo itself accepts (`mihomo -t`), for every
    /// protocol and transport. Skipped where the bundled kernel has not been fetched.
    #[test]
    fn mihomo_accepts_every_converted_node() {
        // Arrange
        let suffix = match std::env::consts::OS { "windows" => "pc-windows-msvc.exe", "macos" => "apple-darwin", _ => "unknown-linux-gnu" };
        let kernel = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries").join(format!("mihomo-{}-{suffix}", std::env::consts::ARCH));
        if !kernel.is_file() { return; }
        let key = "SbVKOEMjK0sIlbwg4akyBg5mL5KZwwB-ed4eEE7YnRc";
        let vmess = STANDARD.encode(json!({"ps": "m", "add": "203.0.113.2", "port": 443, "id": "11111111-2222-4333-8444-555555555555", "net": "grpc", "path": "svc", "tls": "tls"}).to_string());
        let links = [
            format!("vless://11111111-2222-4333-8444-555555555555@203.0.113.1:443?security=reality&sni=www.example.com&pbk={key}&sid=ab12&flow=xtls-rprx-vision#r"),
            "vless://11111111-2222-4333-8444-555555555555@203.0.113.1:443?security=tls&type=ws&path=%2Fv&host=cdn.example.com#w".into(),
            "vless://11111111-2222-4333-8444-555555555555@203.0.113.1:443?security=tls&type=httpupgrade&path=%2Fu#u".into(),
            "trojan://pw@203.0.113.3:443?type=grpc&serviceName=svc&sni=t.example.com#g".into(),
            format!("ss://{}@203.0.113.4:8388/?plugin=v2ray-plugin%3Bhost%3Dcdn.example.com%3Bpath%3D%2Fs%3Btls#s", STANDARD.encode("aes-128-gcm:pw")),
            format!("vmess://{vmess}"),
            "hysteria2://auth@203.0.113.5:443?sni=h.example.com&obfs=salamander&obfs-password=ob#h".into(),
            "tuic://11111111-2222-4333-8444-555555555555:pw@203.0.113.6:443?congestion_control=bbr&alpn=h3#t".into(),
            "socks://dTpw@203.0.113.7:1080#k".into(),
        ];
        let parsed = parse(&links.join("\n"));
        assert_eq!(parsed.skipped, 0, "{:?}", parsed.first_error);
        let proxies: Vec<Value> = parsed.nodes.iter().enumerate().map(|(i, n)| {
            let mut p: Value = serde_json::from_str(&n.password).unwrap();
            p["name"] = json!(format!("n{i}"));
            p
        }).collect();
        let dir = std::env::temp_dir().join(format!("hydra-share-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.yaml"), json!({"proxies": proxies, "rules": ["MATCH,DIRECT"]}).to_string()).unwrap();

        // Act
        let out = std::process::Command::new(&kernel).arg("-t").arg("-d").arg(&dir).arg("-f").arg(dir.join("config.yaml")).output().unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        // Assert
        assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    }
}
