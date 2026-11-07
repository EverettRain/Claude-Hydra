#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = claude_hydra_lib::cli::run(&args) {
        std::process::exit(code);
    }
    claude_hydra_lib::run(args.iter().any(|a| a == "--tray"));
}
