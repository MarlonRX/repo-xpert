// ── Walk del DAG de commits (gix) ────────────────────────────────────
// F1: BFS desde HEAD con worklist + HashSet de oid vistos (el historial
// es un DAG: los merges convergen, el seen-set los dedupa).
// F2: cada commit no-merge trae además su diff de trees (FileStats).
// F3: `stop_at` para el update incremental: el walk se corta en el primer
//     commit ya cacheado (O(delta), sin merge-base).
// F6: dos fases — walk secuencial barato (metadatos) + diffs en paralelo
//     con rayon. `gix::Repository` no es Sync: cada hilo abre el suyo
//     (thread-local, reutilizado entre commits).

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::engine::error::{EngineError, gix_err};
use crate::engine::git::diff::{RawFileStat, diff_commit_raw};
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

struct RawCommit {
    oid: Oid,
    time: i64,
    name: String,
    email: String,
    n_parents: u8,
}

thread_local! {
    static TL_REPO: RefCell<Option<(PathBuf, gix::Repository)>> = const { RefCell::new(None) };
}

/// Corre `f` con el repo de este hilo (lo abre la primera vez / si cambió).
fn with_thread_repo<T>(
    repo_path: &Path,
    f: impl FnOnce(&gix::Repository) -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    TL_REPO.with(|cell| {
        let mut slot = cell.borrow_mut();
        let stale = match slot.as_ref() {
            Some((p, _)) => p != repo_path,
            None => true,
        };
        if stale {
            *slot = Some((repo_path.to_path_buf(), open_repo(repo_path)?));
        }
        let (_p, repo) = slot
            .as_ref()
            .ok_or_else(|| EngineError::Gix("thread-local repo no inicializado".into()))?;
        f(repo)
    })
}

/// Recorre el historial desde HEAD hasta `max_commits`, difunteando cada
/// commit contra su primer padre.
pub fn scan_history(repo_path: &Path, max_commits: u64) -> Result<History, EngineError> {
    let repo = open_repo(repo_path)?;
    let out = scan_repo(repo_path, &repo, None, max_commits)?;
    Ok(History {
        commits: out.records,
        authors: out.authors,
        paths: out.paths,
    })
}

/// BFS secuencial de metadatos + diffs paralelos (rayon).
pub(crate) fn scan_repo(
    repo_path: &Path,
    repo: &gix::Repository,
    stop_at: Option<&HashSet<Oid>>,
    max_commits: u64,
) -> Result<ScanOutput, EngineError> {
    // ── fase 1: walk secuencial de metadatos (barato: ~5% del total) ──
    let (raws, splice_hit) = collect_commits(repo, stop_at, max_commits)?;

    // ── fase 2: diffs en paralelo (independientes por commit) ──
    let path_buf = repo_path.to_path_buf();
    let file_lists: Vec<Result<Vec<RawFileStat>, EngineError>> = raws
        .par_iter()
        .map(|raw| {
            if raw.n_parents > 1 {
                return Ok(Vec::new());
            }
            with_thread_repo(&path_buf, |r| {
                let commit = r
                    .find_object(oid_to_gix(&raw.oid))
                    .map_err(gix_err)?
                    .try_into_commit()
                    .map_err(gix_err)?;
                diff_commit_raw(r, &commit)
            })
        })
        .collect();

    // ── fase 3: internado secuencial (ordenes estables) ──
    let mut author_interner = AuthorInterner::default();
    let mut path_interner = PathInterner::default();
    let mut records = Vec::with_capacity(raws.len());
    for (raw, files) in raws.iter().zip(file_lists) {
        let files = files?;
        let files = files
            .into_iter()
            .map(|(p, a, d, b)| crate::engine::model::FileStat {
                file: path_interner.intern(&p),
                adds: a,
                dels: d,
                binary: b,
            })
            .collect();
        records.push(CommitRecord {
            oid: raw.oid,
            time: raw.time,
            author: author_interner.intern(raw.name.clone(), raw.email.clone()),
            n_parents: raw.n_parents,
            files,
        });
    }

    Ok(ScanOutput {
        records,
        authors: author_interner.into_list(),
        paths: path_interner.into_list(),
        splice: splice_hit,
    })
}

/// BFS de metadatos: devuelve los commits en orden newest→oldest con su
/// primer padre resuelto. El `stop_at` corta el walk (empalme con caché).
fn collect_commits(
    repo: &gix::Repository,
    stop_at: Option<&HashSet<Oid>>,
    max_commits: u64,
) -> Result<(Vec<RawCommit>, Option<Oid>), EngineError> {
    // head_commit() falla con unborn HEAD (repo recién init): es NoHead,
    // no un error de lectura.
    let head = repo
        .head_commit()
        .map_err(|_| EngineError::NoHead)?;

    let mut raws: Vec<RawCommit> = Vec::new();
    let mut seen: HashSet<Oid> = HashSet::new();
    let mut worklist: VecDeque<gix::ObjectId> = VecDeque::new();
    worklist.push_back(head.id);
    let mut splice_hit: Option<Oid> = None;

    while let Some(gix_oid) = worklist.pop_front() {
        if raws.len() as u64 >= max_commits {
            break;
        }
        let oid = oid20(gix_oid.as_bytes())?;
        if !seen.insert(oid) {
            continue;
        }
        if stop_at.is_some_and(|set| set.contains(&oid)) {
            splice_hit = Some(oid);
            break;
        }
        let commit = repo
            .find_object(gix_oid)
            .map_err(gix_err)?
            .try_into_commit()
            .map_err(gix_err)?;
        let author = commit.author().map_err(gix_err)?;
        let parents: Vec<gix::ObjectId> = commit.parent_ids().map(|p| p.detach()).collect();
        let n_parents = parents.len() as u8;
        worklist.extend(parents);

        raws.push(RawCommit {
            oid,
            time: author.seconds(),
            name: String::from_utf8_lossy(author.name.as_ref()).into_owned(),
            email: String::from_utf8_lossy(author.email.as_ref()).into_owned(),
            n_parents,
        });
    }
    Ok((raws, splice_hit))
}

fn oid_to_gix(oid: &Oid) -> gix::ObjectId {
    gix::ObjectId::try_from(oid.as_slice()).expect("sha1 de 20 bytes")
}
