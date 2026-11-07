//! Platform-specific process inspection, window management, autostart, and shortcut creation.

#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use super::Proc;
    use std::ffi::c_void;
    use std::path::Path;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{
        CloseHandle, BOOL, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        MessageBoxW, PostMessageW, SetForegroundWindow, ShowWindow, GW_OWNER, MB_ICONERROR, MB_OK, SW_RESTORE,
        WM_CLOSE,
    };

    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(h: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    }

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(pv_reserved: *const c_void, dw_co_init: u32) -> i32;
        fn CoCreateInstance(
            rclsid: *const Guid,
            p_unk_outer: *mut c_void,
            dw_cls_context: u32,
            riid: *const Guid,
            ppv: *mut *mut c_void,
        ) -> i32;
    }

    const CLSID_SHELL_LINK: Guid = Guid {
        data1: 0x0002_1401,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    const IID_ISHELL_LINK_W: Guid = Guid {
        data1: 0x0002_14F9,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    const IID_IPERSIST_FILE: Guid = Guid {
        data1: 0x0000_010B,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    #[repr(C)]
    struct IShellLinkWVtbl {
        query_interface: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_path: *const c_void,
        get_id_list: *const c_void,
        set_id_list: *const c_void,
        get_description: *const c_void,
        set_description: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        get_working_directory: *const c_void,
        set_working_directory: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        get_arguments: *const c_void,
        set_arguments: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
        get_hotkey: *const c_void,
        set_hotkey: *const c_void,
        get_show_cmd: *const c_void,
        set_show_cmd: *const c_void,
        get_icon_location: *const c_void,
        set_icon_location: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
        set_relative_path: *const c_void,
        resolve: *const c_void,
        set_path: unsafe extern "system" fn(*mut c_void, *const u16) -> i32,
    }

    #[repr(C)]
    struct IPersistFileVtbl {
        query_interface: *const c_void,
        add_ref: *const c_void,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_class_id: *const c_void,
        is_dirty: *const c_void,
        load: *const c_void,
        save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32,
        save_completed: *const c_void,
        get_cur_file: *const c_void,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(pid: u32, access: u32) -> Option<Handle> {
        let h = unsafe { OpenProcess(access, 0, pid) };
        (!h.is_null()).then_some(Handle(h))
    }

    pub fn processes() -> Vec<Proc> {
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let _guard = Handle(snap);
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            if Process32FirstW(snap, &mut e) != 0 {
                loop {
                    let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                    out.push(Proc {
                        pid: e.th32ProcessID,
                        ppid: e.th32ParentProcessID,
                        name: String::from_utf16_lossy(&e.szExeFile[..len]),
                    });
                    if Process32NextW(snap, &mut e) == 0 {
                        break;
                    }
                }
            }
        }
        out
    }

    pub fn image_path(pid: u32) -> Option<String> {
        let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let mut buf = vec![0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) };
        (ok != 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }

    pub fn command_line(pid: u32) -> Option<String> {
        #[repr(C)]
        struct UnicodeString {
            length: u16,
            maximum: u16,
            buffer: *const u16,
        }
        let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        unsafe {
            let mut needed = 0u32;
            NtQueryInformationProcess(h.0, 60, null_mut(), 0, &mut needed);
            if needed == 0 || needed > 1 << 20 {
                return None;
            }
            let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
            if NtQueryInformationProcess(h.0, 60, buf.as_mut_ptr().cast(), needed, &mut needed) < 0 {
                return None;
            }
            let us = &*(buf.as_ptr() as *const UnicodeString);
            if us.buffer.is_null() {
                return Some(String::new());
            }
            let s = std::slice::from_raw_parts(us.buffer, us.length as usize / 2);
            Some(String::from_utf16_lossy(s))
        }
    }

    pub fn is_alive(pid: u32) -> bool {
        let Some(h) = open(pid, PROCESS_QUERY_LIMITED_INFORMATION) else { return false };
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(h.0, &mut code) != 0 && code == 259 }
    }

    pub fn top_windows(pid: u32) -> Vec<HWND> {
        struct Search {
            pid: u32,
            found: Vec<HWND>,
        }
        unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
            let s = &mut *(l as *mut Search);
            let mut p = 0u32;
            GetWindowThreadProcessId(h, &mut p);
            if p == s.pid && IsWindowVisible(h) != 0 && GetWindow(h, GW_OWNER).is_null() && GetWindowTextLengthW(h) > 0 {
                s.found.push(h);
            }
            1
        }
        let mut s = Search { pid, found: Vec::new() };
        unsafe { EnumWindows(Some(each), &mut s as *mut Search as LPARAM) };
        s.found
    }

    pub fn focus(pid: u32) -> bool {
        let Some(&h) = top_windows(pid).first() else { return false };
        unsafe {
            if IsIconic(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            SetForegroundWindow(h);
        }
        true
    }

    pub fn close_windows(pid: u32) {
        for h in top_windows(pid) {
            unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
        }
    }

    pub fn wait_exit(pid: u32, ms: u32) -> bool {
        match open(pid, PROCESS_SYNCHRONIZE) {
            Some(h) => unsafe { WaitForSingleObject(h.0, ms) == WAIT_OBJECT_0 },
            None => !is_alive(pid),
        }
    }

    pub fn terminate(pid: u32) -> bool {
        match open(pid, PROCESS_TERMINATE) {
            Some(h) => unsafe { TerminateProcess(h.0, 1) != 0 },
            None => false,
        }
    }

    pub fn attach_console() -> bool {
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
    }

    pub fn error_box(text: &str) {
        let t = wide(text);
        let c = wide("Claude-Hydra");
        unsafe { MessageBoxW(null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONERROR) };
    }

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const RUN_VALUE: &str = "Claude Hydra";

    pub fn autostart_enabled() -> bool {
        let (k, v) = (wide(RUN_KEY), wide(RUN_VALUE));
        let mut size = 0u32;
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                k.as_ptr(),
                v.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                null_mut(),
                &mut size,
            ) == 0
        }
    }

    pub fn set_autostart(command: Option<&str>) -> Result<(), String> {
        let (k, v) = (wide(RUN_KEY), wide(RUN_VALUE));
        let rc = unsafe {
            match command {
                Some(cmd) => {
                    let data = wide(cmd);
                    RegSetKeyValueW(
                        HKEY_CURRENT_USER,
                        k.as_ptr(),
                        v.as_ptr(),
                        REG_SZ,
                        data.as_ptr().cast(),
                        (data.len() * 2) as u32,
                    )
                }
                None => {
                    let rc = RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr());
                    if rc == 2 { 0 } else { rc }
                }
            }
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("无法写入开机启动项（错误 {rc}）"))
        }
    }

    /// Creates a Windows `.lnk` shortcut directly via COM (`IShellLinkW` + `IPersistFile`).
    pub fn write_shortcut(lnk_path: &Path, target_exe: &Path, arguments: &str, description: &str) -> Result<(), String> {
        unsafe {
            let _ = CoInitializeEx(null_mut(), 0x2); // COINIT_APARTMENTTHREADED
            let mut shell_link: *mut c_void = null_mut();
            let hr = CoCreateInstance(
                &CLSID_SHELL_LINK,
                null_mut(),
                1, // CLSCTX_INPROC_SERVER
                &IID_ISHELL_LINK_W,
                &mut shell_link,
            );
            if hr < 0 || shell_link.is_null() {
                return Err(format!("无法初始化快捷方式组件（HRESULT {hr:#x}）"));
            }
            let sl_vtbl = &**(shell_link as *mut *const IShellLinkWVtbl);
            let target_w = wide(&target_exe.to_string_lossy());
            let args_w = wide(arguments);
            let desc_w = wide(description);
            let _ = (sl_vtbl.set_path)(shell_link, target_w.as_ptr());
            let _ = (sl_vtbl.set_arguments)(shell_link, args_w.as_ptr());
            let _ = (sl_vtbl.set_description)(shell_link, desc_w.as_ptr());
            let _ = (sl_vtbl.set_icon_location)(shell_link, target_w.as_ptr(), 0);
            if let Some(dir) = target_exe.parent() {
                let dir_w = wide(&dir.to_string_lossy());
                let _ = (sl_vtbl.set_working_directory)(shell_link, dir_w.as_ptr());
            }

            let mut persist_file: *mut c_void = null_mut();
            let qhr = (sl_vtbl.query_interface)(shell_link, &IID_IPERSIST_FILE, &mut persist_file);
            if qhr < 0 || persist_file.is_null() {
                let _ = (sl_vtbl.release)(shell_link);
                return Err(format!("无法获取快捷方式保存接口（HRESULT {qhr:#x}）"));
            }
            let pf_vtbl = &**(persist_file as *mut *const IPersistFileVtbl);
            let lnk_w = wide(&lnk_path.to_string_lossy());
            let shr = (pf_vtbl.save)(persist_file, lnk_w.as_ptr(), 1);
            let _ = (pf_vtbl.release)(persist_file);
            let _ = (sl_vtbl.release)(shell_link);
            if shr < 0 {
                return Err(format!("保存快捷方式失败（HRESULT {shr:#x}）"));
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn creates_valid_lnk_shortcut_file() {
            let dir = std::env::temp_dir().join(format!("hydra-lnk-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let lnk = dir.join("Claude - Work.lnk");
            let exe = std::env::current_exe().unwrap();
            write_shortcut(&lnk, &exe, "--launch \"Work\"", "Launch Work").unwrap();
            assert!(lnk.is_file());
            assert!(std::fs::metadata(&lnk).unwrap().len() > 64);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(not(windows))]
pub use unix::*;

#[cfg(not(windows))]
mod unix {
    use super::Proc;
    use std::path::Path;
    use std::process::Command;

    fn ps(args: &[&str]) -> String {
        Command::new("/bin/ps")
            .args(args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default()
    }

    pub fn processes() -> Vec<Proc> {
        ps(&["-axo", "pid=,ppid=,comm="])
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let pid = it.next()?.parse().ok()?;
                let ppid = it.next()?.parse().ok()?;
                let comm: Vec<&str> = it.collect();
                let full = comm.join(" ");
                let name = full.rsplit('/').next().unwrap_or(&full).to_string();
                Some(Proc { pid, ppid, name })
            })
            .collect()
    }

    pub fn image_path(pid: u32) -> Option<String> {
        command_line(pid)
    }

    pub fn command_line(pid: u32) -> Option<String> {
        let s = ps(&["-o", "command=", "-p", &pid.to_string()]);
        let s = s.trim();
        (!s.is_empty()).then(|| s.to_string())
    }

    pub fn is_alive(pid: u32) -> bool {
        Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn focus(pid: u32) -> bool {
        #[cfg(target_os = "macos")]
        {
            let script = format!(
                "tell application \"System Events\" to set frontmost of (first process whose unix id is {pid}) to true"
            );
            return Command::new("/usr/bin/osascript")
                .args(["-e", &script])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
        }
        #[allow(unreachable_code)]
        {
            let _ = pid;
            false
        }
    }

    pub fn close_windows(pid: u32) {
        let _ = Command::new("/bin/kill").args(["-TERM", &pid.to_string()]).status();
    }

    pub fn wait_exit(pid: u32, ms: u32) -> bool {
        let until = std::time::Instant::now() + std::time::Duration::from_millis(ms as u64);
        while std::time::Instant::now() < until {
            if !is_alive(pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        !is_alive(pid)
    }

    pub fn terminate(pid: u32) -> bool {
        Command::new("/bin/kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn attach_console() -> bool {
        true
    }

    pub fn error_box(text: &str) {
        eprintln!("{text}");
    }

    pub fn autostart_enabled() -> bool {
        false
    }

    pub fn set_autostart(_command: Option<&str>) -> Result<(), String> {
        Err("此平台请在系统设置的“登录项”中添加 Claude-Hydra。".into())
    }

    pub fn write_shortcut(lnk_path: &Path, target_exe: &Path, arguments: &str, _description: &str) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let script = format!("#!/bin/sh\nexec \"{}\" {}\n", target_exe.display(), arguments);
        std::fs::write(lnk_path, script).map_err(|e| format!("无法写入快捷脚本：{e}"))?;
        std::fs::set_permissions(lnk_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("无法设置快捷脚本权限：{e}"))
    }
}
