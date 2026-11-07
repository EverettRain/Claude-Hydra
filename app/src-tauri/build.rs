fn main() {
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
