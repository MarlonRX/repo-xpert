// ── Engine (puro) ────────────────────────────────────────────────────
// Motor git → hechos. Nada de ratatui ni std::process acá (regla 1 de
// SKELETON). `git/` es el único submódulo que conoce gix.
//
// F0: lector crudo de HEAD con std::fs (placeholder para la TUI).
// F1: `git::walk::scan_history` sobre gix: DAG completo + autores.
// F2: diffs por commit (FileStat) y métricas núcleo.

pub mod error;
pub mod git;
pub mod metrics;
pub mod model;

pub use error::EngineError;
pub use git::walk::scan_history;
pub use metrics::Window;
pub use metrics::churn::{ChurnRow, churn};
pub use model::{AuthorId, AuthorInfo, CommitRecord, FileId, FileStat, History, Oid};

use std::path::Path;

/// Nombre legible del repo para la UI.
pub fn repo_name(repo_path: &Path) -> String {
    repo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo_path.display().to_string())
}

/// Lee `.git/HEAD` crudo: `"main @ a1b2c3d"`, oid abreviado si es detached,
/// `"(sin commits)"` en un repo recién init, `"not a git repo"` si no hay `.git`.
///
/// Solo cubre refs sueltos y packed-refs; gix (F1) lo reemplaza completo.
pub fn head_raw(repo_path: &Path) -> String {
    let git_dir = repo_path.join(".git");
    if !git_dir.is_dir() {
        return "not a git repo".to_string();
    }
    let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) else {
        return "not a git repo".to_string();
    };
    let head = head.trim();
    if let Some(ref_name) = head.strip_prefix("ref: ") {
        let ref_name = ref_name.trim();
        let oid = read_ref(&git_dir, ref_name).unwrap_or_default();
        let label = ref_name.trim_start_matches("refs/heads/");
        if oid.is_empty() {
            format!("{label} @ (sin commits)")
        } else {
            format!("{label} @ {}", &oid[..oid.len().min(7)])
        }
    } else {
        head[..head.len().min(10)].to_string()
    }
}

/// Resuelve un ref a su oid: primero `.git/<ref>`, luego `packed-refs`.
fn read_ref(git_dir: &Path, ref_name: &str) -> Option<String> {
    if let Ok(oid) = std::fs::read_to_string(git_dir.join(ref_name)) {
        let oid = oid.trim().to_string();
        if !oid.is_empty() {
            return Some(oid);
        }
    }
    let packed = std::fs::read_to_string(git_dir.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        let (oid, name) = line.split_once(' ')?;
        (name.trim() == ref_name).then(|| oid.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gadv-engine-test-{tag}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".git")).unwrap();
        dir
    }

    #[test]
    fn resolves_loose_branch_ref() {
        let dir = temp_repo("loose");
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::create_dir_all(dir.join(".git/refs/heads")).unwrap();
        fs::write(
            dir.join(".git/refs/heads/main"),
            "0d1234567890abcdef01234567890abcdef0123456\n",
        )
        .unwrap();
        assert_eq!(head_raw(&dir), "main @ 0d12345");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_packed_ref() {
        let dir = temp_repo("packed");
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/dev\n").unwrap();
        fs::write(
            dir.join(".git/packed-refs"),
            "aabbccddeeff0011223344556677889900aabbcc refs/heads/dev\n",
        )
        .unwrap();
        assert_eq!(head_raw(&dir), "dev @ aabbccd");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_repo_and_detached_and_non_repo() {
        let dir = temp_repo("empty");
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(head_raw(&dir), "main @ (sin commits)");

        fs::write(dir.join(".git/HEAD"), "f00dd00dfeedfaceb0001111222233334444555\n").unwrap();
        assert_eq!(head_raw(&dir), "f00dd00dfe");
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(head_raw(Path::new("/no/existe/x")), "not a git repo");
    }
}
