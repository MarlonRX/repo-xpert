//! Fixture end-to-end del walk (F1): genera un repo git real con `git`
//! (solo en tests; el binario nunca shell-ea), lo escanea con el engine
//! y verifica conteos. El fixture cubre: 2 autores, merge, rename y un
//! archivo binario.

mod common;

use common::make_fixture;
use std::fs;

#[test]
fn scan_history_counts_all_commits_including_merges() {
    let dir = make_fixture("walk");
    let h = gadv::engine::scan_history(&dir, 50_000).expect("scan ok");

    assert_eq!(h.commits.len(), 10, "10 commits en el fixture");
    assert_eq!(h.authors.len(), 2, "Ana y Beto");
    assert_eq!(h.n_merges(), 1, "un merge --no-ff");
    assert!(h.commits.iter().any(|c| c.n_parents == 0), "hay commit raíz");

    let (n, top) = h.top_author().unwrap();
    assert_eq!(top.name, "Ana");
    assert_eq!(n, 7, "4 + rename + merge + binario");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn scan_history_respects_max_commits() {
    let dir = make_fixture("walk-cap");
    let h = gadv::engine::scan_history(&dir, 3).expect("scan ok");
    assert_eq!(h.commits.len(), 3);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn empty_repo_is_no_head_not_a_crash() {
    let dir = common::temp_dir("walk-empty");
    common::git(&dir, &["init", "-b", "main"]);
    let err = gadv::engine::scan_history(&dir, 50_000).unwrap_err();
    assert!(matches!(err, gadv::engine::EngineError::NoHead));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn non_repo_path_is_not_git() {
    let dir = common::temp_dir("walk-notgit");
    let err = gadv::engine::scan_history(&dir, 50_000).unwrap_err();
    assert!(matches!(err, gadv::engine::EngineError::NotGit(_)));
    let _ = fs::remove_dir_all(&dir);
}
