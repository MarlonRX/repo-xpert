// ── Acceso a Git (único lugar con `use gix`) ─────────────────────────
// Regla de frontera (SKELETON): nada fuera de `engine/git/` importa gix.

use std::path::Path;

use crate::engine::error::EngineError;

pub mod blob;
pub mod diff;
pub mod walk;

pub fn open_repo(path: &Path) -> Result<gix::Repository, EngineError> {
    // gix::open acepta worktrees y subdirectorios de un repo: sube solo
    // hasta descubrir el .git. Un path que no es repo → NotGit.
    gix::open(path).map_err(|_| EngineError::NotGit(path.to_path_buf()))
}

/// Convierte bytes de oid a [u8;20]; SHA-256 (rarísimo hoy) da error claro.
pub(crate) fn oid20(bytes: &[u8]) -> Result<[u8; 20], EngineError> {
    bytes.try_into().map_err(|_| EngineError::UnsupportedHash)
}
