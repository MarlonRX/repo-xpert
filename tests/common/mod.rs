//! Helpers compartidos por los tests de integración: construyen un repo
//! git real con `git` (solo en tests; el binario nunca shell-ea).
//! El fixture cubre: 2 autores, merge, rename y un archivo binario.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn git(dir: &Path, args: &[&str]) {
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

pub fn commit(dir: &Path, author: &str, email: &str, msg: &str) {
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

pub fn write(dir: &Path, name: &str, content: &[u8]) {
    std::fs::write(dir.join(name), content).unwrap();
}

pub fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("gadv-{tag}-{nanos}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 10 commits: 4 Ana en main, 3 Beto en dev, 1 rename Ana, 1 merge, 1 binario.
pub fn make_fixture(tag: &str) -> PathBuf {
    let dir = temp_dir(tag);
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
