//! macOS plumbing for routing `claude://` sign-in callbacks: the LaunchServices default
//! handler for a URL scheme, and delivering a URL to one specific process as a `GURL`
//! Apple event (the same event LaunchServices sends, so Electron's `open-url` fires).
#![cfg(target_os = "macos")]
use std::ffi::c_void;

type CFStringRef = *const c_void;
type OSStatus = i32;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithBytes(alloc: *const c_void, bytes: *const u8, len: isize, encoding: u32, external: u8) -> CFStringRef;
    fn CFStringGetCString(s: CFStringRef, buf: *mut u8, size: isize, encoding: u32) -> u8;
    fn CFRelease(cf: *const c_void);
    fn CFBundleGetMainBundle() -> *const c_void;
    fn CFBundleGetIdentifier(bundle: *const c_void) -> CFStringRef;
}

/// Apple event descriptor. The C header declares it under `#pragma pack(push, 2)`.
#[repr(C, packed(2))]
struct AEDesc { descriptor_type: u32, data_handle: *mut c_void }

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn LSCopyDefaultHandlerForURLScheme(scheme: CFStringRef) -> CFStringRef;
    fn LSSetDefaultHandlerForURLScheme(scheme: CFStringRef, bundle_id: CFStringRef) -> OSStatus;
    fn AECreateDesc(kind: u32, data: *const c_void, size: isize, out: *mut AEDesc) -> i16;
    fn AECreateAppleEvent(class: u32, id: u32, target: *const AEDesc, return_id: i16, transaction: i32, out: *mut AEDesc) -> i16;
    fn AEPutParamPtr(event: *mut AEDesc, key: u32, kind: u32, data: *const c_void, size: isize) -> i16;
    fn AESendMessage(event: *const AEDesc, reply: *mut AEDesc, mode: i32, timeout_ticks: i64) -> OSStatus;
    fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
}

const UTF8: u32 = 0x0800_0100;
const fn code(s: &[u8; 4]) -> u32 { u32::from_be_bytes(*s) }

struct CfString(CFStringRef);
impl CfString {
    fn new(s: &str) -> Option<Self> {
        let r = unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0) };
        (!r.is_null()).then_some(Self(r))
    }
}
impl Drop for CfString {
    fn drop(&mut self) { unsafe { CFRelease(self.0) } }
}

fn to_string(s: CFStringRef) -> Option<String> {
    if s.is_null() { return None; }
    let mut buf = vec![0u8; 1024];
    let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) } != 0;
    ok.then(|| buf.iter().position(|b| *b == 0).map(|n| String::from_utf8_lossy(&buf[..n]).into_owned())).flatten()
}

/// Bundle id of the app currently registered as default handler for `scheme`.
pub fn default_handler(scheme: &str) -> Option<String> {
    let s = CfString::new(scheme)?;
    let r = unsafe { LSCopyDefaultHandlerForURLScheme(s.0) };
    let out = to_string(r);
    if !r.is_null() { unsafe { CFRelease(r) } }
    out
}

pub fn set_default_handler(scheme: &str, bundle_id: &str) -> Result<(), String> {
    let (s, b) = (CfString::new(scheme).ok_or("内部错误")?, CfString::new(bundle_id).ok_or("内部错误")?);
    match unsafe { LSSetDefaultHandlerForURLScheme(s.0, b.0) } {
        0 => Ok(()),
        e => Err(format!("无法设置 {scheme}:// 链接的处理程序（{e}）。")),
    }
}

/// Our own bundle id; `None` when not running from an app bundle (e.g. `cargo run`).
pub fn own_bundle_id() -> Option<String> {
    let bundle = unsafe { CFBundleGetMainBundle() };
    if bundle.is_null() { return None; }
    // Not owned (Get rule): must not be released.
    to_string(unsafe { CFBundleGetIdentifier(bundle) })
}

/// Sends `url` to process `pid` as a `GURL` Apple event.
pub fn open_url_in(pid: u32, url: &str) -> Result<(), String> {
    let mut target = AEDesc { descriptor_type: 0, data_handle: std::ptr::null_mut() };
    let mut event = AEDesc { descriptor_type: 0, data_handle: std::ptr::null_mut() };
    let pid = pid as i32;
    unsafe {
        let mut err = AECreateDesc(code(b"kpid"), &pid as *const i32 as *const c_void, 4, &mut target) as i32;
        if err == 0 {
            // kAutoGenerateReturnID = -1, kAnyTransactionID = 0
            err = AECreateAppleEvent(code(b"GURL"), code(b"GURL"), &target, -1, 0, &mut event) as i32;
        }
        if err == 0 {
            err = AEPutParamPtr(&mut event, code(b"----"), code(b"utf8"), url.as_ptr() as *const c_void, url.len() as isize) as i32;
        }
        if err == 0 {
            // kAENoReply | kAECanInteract, so the one-time Automation consent prompt can show.
            err = AESendMessage(&event, std::ptr::null_mut(), 0x1 | 0x20, -1);
        }
        AEDisposeDesc(&mut event);
        AEDisposeDesc(&mut target);
        match err {
            0 => Ok(()),
            -1743 => Err("macOS 未允许 Claude-Hydra 向 Claude 传递登录结果。请在“系统设置 → 隐私与安全性 → 自动化”中允许 Claude-Hydra 控制 Claude，然后重新登录。".into()),
            -600 => Err("登录目标账号的 Claude 已退出，请重新选择登录。".into()),
            e => Err(format!("无法把登录结果交给 Claude（{e}）。")),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn aedesc_matches_c_layout() {
        // DescType (4) + pointer (8) packed to 2-byte alignment.
        assert_eq!(std::mem::size_of::<super::AEDesc>(), 12);
        assert_eq!(super::code(b"GURL"), 0x4755_524C);
    }

    #[test]
    fn reads_a_default_handler() {
        // Every macOS install has an https handler; proves the CF/LS round trip works.
        assert!(super::default_handler("https").is_some_and(|b| b.contains('.')));
    }
}
