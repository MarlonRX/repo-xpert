// ── Walk del DAG de commits (gix) ────────────────────────────────────
// F1: BFS desde HEAD con worklist + HashSet de oid vistos (el historial
// es un DAG: los merges convergen, el seen-set los dedupa). No hace
// diffs todavía: solo metadatos reducidos a CommitMeta.

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use crate::engine::error::{EngineError, gix_err};
use crate::engine::git::{oid20, open_repo};
use crate::engine::model::{AuthorInterner, CommitMeta, History, Oid};

/// Recorre el historial desde HEAD hasta `max_commits`.
/// Orden: newest→oldest aproximado (BFS por padres); el orden exacto
/// topológico no importa para las métricas, que agregan por ventana.
pub fn scan_history(repo_path: &Path, max_commits: u64) -> Result<History, EngineError> {
    let repo = open_repo(repo_path)?;
    // head_commit() falla con unborn HEAD (repo recién init): es NoHead,
    // no un error de lectura.
    let head = repo
        .head_commit()
        .map_err(|_| EngineError::NoHead)?;

    let mut interner = AuthorInterner::default();
    let mut commits: Vec<CommitMeta> = Vec::new();
    let mut seen: HashSet<Oid> = HashSet::new();
    let mut worklist: VecDeque<gix::ObjectId> = VecDeque::new();
    worklist.push_back(head.id);

    while let Some(gix_oid) = worklist.pop_front() {
        if commits.len() as u64 >= max_commits {
            break;
        }
        let oid = oid20(gix_oid.as_bytes())?;
        if !seen.insert(oid) {
            continue;
        }
        let commit = repo
            .find_object(gix_oid)
            .map_err(gix_err)?
            .try_into_commit()
            .map_err(gix_err)?;
        let author = commit.author().map_err(gix_err)?;
        let name = String::from_utf8_lossy(author.name.as_ref()).into_owned();
        let email = String::from_utf8_lossy(author.email.as_ref()).into_owned();
        let parents: Vec<gix::ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();
        let n_parents = parents.len() as u8;
        worklist.extend(parents);
        commits.push(CommitMeta {
            oid,
            time: author.seconds(),
            author: interner.intern(name, email),
            n_parents,
        });
    }

    Ok(History {
        commits,
        authors: interner.into_list(),
        paths: Vec::new(),
    })
}
