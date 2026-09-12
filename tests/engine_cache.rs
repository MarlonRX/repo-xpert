//! F3: caché JSON — round-trip real, invalidación por HEAD y delta.

mod common;

use common::{commit, git, make_fixture, temp_dir, write};
use gadv::engine::{ScanSource, scan_with_cache};
use std::fs;

const MAX: u64 = 50_000;

#[test]
fn first_scan_full_second_is_cache() {
    let dir = make_fixture("cache-first");
    let a = scan_with_cache(&dir, MAX, true).expect("scan 1");
    assert!(matches!(a.source, ScanSource::Full));
    assert_eq!(a.history.commits.len(), 10);

    let b = scan_with_cache(&dir, MAX, true).expect("scan 2");
    assert!(matches!(b.source, ScanSource::Cache));
    assert_eq!(b.history, a.history, "el cache debe reconstruir el mismo History");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn new_commit_produces_delta_of_one() {
    let dir = make_fixture("cache-delta");
    scan_with_cache(&dir, MAX, true).expect("scan 1");

    write(&dir, "nuevo.txt", b"linea nueva\n");
    commit(&dir, "Cami", "cami@example.com", "commit post-cache");

    let d = scan_with_cache(&dir, MAX, true).expect("scan delta");
    assert_eq!(d.source, ScanSource::Delta(1));
    assert_eq!(d.history.commits.len(), 11);
    assert!(d.history.authors.iter().any(|a| a.name == "Cami"));
    // El commit nuevo debe estar primero (newest→oldest).
    assert_eq!(d.history.commits[0].files.len(), 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rewritten_history_rebuilds() {
    let dir = make_fixture("cache-rebase");
    scan_with_cache(&dir, MAX, true).expect("scan 1");

    // reset --hard a un commit anterior: el viejo HEAD ya no es alcanzable
    // como padre del nuevo → el splice se rechaza y hay rebuild honesto.
    git(&dir, &["reset", "--hard", "HEAD~3"]);

    let r = scan_with_cache(&dir, MAX, true).expect("scan rebuild");
    assert!(matches!(r.source, ScanSource::Full), "reescritura → Full");
    assert_eq!(
        r.history.commits.len(),
        4,
        "desde c3: c0..c3 (dev queda inalcanzable)"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn no_cache_never_writes() {
    let dir = make_fixture("cache-off");
    scan_with_cache(&dir, MAX, false).expect("scan sin cache");
    assert!(
        !dir.join(".git/git-advance/cache.json").exists(),
        "--no-cache no debe dejar rastro"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn corrupt_cache_degrades_to_full() {
    let dir = make_fixture("cache-corrupt");
    scan_with_cache(&dir, MAX, true).expect("scan 1");
    fs::write(dir.join(".git/git-advance/cache.json"), "{basura").unwrap();
    let r = scan_with_cache(&dir, MAX, true).expect("scan tras corrupcion");
    assert!(matches!(r.source, ScanSource::Full));
    assert_eq!(r.history.commits.len(), 10);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn v1_sample_is_the_format_contract() {
    // Congela el formato: si un cambio de modelo rompe este archivo
    // legible a mano, rompe un test (DECISIONS §3c regla 6).
    let raw = include_str!("cache_v1_sample.json");
    let cf: gadv::engine::cache::CacheFileTest = serde_json::from_str(raw).expect("sample v1 valido");
    assert_eq!(cf.format_version, 1);
    assert_eq!(cf.records.len(), 2);
    assert_eq!(cf.records[0].files[0].adds, 4);
    assert!(cf.records[1].files[0].binary);
}

#[test]
fn empty_repo_head_is_no_head() {
    let dir = temp_dir("cache-empty");
    git(&dir, &["init", "-b", "main"]);
    let err = scan_with_cache(&dir, MAX, true).unwrap_err();
    assert!(matches!(err, gadv::engine::EngineError::NoHead));
    let _ = fs::remove_dir_all(&dir);
}
