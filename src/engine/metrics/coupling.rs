// ── Coupling: co-modificación (ALGORITHMS §4) ────────────────────────
// Pares (a<b) de FileId internados, clave ordenada (mitad de espacio).
// Cotas: commits con >MAX_FILES_TOUCADOS son ruido (vendor/lockfiles),
// y las rutas ignorables salen antes de formar pares.

use std::collections::HashMap;

use crate::engine::metrics::Window;
use crate::engine::model::{FileId, History};

pub const MAX_FILES_TOUCHED: usize = 200;
pub const MIN_COOCCURRENCES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CouplingEdge {
    pub a: FileId,
    pub b: FileId,
    pub cooc: u32,
    /// jaccard = cooc / (touches[a] + touches[b] − cooc) ∈ (0,1].
    pub jaccard: f32,
}

/// Aristas de co-modificación, ordenadas por jaccard desc.
pub fn coupling(
    history: &History,
    window: Window,
    ignore: &dyn Fn(&str) -> bool,
) -> Vec<CouplingEdge> {
    let mut cooc: HashMap<(u32, u32), u32> = HashMap::new();
    let mut touches: HashMap<u32, u32> = HashMap::new();

    for record in &history.commits {
        if !window.includes(record.time) {
            continue;
        }
        // rutas únicas del commit, sin ignorables, ordenadas (a<b gratis)
        let mut ids: Vec<u32> = record
            .files
            .iter()
            .filter_map(|f| {
                let path = history.paths.get(f.file.0 as usize).map(String::as_str)?;
                (!ignore(path)).then_some(f.file.0)
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > MAX_FILES_TOUCHED {
            continue; // mega-commit: sin señal de diseño
        }
        for id in &ids {
            *touches.entry(*id).or_insert(0) += 1;
        }
        for i in 0..ids.len() {
            for j in i + 1..ids.len() {
                *cooc.entry((ids[i], ids[j])).or_insert(0) += 1;
            }
        }
    }

    let mut edges: Vec<CouplingEdge> = cooc
        .into_iter()
        .filter(|(_, n)| *n >= MIN_COOCCURRENCES)
        .map(|((a, b), n)| {
            let ta = touches.get(&a).copied().unwrap_or(1);
            let tb = touches.get(&b).copied().unwrap_or(1);
            CouplingEdge {
                a: FileId(a),
                b: FileId(b),
                cooc: n,
                jaccard: n as f32 / (ta + tb - n) as f32,
            }
        })
        .collect();
    edges.sort_by(|x, y| {
        y.jaccard
            .partial_cmp(&x.jaccard)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then((x.a.0, x.b.0).cmp(&(y.a.0, y.b.0)))
    });
    edges
}

/// Top-k vecinos del archivo (por jaccard).
pub fn neighbors(edges: &[CouplingEdge], file: FileId, k: usize) -> Vec<(FileId, f32, u32)> {
    let mut v: Vec<(FileId, f32, u32)> = edges
        .iter()
        .filter_map(|e| {
            if e.a == file {
                Some((e.b, e.jaccard, e.cooc))
            } else if e.b == file {
                Some((e.a, e.jaccard, e.cooc))
            } else {
                None
            }
        })
        .collect();
    v.sort_by(|x, y| {
        y.1.partial_cmp(&x.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(x.0 .0.cmp(&y.0 .0))
    });
    v.truncate(k);
    v
}

#[test]
fn always_together_pair_is_detected() {
    use crate::engine::model::{AuthorId, CommitRecord, FileStat};
    let mk = |t: i64, ids: &[u32]| CommitRecord {
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
    };
    let h = History {
        commits: vec![
            mk(1, &[0, 1]),
            mk(2, &[0, 1]),
            mk(3, &[0, 1]),
            mk(4, &[0, 2]),
        ],
        authors: vec![],
        paths: vec!["a.rs".into(), "b.rs".into(), "c.rs".into()],
    };
    let edges = coupling(&h, Window::ALL, &|_| false);
    assert_eq!(edges.len(), 1, "a-b pasa min_co=3; a-c solo 1");
    assert_eq!((edges[0].a, edges[0].b), (FileId(0), FileId(1)));
    let n = neighbors(&edges, FileId(1), 5);
    assert_eq!(n[0].0, FileId(0));
    assert!(n[0].1 > 0.0 && n[0].1 <= 1.0);
}
