// ── gadv entry point ─────────────────────────────────────────────────
// git-advance: motor de analítica de repositorios Git, solo lectura.
// F0: el TUI solo muestra repo + HEAD. Las métricas llegan en F1+.

mod config;
mod engine;
mod log;
mod theme;
mod ui;
mod version;

use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("gadv {}", version::full());
        return Ok(());
    }

    let debug = args.iter().any(|a| a == "--debug" || a == "-d");
    let no_cache = args.iter().any(|a| a == "--no-cache");
    let repo_path = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if debug {
        log::clear();
        log::log_debug("gadv starting (debug)");
    }

    ui::run(&repo_path, debug, no_cache)
}
