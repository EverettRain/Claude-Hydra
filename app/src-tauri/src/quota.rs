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

/// Finds the single desktop OAuth token in Electron's encrypted token cache.
/// `decrypt` receives the raw `v10`-prefixed blob and returns plaintext JSON bytes.
fn find_token(config: &Value, decrypt: impl Fn(&[u8]) -> Option<Vec<u8>>) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    for field in ["oauth:tokenCacheV2", "oauth:tokenCache"] {
        let Some(encoded) = config.get(field).and_then(Value::as_str) else { continue };
        let Ok(data) = STANDARD.decode(encoded) else { continue };
        if !data.starts_with(b"v10") { continue; }
        let Some(plain) = decrypt(&data) else { continue };
        let Ok(value) = serde_json::from_slice::<Value>(&plain) else { continue };
        let Some(entries) = value.as_object() else { continue };
        let candidates: Vec<&str> = entries.iter().filter(|(k, _)| k.split_whitespace().any(|s| s.ends_with("user:inference")))
            .filter_map(|(_, v)| v.get("token").and_then(Value::as_str)).filter(|t| !t.is_empty()).collect();
        if candidates.len() == 1 { return Ok(candidates[0].to_owned()); }
        if candidates.len() > 1 { return Err("账号凭据包含多个身份，无法安全确定额度归属。".into()); }
    }
    Err("未找到可用的桌面端登录凭据，请打开该账号并确认已登录。".into())
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
