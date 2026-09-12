// ── Walk del DAG de commits (gix) ────────────────────────────────────
// F1: BFS desde HEAD con worklist + HashSet de oid vistos (el historial
// es un DAG: los merges convergen, el seen-set los dedupa).
// F2: cada commit no-merge trae además su diff de trees (FileStats).
// F3: `scan_repo` acepta un `stop_at` para el update incremental: el walk
//     se corta en el primer commit ya cacheado (O(delta), sin merge-base).

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use crate::engine::error::{EngineError, gix_err};
use crate::engine::git::diff::diff_commit;
use crate::engine::git::{oid20, open_repo};
use crate::engine::model::{
    AuthorInfo, AuthorInterner, CommitRecord, History, PathInterner, Oid,
};

pub(crate) struct ScanOutput {
    pub records: Vec<CommitRecord>,
    pub authors: Vec<AuthorInfo>,
    pub paths: Vec<String>,
    /// oid donde el walk empalmó con el cache (solo con `stop_at`).
    pub splice: Option<Oid>,
}

/// Recorre el historial desde HEAD hasta `max_commits`, difunteando cada
/// commit contra su primer padre.
pub fn scan_history(repo_path: &Path, max_commits: u64) -> Result<History, EngineError> {
    let repo = open_repo(repo_path)?;
    let out = scan_repo(&repo, None, max_commits)?;
    Ok(History {
        commits: out.records,
        authors: out.authors,
        paths: out.paths,
    })
}

/// BFS sobre el repo ya abierto. `stop_at`: si un oid poppeado está en el
/// set, el walk termina ahí (empalme con caché) y no se re-parsea.
/// Orden: newest→oldest aproximado (BFS por padres); el orden exacto
/// topológico no importa para las métricas, que agregan por ventana.
pub(crate) fn scan_repo(
    repo: &gix::Repository,
    stop_at: Option<&HashSet<Oid>>,
    max_commits: u64,
) -> Result<ScanOutput, EngineError> {
    // head_commit() falla con unborn HEAD (repo recién init): es NoHead,
    // no un error de lectura.
    let head = repo
        .head_commit()
        .map_err(|_| EngineError::NoHead)?;

    let mut author_interner = AuthorInterner::default();
    let mut path_interner = PathInterner::default();
    let mut records: Vec<CommitRecord> = Vec::new();
    let mut seen: HashSet<Oid> = HashSet::new();
    let mut worklist: VecDeque<gix::ObjectId> = VecDeque::new();
    worklist.push_back(head.id);
    let mut splice = None;

    while let Some(gix_oid) = worklist.pop_front() {
        if records.len() as u64 >= max_commits {
            break;
        }
        let oid = oid20(gix_oid.as_bytes())?;
        if !seen.insert(oid) {
            continue;
        }
        if stop_at.is_some_and(|set| set.contains(&oid)) {
            splice = Some(oid);
            break;
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
        diff_commit(repo, &commit, &mut path_interner, &mut files)?;

        records.push(CommitRecord {
            oid,
            time: author.seconds(),
            author: author_interner.intern(name, email),
            n_parents,
            files,
        });
    }

    Ok(ScanOutput {
        records,
        authors: author_interner.into_list(),
        paths: path_interner.into_list(),
        splice,
    })
}
