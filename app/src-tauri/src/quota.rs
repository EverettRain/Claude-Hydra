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

/// The chosen credential and when it expires (milliseconds), if the cache says.
#[derive(Debug)]
struct Found { token: String, expires: Option<u64> }

/// Finds the desktop OAuth token of the signed-in account in Electron's encrypted token cache.
/// `decrypt` receives the raw `v10`-prefixed blob and returns plaintext JSON bytes.
///
/// The cache can hold tokens of earlier sign-ins too, so candidates are narrowed to the
/// account in `lastKnownAccountUuid`, to the organization currently selected in Claude
/// (`org`, from the `lastActiveOrg` cookie) and to unexpired entries; identical tokens count
/// once, and grants of one account+organization are interchangeable. If that still leaves
/// several, we refuse rather than guess whose usage we would show.
fn find_token(config: &Value, org: Option<&str>, decrypt: impl Fn(&[u8]) -> Option<Vec<u8>>) -> Result<Found, String> {
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
        if let Some(org) = org.map(str::to_ascii_lowercase) {
            let current: Vec<_> = picked.iter().copied().filter(|(k, _, _)| identity(k).to_ascii_lowercase().contains(org.as_str())).collect();
            if !current.is_empty() { picked = current; }
        }
        let live: Vec<_> = picked.iter().copied().filter(|(_, v, _)| expires_ms(v).map_or(true, |e| e > now)).collect();
        if !live.is_empty() { picked = live; }
        let found = |(_, v, t): &(&String, &Value, &str)| Found { token: (*t).to_owned(), expires: expires_ms(v) };
        let mut tokens: Vec<&str> = picked.iter().map(|(_, _, t)| *t).collect();
        tokens.sort_unstable();
        tokens.dedup();
        if tokens.len() == 1 { return Ok(found(&picked[0])); }
        // Several grants of one account and organization (e.g. Claude Code's extra scope) report
        // the same usage; take the one valid longest.
        let first = identity(picked[0].0);
        if picked.iter().all(|(k, _, _)| identity(k) == first) {
            if let Some(best) = picked.iter().max_by_key(|(_, v, _)| expires_ms(v).unwrap_or(0)) { return Ok(found(best)); }
        }
        let why = if org.is_none() { "该账号缓存了多个组织的凭据，且无法读取 Claude 当前选择的组织" } else { "账号凭据包含多个身份" };
        return Err(format!("{why}，无法安全确定额度归属。\n候选（不含令牌）：{}", describe(&picked, account.as_deref())));
    }
    Err("未找到可用的桌面端登录凭据，请打开该账号并确认已登录。".into())
}

/// The organization currently selected in Claude, from claude.ai's `lastActiveOrg` cookie.
/// Cookie values are sealed like the token cache (`decrypt`); newer Chromium prefixes the
/// plaintext with SHA-256(host), so the id is taken as the trailing UUID.
fn active_org(dir: &Path, decrypt: &impl Fn(&[u8]) -> Option<Vec<u8>>) -> Option<String> {
    let db = ["Network/Cookies", "Cookies"].iter().map(|p| dir.join(p)).find(|p| p.is_file())?;
    let blob = read_cookie(&db).or_else(|| {
        // Locked by the running Claude: read a private copy instead.
        let copy = std::env::temp_dir().join(format!("hydra-cookies-{}", uuid::Uuid::new_v4()));
        let out = std::fs::copy(&db, &copy).ok().and_then(|_| read_cookie(&copy));
        let _ = std::fs::remove_file(&copy);
        out
    })?;
    let plain = decrypt(&blob)?;
    let id = std::str::from_utf8(plain.get(plain.len().checked_sub(36)?..)?).ok()?;
    is_uuid(id).then(|| id.to_ascii_lowercase())
}

fn read_cookie(db: &Path) -> Option<Vec<u8>> {
    use rusqlite::{Connection, OpenFlags};
    // immutable=1: read without taking locks, so a running Claude is never disturbed.
    let uri = format!("{}?immutable=1", url::Url::from_file_path(db).ok()?);
    let conn = Connection::open_with_flags(uri, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    conn.query_row(
        "SELECT encrypted_value FROM cookies WHERE host_key IN ('.claude.ai','claude.ai') AND name = 'lastActiveOrg' ORDER BY last_access_utc DESC LIMIT 1",
        [], |r| r.get::<_, Vec<u8>>(0)).ok().filter(|b| !b.is_empty())
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
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
            else if is_uuid(part) { "<其他ID>".to_string() }
            else { part.to_string() }
        }).collect::<Vec<_>>().join("·");
        let fields = v.as_object().map(|o| o.keys().filter(|f| *f != "token").cloned().collect::<Vec<_>>().join(",")).unwrap_or_default();
        format!("[{key} | {fields}]")
    }).collect::<Vec<_>>().join(" ")
}

/// Windows: Chromium os_crypt key protected by DPAPI, values sealed with AES-256-GCM.
#[cfg(windows)]
fn token(dir: &Path) -> Result<Found, String> {
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
    let decrypt = |data: &[u8]| {
        if data.len() < 31 { return None; }
        cipher.decrypt(Nonce::from_slice(&data[3..15]), &data[15..]).ok()
    };
    let org = active_org(dir, &decrypt);
    find_token(&config, org.as_deref(), decrypt)
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
fn token(dir: &Path) -> Result<Found, String> {
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
    let decrypt = |data: &[u8]| mac_decrypt(&password, data);
    let org = active_org(dir, &decrypt);
    find_token(&config, org.as_deref(), decrypt)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn token(_dir: &Path) -> Result<Found, String> { Err("当前平台暂不支持读取桌面端额度。".into()) }

pub fn fetch(profile: &crate::profiles::Profile) -> Result<Quota, String> {
    let found = token(Path::new(&profile.path))?;
    // Hydra only reads credentials; refreshing them is Claude's job while it runs.
    if found.expires.is_some_and(|e| e <= crate::paths::now_millis()) {
        return Err("该账号的登录凭据已过期。Hydra 不会自行刷新凭据：打开一次该账号的 Claude，等它自动刷新后再点“刷新”。".into());
    }
    let credential = found.token;
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
    if !status.is_success() { return Err(match status.as_u16() { 401 | 403 => "登录凭据已失效。打开一次该账号的 Claude 让它刷新凭据；如果仍然失败，请在 Claude 中重新登录。".into(), 429 => "额度服务暂时限流，请稍后再试。".into(), _ => format!("额度服务返回错误（HTTP {}）。", status.as_u16()) }); }
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
        assert_eq!(find_token(&one, None, open).unwrap().token, "t1");
        let two = serde_json::json!({"oauth:tokenCache": seal(serde_json::json!({"a user:inference":{"token":"t1"},"b user:inference":{"token":"t2"}}))});
        assert!(find_token(&two, None, open).is_err());
        assert!(find_token(&serde_json::json!({}), None, open).is_err());
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
        assert_eq!(find_token(&c, None, open).unwrap().token, "mine");
        // Same account, one expired entry: pick the live one (expiry in seconds is accepted too).
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            format!("a:{me}:user:inference"): {"token":"stale","expiresAt":1_000},
            format!("b:{me}:user:inference"): {"token":"live","expiresAt":4_000_000_000u64}}))});
        assert_eq!(find_token(&c, None, open).unwrap().token, "live");
        // Duplicates of one token are not ambiguous.
        let c = serde_json::json!({"oauth:tokenCacheV2": seal(serde_json::json!({
            "a user:inference": {"token":"same"}, "b user:inference user:profile": {"token":"same"}}))});
        assert_eq!(find_token(&c, None, open).unwrap().token, "same");
        // Still ambiguous: refuse, and never leak tokens into the message.
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            format!("a:{me}:user:inference"): {"token":"secret-1","expiresAt":far},
            format!("b:{me}:user:inference"): {"token":"secret-2","expiresAt":far}}))});
        let err = find_token(&c, None, open).unwrap_err();
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
        assert_eq!(find_token(&c, None, open).unwrap().token, "code");
        // Same account in two organizations has separate usage: refuse.
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            key(org, "user:inference"): {"token":"t1","expiresAt":4_000_000_000_000u64},
            key("00000000-0000-0000-0000-000000000000", "user:inference"): {"token":"t2","expiresAt":4_000_000_000_000u64}}))});
        let err = find_token(&c, None, open).unwrap_err();
        assert!(err.contains("<当前账号>") && !err.contains("t1") && !err.contains("t2"));
    }

    #[test] fn current_org_cookie_picks_between_organizations() {
        let seal = |v: Value| STANDARD.encode([b"v10".as_slice(), v.to_string().as_bytes()].concat());
        let open = |d: &[u8]| Some(d[3..].to_vec());
        let me = "789f2b9b-15d6-49dd-ab6d-030c2f759751";
        let (org_a, org_b) = ("766c0284-b271-4b5d-aa58-031ed7724bee", "75536ba4-ef49-46fe-bc3a-10d101225d91");
        let key = |org: &str| format!("acct:{me}|9d1c250a-e61b-44d9-88ed-5944d1962f5e:{org}:https://api.anthropic.com:user:inference");
        let c = serde_json::json!({"lastKnownAccountUuid": me, "oauth:tokenCacheV2": seal(serde_json::json!({
            key(org_a): {"token":"a","expiresAt":4_000_000_000_000u64},
            key(org_b): {"token":"b","expiresAt":4_000_000_000_500u64}}))});
        let picked = find_token(&c, Some(&org_a.to_uppercase()), open).unwrap();
        assert_eq!((picked.token.as_str(), picked.expires), ("a", Some(4_000_000_000_000)));
        assert_eq!(find_token(&c, Some(org_b), open).unwrap().token, "b");
        // Without the cookie, two organizations stay ambiguous.
        assert!(find_token(&c, None, open).unwrap_err().contains("多个组织"));
    }

    #[test] fn reads_last_active_org_cookie() {
        let dir = std::env::temp_dir().join(format!("hydra quota test {}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("Network")).unwrap();
        let org = "766c0284-b271-4b5d-aa58-031ed7724bee";
        // Chromium DB v24+: SHA-256(host) prefix, then the value; sealed with a "v10" header.
        let sealed = [b"v10".as_slice(), &[7u8; 32], org.as_bytes()].concat();
        let conn = rusqlite::Connection::open(dir.join("Network/Cookies")).unwrap();
        conn.execute_batch("CREATE TABLE cookies(host_key TEXT, name TEXT, encrypted_value BLOB, last_access_utc INTEGER);").unwrap();
        conn.execute("INSERT INTO cookies VALUES ('.claude.ai','lastActiveOrg',?1,2), ('.example.com','lastActiveOrg',x'00',3)", [&sealed]).unwrap();
        drop(conn);
        let open = |d: &[u8]| Some(d[3..].to_vec());
        assert_eq!(active_org(&dir, &open).as_deref(), Some(org));
        assert_eq!(active_org(&dir, &|_: &[u8]| None), None);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!is_uuid("not-a-uuid") && is_uuid(org));
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
