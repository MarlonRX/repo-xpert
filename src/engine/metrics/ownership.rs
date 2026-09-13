// ── Ownership y bus factor (ALGORITHMS §3) ───────────────────────────
// Heurística por commits (NO blame): kept[a,f] = max(0, Σ adds[a,f] −
// Σ dels[otros→f]). Sale gratis del mismo barrido: cada FileStat vive en
// un CommitRecord que ya tiene su autor (no hace falta bump de caché).

use std::collections::HashMap;

use crate::engine::metrics::Window;
use crate::engine::model::{AuthorId, FileId, History};

/// Umbral de conocimiento para el bus factor (50%, heurística estándar).
pub const THETA: f32 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub struct OwnershipRow {
    pub file: FileId,
    /// Autor con más líneas kept (None si el archivo está todo roto: bf 0).
    pub owner: Option<AuthorId>,
    /// Share del owner primario sobre el total kept.
    pub owner_share: f32,
    /// Autores necesarios para cubrir THETA del conocimiento. 0 = muerto.
    pub bus_factor: usize,
    pub kept_total: u32,
    /// (autor, kept) ordenado desc.
    pub shares: Vec<(AuthorId, u32)>,
}

#[derive(Debug, Default)]
struct FileAcc {
    adds_by_author: HashMap<AuthorId, u32>,
    dels_by_author: HashMap<AuthorId, u32>,
    dels_total: u32,
}

/// ownership por archivo, ordenado por riesgo (bus factor asc, luego
/// kept_total desc para que lo grande y frágil quede arriba).
pub fn ownership(history: &History, window: Window) -> Vec<OwnershipRow> {
    let mut files: HashMap<FileId, FileAcc> = HashMap::new();
    for record in &history.commits {
        if !window.includes(record.time) {
            continue;
        }
        for f in &record.files {
            let acc = files.entry(f.file).or_default();
            *acc.adds_by_author.entry(record.author).or_insert(0) += f.adds;
            *acc.dels_by_author.entry(record.author).or_insert(0) += f.dels;
            acc.dels_total += f.dels;
        }
    }

    let mut rows: Vec<OwnershipRow> = files
        .into_iter()
        .map(|(file, acc)| {
            // kept[a] = adds[a] − (dels totales de OTROS sobre el archivo)
            let mut kept: Vec<(AuthorId, u32)> = acc
                .adds_by_author
                .iter()
                .map(|(a, adds)| {
                    let others_dels = acc.dels_total - acc.dels_by_author.get(a).copied().unwrap_or(0);
                    (*a, adds.saturating_sub(others_dels))
                })
                .filter(|(_, k)| *k > 0)
                .collect();
            kept.sort_by(|(a1, k1), (a2, k2)| k2.cmp(k1).then(a1.0.cmp(&a2.0)));
            let kept_total: u32 = kept.iter().map(|(_, k)| k).sum();

            let (owner, owner_share, bus_factor) = if kept_total == 0 {
                (None, 0.0, 0)
            } else {
                let mut acc_share = 0.0f32;
                let mut bf = 0usize;
                for (_, k) in &kept {
                    acc_share += *k as f32 / kept_total as f32;
                    bf += 1;
                    // `>` estricto: un 50/50 necesita a los dos (bf 2);
                    // uno que cubre el 70% alcanza solo (bf 1).
                    if acc_share > THETA {
                        break;
                    }
                }
                (Some(kept[0].0), kept[0].1 as f32 / kept_total as f32, bf)
            };

            OwnershipRow {
                file,
                owner,
                owner_share,
                bus_factor,
                kept_total,
                shares: kept,
            }
        })
        .collect();

    rows.sort_by(|a, b| {
        a.bus_factor
            .cmp(&b.bus_factor)
            .then(b.kept_total.cmp(&a.kept_total))
            .then(a.file.0.cmp(&b.file.0))
    });
    rows
}

/// (módulos con bus factor 1, módulos con dueño) — el agregado del repo.
pub fn repo_risk(rows: &[OwnershipRow], min_lines: u32) -> (usize, usize) {
    let considered: Vec<&OwnershipRow> = rows
        .iter()
        .filter(|r| r.kept_total >= min_lines)
        .collect();
    let bf1 = considered.iter().filter(|r| r.bus_factor == 1).count();
    (bf1, considered.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{CommitRecord, FileStat};

    fn commit(author: u32, time: i64, files: Vec<(u32, u32, u32)>) -> CommitRecord {
        CommitRecord {
            oid: [0; 20],
            time,
            author: AuthorId(author),
            n_parents: 1,
            files: files
                .into_iter()
                .map(|(f, a, d)| FileStat {
                    file: FileId(f),
                    adds: a,
                    dels: d,
                    binary: false,
                })
                .collect(),
        }
    }

    fn hist(commits: Vec<CommitRecord>) -> History {
        History {
            commits,
            authors: vec![],
            paths: vec![],
        }
    }

    #[test]
    fn sole_author_is_bus_factor_one() {
        let h = hist(vec![commit(0, 1, vec![(0, 100, 0)]), commit(0, 2, vec![(0, 10, 5)])]);
        let rows = ownership(&h, Window::ALL);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bus_factor, 1);
        assert_eq!(rows[0].owner, Some(AuthorId(0)));
        assert_eq!(rows[0].kept_total, 110); // adds 110; sus propios dels no descuentan
    }

    #[test]
    fn dominant_author_still_bus_factor_one() {
        // Ana 70%, Beto 30% → cubrir 50% necesita solo a Ana.
        let h = hist(vec![
            commit(0, 1, vec![(0, 70, 0)]),
            commit(1, 2, vec![(0, 30, 0)]),
        ]);
        let rows = ownership(&h, Window::ALL);
        assert_eq!(rows[0].bus_factor, 1);
        assert_eq!(rows[0].owner, Some(AuthorId(0)));
    }

    #[test]
    fn even_split_is_bus_factor_two() {
        let h = hist(vec![
            commit(0, 1, vec![(0, 50, 0)]),
            commit(1, 2, vec![(0, 50, 0)]),
        ]);
        let rows = ownership(&h, Window::ALL);
        assert_eq!(rows[0].bus_factor, 2);
    }

    #[test]
    fn wiped_out_file_is_dead() {
        // Ana aporta 10; Beto borra 10 líneas de más → kept 0 para ambos.
        let h = hist(vec![
            commit(0, 1, vec![(0, 10, 0)]),
            commit(1, 2, vec![(0, 0, 10)]),
        ]);
        let rows = ownership(&h, Window::ALL);
        assert_eq!(rows[0].bus_factor, 0);
        assert_eq!(rows[0].kept_total, 0);
        assert_eq!(rows[0].owner, None);
    }

    #[test]
    fn repo_risk_counts_modules_with_owner() {
        let h = hist(vec![
            commit(0, 1, vec![(0, 100, 0), (1, 60, 0), (1, 60, 0)]),
        ]);
        let rows = ownership(&h, Window::ALL);
        // f0: bf1 (100≥50 min), f1: bf1 Ana (120) → (2,2)
        assert_eq!(repo_risk(&rows, 50), (2, 2));
        // subiendo el piso a 110 queda solo f1
        assert_eq!(repo_risk(&rows, 110), (1, 1));
    }
}
