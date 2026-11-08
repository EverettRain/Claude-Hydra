//! Read-only per-profile subscription usage. Never refreshes or writes credentials.
use serde::Serialize;
use serde_json::Value;
use std::{path::Path, time::Duration};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Window { utilization: f64, resets_at: Option<String> }
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota { five_hour: Option<Window>, seven_day: Option<Window>, fetched_at: u64 }

fn parse_window(v: &Value) -> Option<Window> {
    let n = v.get("utilization")?.as_f64()?;
    if !n.is_finite() || !(0.0..=100.0).contains(&n) { return None; }
    Some(Window { utilization: n, resets_at: v.get("resets_at").and_then(Value::as_str).map(str::to_owned) })
}
fn parse_quota(v: &Value) -> Result<Quota, String> {
    let five_hour = parse_window(&v["five_hour"]);
    let seven_day = parse_window(&v["seven_day"]);
    if five_hour.is_none() && seven_day.is_none() { return Err("服务没有返回可识别的额度数据。".into()); }
    Ok(Quota { five_hour, seven_day, fetched_at: crate::paths::now_millis() })
}

fn read_json(dir: &Path, name: &str) -> Result<Value, String> {
    let data = std::fs::read(dir.join(name)).map_err(|_| format!("无法读取账号的 {name}。请先在该账号登录 Claude。"))?;
    serde_json::from_slice(&data).map_err(|_| format!("账号的 {name} 格式无法识别。"))
}

/// Finds the desktop OAuth token of the signed-in account in Electron's encrypted token cache.
/// `decrypt` receives the raw `v10`-prefixed blob and returns plaintext JSON bytes.
///
/// The cache can hold tokens of earlier sign-ins too, so candidates are narrowed to the
/// account in `lastKnownAccountUuid` and to unexpired entries; identical tokens count once.
/// If that still leaves several, we refuse rather than guess whose usage we would show.
fn find_token(config: &Value, decrypt: impl Fn(&[u8]) -> Option<Vec<u8>>) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let account = config.get("lastKnownAccountUuid").and_then(Value::as_str).map(str::to_ascii_lowercase).filter(|a| !a.is_empty());
    let now = crate::paths::now_millis();
    for field in ["oauth:tokenCacheV2", "oauth:tokenCache"] {
        let Some(encoded) = config.get(field).and_then(Value::as_str) else { continue };
        let Ok(data) = STANDARD.decode(encoded) else { continue };
        if !data.starts_with(b"v10") { continue; }
        let Some(plain) = decrypt(&data) else { continue };
        let Ok(value) = serde_json::from_slice::<Value>(&plain) else { continue };
        let Some(entries) = value.as_object() else { continue };
        let all: Vec<(&String, &Value, &str)> = entries.iter()
            .filter(|(k, _)| k.split_whitespace().any(|s| s.ends_with("user:inference")))
            .filter_map(|(k, v)| v.get("token").and_then(Value::as_str).filter(|t| !t.is_empty()).map(|t| (k, v, t)))
            .collect();
        if all.is_empty() { continue; }
        let mut picked = all.clone();
        if let Some(acc) = &account {
            let mine: Vec<_> = picked.iter().copied().filter(|(k, v, _)| k.to_ascii_lowercase().contains(acc.as_str()) || v.to_string().to_ascii_lowercase().contains(acc.as_str())).collect();
            if !mine.is_empty() { picked = mine; }
        }
        let live: Vec<_> = picked.iter().copied().filter(|(_, v, _)| expires_ms(v).map_or(true, |e| e > now)).collect();
        if !live.is_empty() { picked = live; }
        let mut tokens: Vec<&str> = picked.iter().map(|(_, _, t)| *t).collect();
        tokens.sort_unstable();
        tokens.dedup();
        if tokens.len() == 1 { return Ok(tokens[0].to_owned()); }
        // Several grants of one account and organization (e.g. Claude Code's extra scope) report
        // the same usage; take the one valid longest.
        let first = identity(picked[0].0);
        if picked.iter().all(|(k, _, _)| identity(k) == first) {
            if let Some((_, _, t)) = picked.iter().max_by_key(|(_, v, _)| expires_ms(v).unwrap_or(0)) { return Ok((*t).to_owned()); }
        }
        return Err(format!("账号凭据包含多个身份，无法安全确定额度归属。\n候选（不含令牌）：{}", describe(&picked, account.as_deref())));
    }
    Err("未找到可用的桌面端登录凭据，请打开该账号并确认已登录。".into())
}

/// Account + client + organization part of a cache key, i.e. everything before the scopes:
/// `acct:<account>|<client>:<org>:https://api.anthropic.com:user:inference user:profile …`.
fn identity(key: &str) -> &str {
    let head = key.split_whitespace().next().unwrap_or(key);
    head.find(":user:").map_or(head, |i| &head[..i])
}

/// Expiry of a cache entry in milliseconds, accepting seconds or milliseconds.
fn expires_ms(v: &Value) -> Option<u64> {
    let n = ["expiresAt", "expires_at", "expiry"].iter().find_map(|k| v.get(*k).and_then(Value::as_u64))?;
    Some(if n < 100_000_000_000 { n * 1000 } else { n })
}

/// Token-free shape of the remaining candidates, so an ambiguous cache can be diagnosed.
fn describe(c: &[(&String, &Value, &str)], account: Option<&str>) -> String {
    c.iter().map(|(k, v, _)| {
        let key: String = k.split(|ch: char| ch == ':' || ch == '|' || ch.is_whitespace()).map(|part| {
            let lower = part.to_ascii_lowercase();
            if account == Some(lower.as_str()) { "<当前账号>".to_string() }
            else if part.len() == 36 && part.chars().filter(|c| *c == '-').count() == 4 { "<其他ID>".to_string() }
            else { part.to_string() }
        }).collect::<Vec<_>>().join("·");
        let fields = v.as_object().map(|o| o.keys().filter(|f| *f != "token").cloned().collect::<Vec<_>>().join(",")).unwrap_or_default();
        format!("[{key} | {fields}]")
    }).collect::<Vec<_>>().join(" ")
}

/// Windows: Chromium os_crypt key protected by DPAPI, values sealed with AES-256-GCM.
#[cfg(windows)]
fn token(dir: &Path) -> Result<String, String> {
    use aes_gcm::{aead::{Aead, KeyInit}, Aes256Gcm, Nonce};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use std::ffi::c_void;
    #[repr(C)] struct Blob { len: u32, data: *mut u8 }
    #[link(name="crypt32")] extern "system" { fn CryptUnprotectData(input: *const Blob, description: *mut *mut u16, entropy: *const Blob, reserved: *mut c_void, prompt: *mut c_void, flags: u32, output: *mut Blob) -> i32; }
    #[link(name="kernel32")] extern "system" { fn LocalFree(mem: *mut c_void) -> *mut c_void; }
    let local = read_json(dir, "Local State")?;
    let config = read_json(dir, "config.json")?;
    let encoded = local.pointer("/os_crypt/encrypted_key").and_then(Value::as_str).ok_or("找不到该账号的加密密钥。")?;
    let bytes = STANDARD.decode(encoded).map_err(|_| "加密密钥格式无法识别。")?;
    let protected = bytes.strip_prefix(b"DPAPI").ok_or("该账号使用了不支持的加密格式。")?;
    let input = Blob { len: protected.len() as u32, data: protected.as_ptr() as *mut u8 };
    let mut output = Blob { len: 0, data: std::ptr::null_mut() };
    if unsafe { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), 1, &mut output) } == 0 { return Err("无法使用当前 Windows 用户解密该账号凭据。".into()); }
    let key = unsafe { let v = std::slice::from_raw_parts(output.data, output.len as usize).to_vec(); LocalFree(output.data as *mut c_void); v };
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "账号密钥格式无法识别。")?;
    find_token(&config, |data| {
        if data.len() < 31 { return None; }
        cipher.decrypt(Nonce::from_slice(&data[3..15]), &data[15..]).ok()
    })
}

/// macOS: Electron safeStorage. The password lives in the login Keychain item
/// "Claude Safe Storage" (shared by every profile of the app); values use AES-128-CBC
/// with a PBKDF2-SHA1 derived key, salt "saltysalt", 1003 rounds and a space-filled IV.
#[cfg(target_os = "macos")]
fn mac_decrypt(password: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
    let body = data.strip_prefix(b"v10")?;
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, b"saltysalt", 1003, &mut key);
    cbc::Decryptor::<aes::Aes128>::new(&key.into(), &[b' '; 16].into()).decrypt_padded_vec_mut::<Pkcs7>(body).ok()
}

#[cfg(target_os = "macos")]
fn token(dir: &Path) -> Result<String, String> {
    let config = read_json(dir, "config.json")?;
    if !["oauth:tokenCacheV2", "oauth:tokenCache"].iter().any(|k| config.get(*k).is_some()) {
        return Err("未找到可用的桌面端登录凭据，请打开该账号并确认已登录。".into());
    }
    // `security` asks the user before releasing the Keychain item to Hydra.
    let out = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-w", "-s", "Claude Safe Storage"])
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|_| "无法调用系统钥匙串。")?;
    if !out.status.success() { return Err("未获得钥匙串中「Claude Safe Storage」的访问授权，无法读取登录凭据。".into()); }
    let mut password = out.stdout;
    while password.last().is_some_and(|b| *b == b'\n' || *b == b'\r') { password.pop(); }
    find_token(&config, |data| mac_decrypt(&password, data))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn token(_dir: &Path) -> Result<String, String> { Err("当前平台暂不支持读取桌面端额度。".into()) }

pub fn fetch(profile: &crate::profiles::Profile) -> Result<Quota, String> {
    let credential = token(Path::new(&profile.path))?;
    let settings = crate::settings::load();
    let mut builder = reqwest::blocking::Client::builder().timeout(Duration::from_secs(15)).redirect(reqwest::redirect::Policy::none());
    if let Some(node) = settings.effective(&profile.id).node {
        // Same exit as the account itself; never a direct connection.
        let (node, _core) = crate::clash::resolve(&node)?;
        return query(node.client(Duration::from_secs(15))?, credential);
    }
    let vars = crate::profiles::env_vars(&settings, &profile.id);
    let proxy = ["HTTPS_PROXY", "ALL_PROXY", "HTTP_PROXY"].iter().find_map(|key| vars.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str()));
    let args = crate::profiles::extra_args(&settings, &profile.id);
    if proxy.is_none() && args.contains("--proxy-server") { return Err("该账号仅通过启动参数配置代理，额度查询无法安全确定代理地址。请改用账号的代理节点设置。".into()); }
    if let Some(p) = proxy {
        let parsed = url::Url::parse(p).map_err(|_| "账号代理地址无效。")?;
        if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h") { return Err("账号代理类型不受支持。".into()); }
        builder = builder.no_proxy().proxy(reqwest::Proxy::all(p).map_err(|_| "账号代理地址无效。")?);
    }
    query(builder.build().map_err(|_| "无法初始化网络连接。")?, credential)
}

fn query(client: reqwest::blocking::Client, credential: String) -> Result<Quota, String> {
    let response = client.get("https://api.anthropic.com/api/oauth/usage").bearer_auth(credential).header("anthropic-beta", "oauth-2025-04-20").send().map_err(|_| "额度查询连接失败，请检查账号代理和网络。")?;
    let status = response.status();
    if !status.is_success() { return Err(match status.as_u16() { 401 | 403 => "登录凭据已过期或没有查询权限；请在 Claude 中重新登录。".into(), 429 => "额度服务暂时限流，请稍后再试。".into(), _ => format!("额度服务返回错误（HTTP {}）。", status.as_u16()) }); }
    let body: Value = response.json().map_err(|_| "额度服务返回了无法识别的数据。")?;
    parse_quota(&body)
}

#[cfg(test)] mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    #[test] fn picks_single_inference_token_and_rejects_ambiguity() {
        let seal = |v: Value| STANDARD.encode([b"v10".as_slice(), v.to_string().as_bytes()].concat());
        let open = |d: &[u8]| Some(d[3..].to_vec());
        let one = serde_json::json!({"oauth:tokenCacheV2": seal(serde_json::json!({"a user:inference user:profile":{"token":"t1"},"b user:profile":{"token":"x"}}))});
        assert_eq!(find_token(&one, open).unwrap(), "t1");
        let two = serde_json::json!({"oauth:tokenCache": seal(serde_json::json!({"a user:inference":{"token":"t1"},"b user:inference":{"token":"t2"}}))});
        assert!(find_token(&two, open).is_err());
        assert!(find_token(&serde_json::json!({}), open).is_err());
    }

    #[test] fn narrows_multiple_identities_to_current_live_account() {
        let seal = |v: Value| STANDARD.encode([b"v10".as_slice(), v.to_string().as_bytes()].concat());
        let open = |d: &[u8]| Some(d[3..].to_vec());
        let me = "11111111-2222-3333-4444-555555555555";
        let other = "99999999-8888-7777-6666-555555555555";
        let far = 4_000_000_000_000u64;
        // Old sign-in of another account plus the current one: pick the current account.
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            format!("cid:{other}:user:inference"): {"token":"old","expiresAt":far},
            format!("cid:{me}:user:inference"): {"token":"mine","expiresAt":far}}))});
        assert_eq!(find_token(&c, open).unwrap(), "mine");
        // Same account, one expired entry: pick the live one (expiry in seconds is accepted too).
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            format!("a:{me}:user:inference"): {"token":"stale","expiresAt":1_000},
            format!("b:{me}:user:inference"): {"token":"live","expiresAt":4_000_000_000u64}}))});
        assert_eq!(find_token(&c, open).unwrap(), "live");
        // Duplicates of one token are not ambiguous.
        let c = serde_json::json!({"oauth:tokenCacheV2": seal(serde_json::json!({
            "a user:inference": {"token":"same"}, "b user:inference user:profile": {"token":"same"}}))});
        assert_eq!(find_token(&c, open).unwrap(), "same");
        // Still ambiguous: refuse, and never leak tokens into the message.
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            format!("a:{me}:user:inference"): {"token":"secret-1","expiresAt":far},
            format!("b:{me}:user:inference"): {"token":"secret-2","expiresAt":far}}))});
        let err = find_token(&c, open).unwrap_err();
        assert!(err.contains("<当前账号>") && err.contains("expiresAt") && !err.contains("secret"));
    }

    #[test] fn same_account_and_org_with_extra_scope_is_one_identity() {
        let seal = |v: Value| STANDARD.encode([b"v10".as_slice(), v.to_string().as_bytes()].concat());
        let open = |d: &[u8]| Some(d[3..].to_vec());
        let me = "789f2b9b-15d6-49dd-ab6d-030c2f759751";
        let key = |org: &str, scopes: &str| format!("acct:{me}|9d1c250a-e61b-44d9-88ed-5944d1962f5e:{org}:https://api.anthropic.com:{scopes}");
        let org = "766c0284-b271-4b5d-aa58-031ed7724bee";
        // Real Desktop layout: the plain grant plus Claude Code's grant with an extra scope.
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            key(org, "user:inference user:file_upload user:profile user:sessions:claude_code"): {"token":"code","expiresAt":4_000_000_000_001u64},
            key(org, "user:inference user:file_upload user:profile"): {"token":"desktop","expiresAt":4_000_000_000_000u64}}))});
        assert_eq!(find_token(&c, open).unwrap(), "code");
        // Same account in two organizations has separate usage: refuse.
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            key(org, "user:inference"): {"token":"t1","expiresAt":4_000_000_000_000u64},
            key("00000000-0000-0000-0000-000000000000", "user:inference"): {"token":"t2","expiresAt":4_000_000_000_000u64}}))});
        let err = find_token(&c, open).unwrap_err();
        assert!(err.contains("<当前账号>") && !err.contains("t1") && !err.contains("t2"));
    }

    #[cfg(target_os = "macos")]
    #[test] fn mac_safe_storage_round_trip() {
        use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        let mut key = [0u8; 16];
        pbkdf2::pbkdf2_hmac::<sha1::Sha1>(b"secret", b"saltysalt", 1003, &mut key);
        let sealed = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &[b' '; 16].into()).encrypt_padded_vec_mut::<Pkcs7>(b"{\"k\":1}");
        let blob = [b"v10".as_slice(), &sealed].concat();
        assert_eq!(mac_decrypt(b"secret", &blob).unwrap(), b"{\"k\":1}");
        assert!(mac_decrypt(b"wrong", &blob).map_or(true, |p| p != b"{\"k\":1}"));
    }

    #[test] fn parse_preserves_missing_windows() {
        let q = parse_quota(&serde_json::json!({"five_hour":{"utilization":23.5,"resets_at":"2026-01-01T00:00:00Z"},"seven_day":null})).unwrap();
        assert_eq!(q.five_hour.unwrap().utilization, 23.5); assert!(q.seven_day.is_none());
        assert!(parse_quota(&serde_json::json!({"five_hour":{"utilization":999}})).is_err());
    }
}
