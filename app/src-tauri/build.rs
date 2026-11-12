fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=src/taskbar.cpp");
        cc::Build::new().cpp(true).file("src/taskbar.cpp").compile("hydra_taskbar");
        println!("cargo:rerun-if-changed=src/taskbar.cpp");
        for lib in ["propsys", "shell32", "ole32", "uuid", "user32"] {
            println!("cargo:rustc-link-lib={lib}");
        }
        build_dns_guard();
    }
    tauri_build::build();
    println!("cargo:rerun-if-changed=icons");

    // cargo test binaries do not get Tauri's manifest; without Common Controls v6 they
    // fail to start. Embed it for tests and leave the app binary to Tauri.
    #[cfg(all(target_os = "windows", target_env = "msvc"))]
    {
        let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("common-controls.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
        println!("cargo:rerun-if-changed={}", manifest.display());
    }
}

/// Builds `src/dnsguard.c` into the DLL that sandboxed accounts load (see `sandbox.rs`) and
/// passes its path as `HYDRA_DNSGUARD`. 64-bit MSVC targets only; elsewhere it stays unset.
fn build_dns_guard() {
    println!("cargo:rerun-if-changed=src/dnsguard.c");
    let msvc_x64 = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
        && std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86_64");
    if !msvc_x64 { return; }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let dll = out.join("hydra_dnsguard.dll");
    let compiler = cc::Build::new().get_compiler();
    let status = compiler
        .to_command()
        .current_dir(&out)
        .args(["/nologo", "/O1", "/MT", "/GS-", "/W3", "/LD"])
        .arg(std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src").join("dnsguard.c"))
        .arg(format!("/Fe{}", dll.display()))
        .args(["/link", "/NOLOGO", "kernel32.lib"])
        .status()
        .expect("failed to run the C compiler for dnsguard.c");
    assert!(status.success(), "building dnsguard.c failed");
    println!("cargo:rustc-env=HYDRA_DNSGUARD={}", dll.display());
}
