// ── Walk del DAG de commits (gix) ────────────────────────────────────
// F1: BFS desde HEAD con worklist + HashSet de oid vistos (el historial
// es un DAG: los merges convergen, el seen-set los dedupa).
// F2: cada commit no-merge trae además su diff de trees (FileStats).

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use crate::engine::error::{EngineError, gix_err};
use crate::engine::git::diff::diff_commit;
use crate::engine::git::{oid20, open_repo};
use crate::engine::model::{AuthorInterner, CommitRecord, History, PathInterner, Oid};

/// Recorre el historial desde HEAD hasta `max_commits`, difunteando cada
/// commit contra su primer padre.
/// Orden: newest→oldest aproximado (BFS por padres); el orden exacto
/// topológico no importa para las métricas, que agregan por ventana.
pub fn scan_history(repo_path: &Path, max_commits: u64) -> Result<History, EngineError> {
    let repo = open_repo(repo_path)?;
    // head_commit() falla con unborn HEAD (repo recién init): es NoHead,
    // no un error de lectura.
    let head = repo
        .head_commit()
        .map_err(|_| EngineError::NoHead)?;

    let mut author_interner = AuthorInterner::default();
    let mut path_interner = PathInterner::default();
    let mut commits: Vec<CommitRecord> = Vec::new();
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

        let mut files = Vec::new();
        diff_commit(&repo, &commit, &mut path_interner, &mut files)?;

        commits.push(CommitRecord {
            oid,
            time: author.seconds(),
            author: author_interner.intern(name, email),
            n_parents,
            files,
        });
    }

    Ok(History {
        commits,
        authors: author_interner.into_list(),
        paths: path_interner.into_list(),
    })
}
