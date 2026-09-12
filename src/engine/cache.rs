// ── Caché de historial (DECISIONS §3c) ───────────────────────────────
// JSON endurecido v1 en `<repo>/.git/git-advance/cache.json`:
//  1. envelope con format_version; mismatch → rebuild (nunca migrar)
//  2. oids hex en disco
//  3. interning también en disco: records usan u32 sobre tablas
//  4. solo enteros; lo derivado se recalcula al cargar
//  5. campos nuevos con #[serde(default)]; sin version/head → rebuild
//  6. sample legible congelado en tests/cache_v1_sample.json

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::engine::error::{EngineError, gix_err};
use crate::engine::git::walk::scan_repo;
use crate::engine::git::{oid20, open_repo};
use crate::engine::model::{AuthorInfo, CACHE_FORMAT_VERSION, History, Oid};

/// Qué se hizo para producir el History devuelto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanSource {
    /// HEAD cacheado: cero parsing.
    Cache,
    /// Delta de N commits nuevos + resto cacheado.
    Delta(usize),
    /// Reconstrucción completa (primera vez, mismatch de versión o no-empalme).
    Full,
}

#[derive(Debug)]
pub struct CacheOutcome {
    pub history: History,
    pub source: ScanSource,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    format_version: u32,
    /// hex 40; `#[serde(default)]` no: si falta, es archivo corrupto → rebuild.
    head_oid: String,
    commit_count: usize,
    authors: Vec<AuthorInfo>,
    paths: Vec<String>,
    records: Vec<crate::engine::model::CommitRecord>,
}

/// Vista pública del formato para el test de contrato (v1 congelado).
#[doc(hidden)]
#[derive(Debug, Serialize, Deserialize)]
pub struct CacheFileTest {
    pub format_version: u32,
    pub head_oid: String,
    pub commit_count: usize,
    pub authors: Vec<AuthorInfo>,
    pub paths: Vec<String>,
    pub records: Vec<crate::engine::model::CommitRecord>,
}

/// Escanea con caché. `use_cache=false` (o repo sin permiso de escritura)
/// degrada a Full sin error.
pub fn scan_with_cache(
    repo_path: &Path,
    max_commits: u64,
    use_cache: bool,
) -> Result<CacheOutcome, EngineError> {
    if !use_cache {
        let repo = open_repo(repo_path)?;
        let out = scan_repo(&repo, None, max_commits)?;
        return Ok(CacheOutcome {
            history: History {
                commits: out.records,
                authors: out.authors,
                paths: out.paths,
            },
            source: ScanSource::Full,
        });
    }

    let repo = open_repo(repo_path)?;
    let head = repo.head_commit().map_err(|_| EngineError::NoHead)?;
    let head_oid = oid20(head.id.as_bytes())?;
    let path = cache_path(repo.path());

    if let Some(cf) = read_cache(&path) {
        if cf.format_version == CACHE_FORMAT_VERSION && hex_to_oid(&cf.head_oid) == Some(head_oid)
        {
            return Ok(CacheOutcome {
                history: History {
                    commits: cf.records,
                    authors: cf.authors,
                    paths: cf.paths,
                },
                source: ScanSource::Cache,
            });
        }

        // HEAD nuevo: walk incremental hasta empalmar con el viejo HEAD.
        // Solo se acepta empalme en cached[0]: un empalme más abajo implica
        // historial reescrito (amend/reset entre merges) y dejaría commits
        // inalcanzables en el resto del cache → rebuild limpio.
        let cached_set: HashSet<Oid> = cf.records.iter().map(|r| r.oid).collect();
        let walked = scan_repo(&repo, Some(&cached_set), max_commits)?;
        if let Some(splice) = walked.splice
            && cf.records.first().is_some_and(|r| r.oid == splice)
        {
            let delta = walked.records.len();
            let mut history = merge_delta(cf, 0, walked);
            sort_newest_first(&mut history);
            // El HEAD cacheado ya no sirve: guardamos el estado actual.
            let _ = save_cache(&path, &head_oid, &history);
            return Ok(CacheOutcome {
                source: ScanSource::Delta(delta),
                history,
            });
        }
        // Sin empalme (rebase/gc): rebuild total.
    }

    let out = scan_repo(&repo, None, max_commits)?;
    let history = History {
        commits: out.records,
        authors: out.authors,
        paths: out.paths,
    };
    let _ = save_cache(&path, &head_oid, &history);
    Ok(CacheOutcome {
        history,
        source: ScanSource::Full,
    })
}

/// `<git_dir>/git-advance/cache.json` (dentro de .git: automático por repo).
fn cache_path(git_dir: &Path) -> PathBuf {
    git_dir.join("git-advance").join("cache.json")
}

fn read_cache(path: &Path) -> Option<CacheFile> {
    let data = fs::read_to_string(path).ok()?;
    let cf: CacheFile = serde_json::from_str(&data).ok()?;
    // Regla 1: versión desconocida → tratar como inexistente (rebuild).
    if cf.format_version != CACHE_FORMAT_VERSION {
        return None;
    }
    // Regla 5: sin head_oid util o contadores que cuadran, es archivo ajeno.
    if cf.head_oid.len() != 40 || cf.commit_count != cf.records.len() {
        return None;
    }
    Some(cf)
}

fn save_cache(path: &Path, head_oid: &Oid, history: &History) -> Result<(), EngineError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(gix_err)?;
    }
    let cf = CacheFile {
        format_version: CACHE_FORMAT_VERSION,
        head_oid: oid_hex(head_oid),
        commit_count: history.commits.len(),
        authors: history.authors.clone(),
        paths: history.paths.clone(),
        records: history.commits.clone(),
    };
    let data = serde_json::to_string(&cf).map_err(gix_err)?;
    // Escritura atómica: temp + rename (un cache roto nunca debe tumbarte).
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, data).map_err(gix_err)?;
    fs::rename(&tmp, path).map_err(gix_err)?;
    Ok(())
}

/// Une el delta recién parseado (con sus propias tablas u32) al cache.
fn merge_delta(
    cf: CacheFile,
    splice_idx: usize,
    walked: crate::engine::git::walk::ScanOutput,
) -> History {
    let mut authors = cf.authors;
    let mut paths = cf.paths;
    let mut author_map: HashMap<(String, String), u32> = authors
        .iter()
        .enumerate()
        .map(|(i, a)| ((a.name.clone(), a.email.clone()), i as u32))
        .collect();
    let mut path_map: HashMap<String, u32> =
        paths.iter().enumerate().map(|(i, p)| (p.clone(), i as u32)).collect();

    let mut records = Vec::with_capacity(walked.records.len() + cf.records.len() - splice_idx);
    for mut r in walked.records {
        let a = &walked.authors[r.author.0 as usize];
        r.author.0 = *author_map.entry((a.name.clone(), a.email.clone())).or_insert_with(|| {
            authors.push(a.clone());
            (authors.len() - 1) as u32
        });
        for f in &mut r.files {
            let p = walked.paths[f.file.0 as usize].clone();
            f.file.0 = *path_map.entry(p.clone()).or_insert_with(|| {
                paths.push(p);
                (paths.len() - 1) as u32
            });
        }
        records.push(r);
    }
    records.extend(cf.records.into_iter().skip(splice_idx));
    History {
        commits: records,
        authors,
        paths,
    }
}

/// Orden newest→oldest por epoch (empates: el que ya estaba primero).
fn sort_newest_first(h: &mut History) {
    h.commits.sort_by_key(|c| std::cmp::Reverse(c.time));
}

fn oid_hex(oid: &Oid) -> String {
    oid.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_to_oid(s: &str) -> Option<Oid> {
    if s.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{AuthorId, CommitRecord, FileId, FileStat};

    fn sample_history() -> History {
        History {
            commits: vec![
                CommitRecord {
                    oid: [0xab; 20],
                    time: 200,
                    author: AuthorId(0),
                    n_parents: 1,
                    files: vec![FileStat {
                        file: FileId(0),
                        adds: 10,
                        dels: 2,
                        binary: false,
                    }],
                },
                CommitRecord {
                    oid: [0x11; 20],
                    time: 100,
                    author: AuthorId(0),
                    n_parents: 0,
                    files: vec![FileStat {
                        file: FileId(1),
                        adds: 3,
                        dels: 0,
                        binary: true,
                    }],
                },
            ],
            authors: vec![AuthorInfo {
                name: "Ana".into(),
                email: "ana@x".into(),
            }],
            paths: vec!["a.rs".into(), "b.bin".into()],
        }
    }

    #[test]
    fn round_trip_preserves_history() {
        let h = sample_history();
        let dir = std::env::temp_dir().join(format!("gadv-cache-rt-{}", std::process::id()));
        let path = cache_path(&dir);
        save_cache(&path, &[0xcd; 20], &h).unwrap();
        let cf = read_cache(&path).expect("cache legible");
        assert_eq!(cf.format_version, CACHE_FORMAT_VERSION);
        let back = History {
            commits: cf.records,
            authors: cf.authors,
            paths: cf.paths,
        };
        assert_eq!(back, h);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_version_is_rejected() {
        let h = sample_history();
        let dir = std::env::temp_dir().join(format!("gadv-cache-ver-{}", std::process::id()));
        let path = cache_path(&dir);
        save_cache(&path, &[0xab; 20], &h).unwrap();
        let mut raw: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        raw["format_version"] = serde_json::json!(999);
        fs::write(&path, raw.to_string()).unwrap();
        assert!(read_cache(&path).is_none(), "version desconocida → rebuild");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn oids_are_hex_on_disk() {
        let h = sample_history();
        let dir = std::env::temp_dir().join(format!("gadv-cache-hex-{}", std::process::id()));
        let path = cache_path(&dir);
        save_cache(&path, &[0xab; 20], &h).unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"abababab"), "oid debe verse hex en el JSON");
        assert!(!raw.contains("[171,"), "nunca array de numeros");
        let _ = fs::remove_dir_all(&dir);
    }
}
