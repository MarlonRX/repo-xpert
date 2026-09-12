// ── LOC de archivos en HEAD (ALGORITHMS §2) ──────────────────────────
// Solo se leen los candidatos a hotspot (K≈500), no todo el repo.
// Ruta no existente en HEAD (borrada) o binaria → 0.

use std::collections::HashMap;
use std::path::Path;

use crate::engine::error::{EngineError, gix_err};
use crate::engine::git::open_repo;
use crate::engine::model::FileId;

/// LOC (líneas) de cada ruta pedida tal como está en HEAD.
pub fn head_locs(
    repo_path: &Path,
    paths: &[(FileId, &str)],
) -> Result<HashMap<FileId, u32>, EngineError> {
    let repo = open_repo(repo_path)?;
    let head = repo.head_commit().map_err(|_| EngineError::NoHead)?;
    let tree = head.tree().map_err(gix_err)?;
    let mut out = HashMap::with_capacity(paths.len());
    for (id, path) in paths {
        let loc = match blob_at(&repo, &tree, path)? {
            Some(bytes) => count_lines(&bytes),
            None => 0,
        };
        out.insert(*id, loc);
    }
    Ok(out)
}

/// Resuelve `path` en el tree de HEAD. `None` = no existe o no es blob.
fn blob_at(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    path: &str,
) -> Result<Option<Vec<u8>>, EngineError> {
    let mut current = tree.clone();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let mut found = None;
        for entry in current.iter() {
            let entry = entry.map_err(gix_err)?.inner;
            if entry.filename == *part {
                found = Some(entry);
                break;
            }
        }
        let entry = match found {
            Some(e) => e,
            None => return Ok(None),
        };
        if i + 1 == parts.len() {
            if !entry.mode.is_blob() {
                return Ok(None);
            }
            let obj = repo
                .find_object(entry.oid.to_owned())
                .map_err(gix_err)?;
            if obj.kind != gix::object::Kind::Blob {
                return Ok(None);
            }
            let blob = obj.try_into_blob().map_err(gix_err)?;
            let data = blob.data.clone();
            if data.iter().take(8000).any(|&b| b == 0) {
                return Ok(None); // binario: no cuenta como LOC
            }
            return Ok(Some(data));
        } else {
            if !entry.mode.is_tree() {
                return Ok(None);
            }
            current = repo.find_tree(entry.oid.to_owned()).map_err(gix_err)?;
        }
    }
    Ok(None)
}

fn count_lines(data: &[u8]) -> u32 {
    if data.is_empty() {
        return 0;
    }
    let newlines = data.iter().filter(|&&b| b == b'\n').count() as u32;
    if data.last() == Some(&b'\n') {
        newlines
    } else {
        newlines + 1 // última línea sin terminator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_lines_with_and_without_trailing_newline() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\nb\n"), 2);
        assert_eq!(count_lines(b"a\nb"), 2);
        assert_eq!(count_lines(b"single"), 1);
    }
}
