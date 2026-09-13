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
    let neighbors_idx = args.iter().position(|a| a == "--neighbors");
    let neighbor_of = neighbors_idx.and_then(|i| args.get(i + 1)).cloned();
    // repo_path: primer posicional que no sea flag ni valor de --neighbors.
    let repo_path = args
        .iter()
        .enumerate()
        .find(|(i, a)| {
            !a.starts_with('-')
                && a.as_str() != "scan"
                && neighbors_idx.is_none_or(|ni| *i != ni + 1)
        })
        .map(|(_, a)| PathBuf::from(a))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if debug {
        log::clear();
        log::log_debug("gadv starting (debug)");
    }

    let cfg = config::load_config();

    if scan {
        return run_scan(&repo_path, &cfg, debug, no_cache, neighbor_of);
    }
    ui::run(&repo_path, debug, no_cache)
}

fn run_scan(
    repo_path: &std::path::Path,
    cfg: &config::Config,
    debug: bool,
    no_cache: bool,
    neighbor_of: Option<String>,
) -> Result<(), Box<dyn Error>> {
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

    // F5: riesgo de conocimiento (bus factor 1) como resumen de una línea.
    let own = engine::ownership(history, engine::Window::ALL);
    let (bf1, total) = engine::repo_risk(&own, 50);
    println!("ownership: {bf1} de {total} modulos con bus factor 1 (>=50 lineas kept)");
    for r in own.iter().filter(|r| r.bus_factor == 1).take(5) {
        let path = history
            .paths
            .get(r.file.0 as usize)
            .map(String::as_str)
            .unwrap_or("?");
        let owner = r
            .owner
            .and_then(|a| history.authors.get(a.0 as usize))
            .map(|a| a.name.as_str())
            .unwrap_or("?");
        println!("  bf 1  {:>6} kept  {owner:<12} {path}", r.kept_total);
    }

    // F6: top-3 aristas de co-modificación (o vecinos de --neighbors).
    let edges = engine::coupling(history, engine::Window::ALL, &|p| {
        engine::is_ignored(p, &cfg.ignores)
    });
    if let Some(target) = neighbor_of {
        let Some(fid) = history
            .paths
            .iter()
            .position(|p| p.ends_with(target.as_str()) || p == &target)
            .map(|i| engine::FileId(i as u32))
        else {
            println!("no encontre un path que calce con {target}");
            return Ok(());
        };
        println!("vecinos de {target}:");
        for (f, j, cooc) in engine::neighbors(&edges, fid, 5) {
            let path = history.paths.get(f.0 as usize).map(String::as_str).unwrap_or("?");
            println!("  {j:.2}  ×{cooc:<4} {path}");
        }
        return Ok(());
    }
    println!("coupling (co-modificacion, min 3):");
    for e in edges.iter().take(3) {
        let pa = history.paths.get(e.a.0 as usize).map(String::as_str).unwrap_or("?");
        let pb = history.paths.get(e.b.0 as usize).map(String::as_str).unwrap_or("?");
        println!("  {:.2}  ×{:<4} {pa} <-> {pb}", e.jaccard, e.cooc);
    }
    Ok(())
}
