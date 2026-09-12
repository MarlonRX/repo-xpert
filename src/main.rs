// ── gadv entry point ─────────────────────────────────────────────────
// git-advance: motor de analítica de repositorios Git, solo lectura.
// F1: modo `scan` (gix, sin TUI) + TUI F0. Las métricas llegan en F2+.

mod ui;

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

use gadv::config;
use gadv::engine::{self, EngineError};
use gadv::log;

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("gadv {}", gadv::version::full());
        return Ok(());
    }

    let debug = args.iter().any(|a| a == "--debug" || a == "-d");
    let no_cache = args.iter().any(|a| a == "--no-cache");
    let scan = args.iter().any(|a| a == "scan");
    let repo_path = args
        .iter()
        .find(|a| !a.starts_with('-') && a.as_str() != "scan")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if debug {
        log::clear();
        log::log_debug("gadv starting (debug)");
    }

    let cfg = config::load_config();

    if scan {
        return run_scan(&repo_path, &cfg, debug);
    }
    ui::run(&repo_path, debug, no_cache)
}

fn run_scan(repo_path: &std::path::Path, cfg: &config::Config, debug: bool) -> Result<(), Box<dyn Error>> {
    let t0 = Instant::now();
    let history = match engine::scan_history(repo_path, cfg.max_commits) {
        Ok(h) => h,
        // Un repo sin commits no es un error: se reporta y sale limpio.
        Err(EngineError::NoHead) => {
            println!("{}: repo sin commits", engine::repo_name(repo_path));
            return Ok(());
        }
        Err(other) => return Err(other.into()),
    };
    let ms = t0.elapsed().as_millis();
    let n = history.commits.len();
    let merges = history.n_merges();
    let authors = history.authors.len();
    let top = history
        .top_author()
        .map(|(count, a)| format!("{name} <{email}> con {count}", name = a.name, email = a.email))
        .unwrap_or_else(|| "—".to_string());

    if debug {
        log::log_debug(&format!("scan: {n} commits en {ms} ms"));
    }
    println!(
        "{}: {n} commits ({merges} merges, {authors} autores) en {ms} ms · top autor: {top}",
        engine::repo_name(repo_path)
    );
    Ok(())
}
