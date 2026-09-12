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
        return run_scan(&repo_path, &cfg, debug, no_cache);
    }
    ui::run(&repo_path, debug, no_cache)
}

fn run_scan(repo_path: &std::path::Path, cfg: &config::Config, debug: bool, no_cache: bool) -> Result<(), Box<dyn Error>> {
    let t0 = Instant::now();
    let outcome = match engine::scan_with_cache(repo_path, cfg.max_commits, !no_cache) {
        Ok(o) => o,
        // Un repo sin commits no es un error: se reporta y sale limpio.
        Err(EngineError::NoHead) => {
            println!("{}: repo sin commits", engine::repo_name(repo_path));
            return Ok(());
        }
        Err(other) => return Err(other.into()),
    };
    let history = &outcome.history;
    let ms = t0.elapsed().as_millis();
    let n = history.commits.len();
    let merges = history.n_merges();
    let authors = history.authors.len();
    let source = match outcome.source {
        engine::ScanSource::Cache => "cache",
        engine::ScanSource::Delta(d) => {
            println!("delta: +{d} commits nuevos");
            "delta"
        }
        engine::ScanSource::Full => "full",
    };
    let top = history
        .top_author()
        .map(|(count, a)| format!("{name} <{email}> con {count}", name = a.name, email = a.email))
        .unwrap_or_else(|| "—".to_string());

    if debug {
        log::log_debug(&format!("scan: {n} commits en {ms} ms"));
    }
    println!(
        "{}: {n} commits ({merges} merges, {authors} autores) en {ms} ms · fuente: {source} · top autor: {top}",
        engine::repo_name(repo_path)
    );

    // F2: top-10 churn como verificación rápida del diff engine.
    let rows = engine::churn(history, engine::Window::ALL);
    println!("top churn:");
    for row in rows.iter().take(10) {
        let path = history
            .paths
            .get(row.file.0 as usize)
            .map(String::as_str)
            .unwrap_or("?");
        println!(
            "  {:>6}  +{:<5} -{:<5} ×{:<4} {path}",
            row.churn(),
            row.adds,
            row.dels,
            row.touches
        );
    }

    // F4: top-5 hotspots (churn × LOC en HEAD, ignorables fuera).
    let candidates: Vec<(engine::FileId, &str)> = rows
        .iter()
        .take(500)
        .filter_map(|r| history.paths.get(r.file.0 as usize).map(|p| (r.file, p.as_str())))
        .collect();
    let locs = engine::head_locs(repo_path, &candidates).unwrap_or_default();
    let hs = engine::hotspots(
        history,
        engine::Window::ALL,
        &|f| locs.get(&f).copied().unwrap_or(0),
        &|p| engine::is_ignored(p, &cfg.ignores),
        5,
    );
    println!("top hotspots (churn x LOC):");
    for h in hs {
        let path = history
            .paths
            .get(h.file.0 as usize)
            .map(String::as_str)
            .unwrap_or("?");
        println!("  {:.2}  churn {:>6}  loc {:>6}  {path}", h.score, h.churn, h.loc);
    }
    Ok(())
}
