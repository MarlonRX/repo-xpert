//! Fixture end-to-end del walk (F1): genera un repo git real con `git`
//! (solo en tests; el binario nunca shell-ea), lo escanea con el engine
//! y verifica conteos. El fixture cubre: 2 autores, merge, rename y un
//! archivo binario.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git debe estar en PATH para los tests");
    assert!(
        out.status.success(),
        "git {args:?} fallo: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn commit(dir: &Path, author: &str, email: &str, msg: &str) {
    git(dir, &["add", "."]);
    git(
        dir,
        &[
            "-c",
            &format!("user.name={author}"),
            "-c",
            &format!("user.email={email}"),
            "commit",
            "-m",
            msg,
        ],
    );
}

fn write(dir: &Path, name: &str, content: &[u8]) {
    std::fs::write(dir.join(name), content).unwrap();
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("gadv-fixture-{tag}-{nanos}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 10 commits: 4 Ana en main, 3 Beto en dev, 1 rename Ana, 1 merge, 1 binario.
fn make_fixture() -> PathBuf {
    let dir = temp_dir("walk");
    git(&dir, &["init", "-b", "main"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);

    for i in 0..4 {
        write(&dir, "a.txt", format!("linea {i}\n").as_bytes());
        commit(&dir, "Ana", "ana@example.com", &format!("c{i}"));
    }

    git(&dir, &["checkout", "-b", "dev"]);
    for i in 0..3 {
        write(&dir, "b.txt", format!("beto {i}\n").as_bytes());
        commit(&dir, "Beto", "beto@example.com", &format!("d{i}"));
    }

    git(&dir, &["checkout", "main"]);
    git(&dir, &["mv", "a.txt", "renamed.txt"]);
    commit(&dir, "Ana", "ana@example.com", "rename a.txt");

    git(
        &dir,
        &[
            "-c",
            "user.name=Ana",
            "-c",
            "user.email=ana@example.com",
            "merge",
            "--no-ff",
            "-m",
            "merge dev",
            "dev",
        ],
    );

    write(&dir, "bin.dat", &[0u8, 1, 2, 253, 254, 255]);
    commit(&dir, "Ana", "ana@example.com", "binario");
    dir
}

#[test]
fn scan_history_counts_all_commits_including_merges() {
    let dir = make_fixture();
    let h = gadv::engine::scan_history(&dir, 50_000).expect("scan ok");

    assert_eq!(h.commits.len(), 10, "10 commits en el fixture");
    assert_eq!(h.authors.len(), 2, "Ana y Beto");
    assert_eq!(h.n_merges(), 1, "un merge --no-ff");
    assert!(h.commits.iter().any(|c| c.n_parents == 0), "hay commit raíz");

    let (n, top) = h.top_author().unwrap();
    assert_eq!(top.name, "Ana");
    assert_eq!(n, 7, "4 + rename + merge + binario");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_history_respects_max_commits() {
    let dir = make_fixture();
    let h = gadv::engine::scan_history(&dir, 3).expect("scan ok");
    assert_eq!(h.commits.len(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn empty_repo_is_no_head_not_a_crash() {
    let dir = temp_dir("empty");
    git(&dir, &["init", "-b", "main"]);
    let err = gadv::engine::scan_history(&dir, 50_000).unwrap_err();
    assert!(matches!(err, gadv::engine::EngineError::NoHead));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_repo_path_is_not_git() {
    let dir = temp_dir("notgit");
    let err = gadv::engine::scan_history(&dir, 50_000).unwrap_err();
    assert!(matches!(err, gadv::engine::EngineError::NotGit(_)));
    let _ = std::fs::remove_dir_all(&dir);
}
