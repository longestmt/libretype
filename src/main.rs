// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
mod alternatives;
mod background;
mod code_editor;
mod config;
mod core;
use libretypist::docs_bridge;
mod emoji;
pub mod history;
mod local_http;
mod model;
pub mod model_catalog;
pub mod model_setup;
#[cfg(target_os = "macos")]
mod native;
#[cfg(target_os = "macos")]
mod native_abi;
#[cfg(target_os = "macos")]
mod native_blocks;
mod screen_context;
mod shortcuts;
mod spelling;
pub mod statistics;
mod terminal;
mod website;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let result = cli(&args[1..]);
        if let Err(error) = result {
            eprintln!("LibreType: {error}");
            std::process::exit(1);
        }
        return;
    }
    #[cfg(target_os = "macos")]
    if let Err(error) = native::run() {
        eprintln!("LibreType: {error}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("LibreType's desktop app requires macOS.");
        std::process::exit(1);
    }
}

fn cli(args: &[String]) -> Result<(), String> {
    match args[0].as_str() {
        "--help" | "-h" => println!("LibreType\n\nNo arguments: open the macOS menu-bar app.\n--models: list downloaded local GGUF models.\n--complete TEXT: generate a local continuation for supplied text.\n--help: show this help."),
        "--models" => {
            for name in model::list_models().map_err(|e| e.to_string())? {
                println!("{name}");
            }
        }
        "--complete" if args.len() == 2 => {
            let config = config::Config::load()?;
            let request = core::CompletionRequest {
                id: 1,
                prefix: args[1].clone(),
                suffix: String::new(),
                app_id: "local.libretypist.cli".into(),
                website: None,
                profile: None,
                background: crate::background::Background::default(),
            };
            let epoch = std::sync::atomic::AtomicU64::new(1);
            let output = model::generate(&config, &request, &epoch).map_err(|e| e.to_string())?;
            println!("{output}");
        }
        _ => return Err("Unknown arguments. Use --help for usage.".into()),
    }
    Ok(())
}
mod acceptance;
