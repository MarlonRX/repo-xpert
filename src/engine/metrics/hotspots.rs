// ── Hotspots: churn × complejidad (ALGORITHMS §2) ───────────────────
// Puro: recibe la función LOC (el HEAD real la inyecta en git/blob.rs;
// los tests inyectan un mapa). Complejidad MVP = LOC actual (proxy).
// Normalización por rank percentílico (robusta a outliers, no min-max).

use std::collections::HashMap;

use crate::engine::metrics::Window;
use crate::engine::metrics::churn::churn;
use crate::engine::model::{FileId, History};

#[derive(Debug, Clone, PartialEq)]
pub struct HotspotRow {
    pub file: FileId,
    pub churn: u32,
    pub loc: u32,
    /// (0,1]: rank_pct(churn) × rank_pct(loc).
    pub score: f32,
}

/// Patrones que nunca deberían ser candidatos a hotspot (ruido, no código).
pub const DEFAULT_IGNORES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    ".min.js",
    ".min.css",
    "vendor/",
    "dist/",
    "build/",
    "target/",
    "node_modules/",
];

/// `true` si la ruta debe excluirse de métricas.
pub fn is_ignored(path: &str, extra: &[String]) -> bool {
    if extra.iter().any(|p| path.contains(p.as_str())) {
        return true;
    }
    DEFAULT_IGNORES.iter().any(|p| path.contains(p))
}

/// Top-k hotspots: candidatos por churn (sin ignorables ni borrados),
/// luego score por producto de ranks percentílicos.
pub fn hotspots(
    history: &History,
    window: Window,
    loc_of: &dyn Fn(FileId) -> u32,
    ignore: &dyn Fn(&str) -> bool,
    top_k: usize,
) -> Vec<HotspotRow> {
    let mut candidates: Vec<(FileId, u32)> = churn(history, window)
        .into_iter()
        .filter(|r| {
            let path = history.paths.get(r.file.0 as usize).map(String::as_str);
            path.is_some_and(|p| !ignore(p))
        })
        .take(top_k.max(1) * 10) // sobreselección: los ignorados y borrados salen
        .map(|r| (r.file, r.churn()))
        .filter(|(f, _)| loc_of(*f) > 0) // borrado desde HEAD: no es hotspot vivo
        .collect();
    candidates.truncate(top_k);
    if candidates.is_empty() {
        return Vec::new();
    }

    // Ranks percentílicos: posición tras ordenar / n (empates: posición
    // contigua, aceptable para MVP y determinista).
    let mut by_churn = candidates.clone();
    by_churn.sort_by_key(|(f, c)| (*c, f.0));
    let n = by_churn.len();
    let churn_rank: HashMap<FileId, f32> = by_churn
        .iter()
        .enumerate()
        .map(|(i, (f, _))| (*f, (i + 1) as f32 / n as f32))
        .collect();

    let mut by_loc: Vec<(FileId, u32)> = candidates.iter().map(|(f, _)| (*f, loc_of(*f))).collect();
    by_loc.sort_by_key(|(f, l)| (*l, f.0));
    let loc_rank: HashMap<FileId, f32> = by_loc
        .iter()
        .enumerate()
        .map(|(i, (f, _))| (*f, (i + 1) as f32 / n as f32))
        .collect();

    let mut rows: Vec<HotspotRow> = candidates
        .iter()
        .map(|(f, c)| HotspotRow {
            file: *f,
            churn: *c,
            loc: loc_of(*f),
            score: churn_rank[f] * loc_rank[f],
        })
        .collect();
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.file.0.cmp(&b.file.0))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{AuthorId, CommitRecord, FileStat};
    use std::collections::HashMap;

    fn hist(files: &[&[u32]]) -> History {
        // files[i] = lista de file ids tocados (1 línea cada uno) en el commit i
        let mut paths: Vec<String> = Vec::new();
        let mut commits = Vec::new();
        for (t, ids) in files.iter().enumerate() {
            let rec = CommitRecord {
                oid: [0; 20],
                time: t as i64,
                author: AuthorId(0),
                n_parents: 1,
                files: ids
                    .iter()
                    .map(|i| FileStat {
                        file: FileId(*i),
                        adds: 1,
                        dels: 0,
                        binary: false,
                    })
                    .collect(),
            };
            for i in ids.iter() {
                let p = format!("f{i}.rs");
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }
            commits.push(rec);
        }
        History {
            commits,
            authors: vec![],
            paths,
        }
    }

    #[test]
    fn dust_and_monolith_lose_to_hotspot() {
        // f0: mucho churn, chico (polvo). f1: enorme, casi no cambia (moleza).
        // f2: mucho churn Y grande → hotspot.
        let mut commits = Vec::new();
        for t in 0..100 {
            let ids: Vec<u32> = if t < 90 {
                vec![0, 2] // f0 y f2 cambian 90 veces
            } else {
                vec![2]
            };
            commits.push(CommitRecord {
                oid: [0; 20],
                time: t,
                author: AuthorId(0),
                n_parents: 1,
                files: ids
                    .iter()
                    .map(|i| FileStat {
                        file: FileId(*i),
                        adds: 1,
                        dels: 0,
                        binary: false,
                    })
                    .collect(),
            });
        }
        let h = History {
            commits,
            authors: vec![],
            paths: vec!["f0.rs".into(), "f1.rs".into(), "f2.rs".into()],
        };
        let locs: HashMap<u32, u32> = [(0u32, 50u32), (1, 5000), (2, 4000)]
            .into_iter()
            .collect();
        let rows = hotspots(
            &h,
            Window::ALL,
            &|f| locs.get(&f.0).copied().unwrap_or(0),
            &|_| false,
            10,
        );
        assert_eq!(rows[0].file, FileId(2), "churn alto + grande gana");
        assert!(rows.iter().all(|r| r.file != FileId(1)), "sin churn no es candidato");
    }

    #[test]
    fn deleted_files_are_not_hotspots() {
        let h = hist(&[&[0], &[0]]);
        let rows = hotspots(&h, Window::ALL, &|_| 0, &|_| false, 5);
        assert!(rows.is_empty(), "loc 0 = borrado: fuera");
    }

    #[test]
    fn ignores_filter_lockfiles() {
        let h = hist(&[&[0], &[0], &[0]]);
        let rows = hotspots(
            &h,
            Window::ALL,
            &|_| 100,
            &|p| is_ignored(p, &[]),
            5,
        );
        assert_eq!(rows.len(), 1, "f0.rs no es ignorable");

        let mut paths = h.clone();
        paths.paths = vec!["Cargo.lock".into()];
        let rows = hotspots(
            &paths,
            Window::ALL,
            &|_| 100,
            &|p| is_ignored(p, &[]),
            5,
        );
        assert!(rows.is_empty());
    }
}
