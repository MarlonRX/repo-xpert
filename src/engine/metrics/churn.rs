// ── Churn por archivo (ALGORITHMS §1) ────────────────────────────────
// O(C·F̄) tiempo, O(F) memoria. Single-thread hasta F6 (rayon).

use std::collections::HashMap;

use crate::engine::metrics::Window;
use crate::engine::model::{FileId, History};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChurnRow {
    pub file: FileId,
    pub adds: u32,
    pub dels: u32,
    pub touches: u32,
    pub last_time: i64,
    /// Archivo tocado solo como binario en todos sus commits.
    pub binary: bool,
}

impl ChurnRow {
    pub fn churn(&self) -> u32 {
        self.adds + self.dels
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Acc {
    adds: u32,
    dels: u32,
    touches: u32,
    last_time: i64,
    binary: bool,
}

/// Top-archivos por churn (adds+dels) dentro de la ventana, desc.
pub fn churn(history: &History, window: Window) -> Vec<ChurnRow> {
    let mut agg: HashMap<FileId, Acc> = HashMap::new();
    for record in &history.commits {
        if !window.includes(record.time) {
            continue;
        }
        for f in &record.files {
            // `binary` = "todos los toques fueron binarios": arranca en true
            // y se apaga al primer toque de texto.
            let a = agg
                .entry(f.file)
                .or_insert(Acc {
                    binary: true,
                    ..Default::default()
                });
            a.adds += f.adds;
            a.dels += f.dels;
            a.touches += 1;
            a.last_time = a.last_time.max(record.time);
            a.binary &= f.binary;
        }
    }
    let mut rows: Vec<ChurnRow> = agg
        .into_iter()
        .map(
            |(file, a)| ChurnRow {
                file,
                adds: a.adds,
                dels: a.dels,
                touches: a.touches,
                last_time: a.last_time,
                binary: a.binary,
            },
        )
        .collect();
    rows.sort_by(|x, y| y.churn().cmp(&x.churn()).then(x.file.0.cmp(&y.file.0)));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{AuthorId, CommitRecord, FileStat};

    fn record(time: i64, files: Vec<FileStat>) -> CommitRecord {
        CommitRecord {
            oid: [0; 20],
            time,
            author: AuthorId(0),
            n_parents: 1,
            files,
        }
    }

    fn stat(file: u32, adds: u32, dels: u32) -> FileStat {
        FileStat {
            file: FileId(file),
            adds,
            dels,
            binary: false,
        }
    }

    #[test]
    fn sums_adds_and_dels_per_file() {
        let h = History {
            commits: vec![
                record(100, vec![stat(0, 10, 2), stat(1, 1, 0)]),
                record(200, vec![stat(0, 4, 0)]),
            ],
            authors: Vec::new(),
            paths: Vec::new(),
        };
        let rows = churn(&h, Window::ALL);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].file, FileId(0), "12+4=16 gana");
        assert_eq!((rows[0].adds, rows[0].dels, rows[0].touches), (14, 2, 2));
        assert_eq!(rows[0].last_time, 200);
    }

    #[test]
    fn window_filters_by_time() {
        let h = History {
            commits: vec![
                record(50, vec![stat(0, 100, 0)]),
                record(150, vec![stat(1, 5, 0)]),
            ],
            authors: Vec::new(),
            paths: Vec::new(),
        };
        let rows = churn(&h, Window { from: Some(100) });
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].file, FileId(1));
    }

    #[test]
    fn binary_only_files_flagged() {
        let h = History {
            commits: vec![record(1, vec![FileStat { file: FileId(7), adds: 0, dels: 0, binary: true }])],
            authors: Vec::new(),
            paths: Vec::new(),
        };
        let rows = churn(&h, Window::ALL);
        assert!(rows[0].binary);
        assert_eq!(rows[0].churn(), 0);
    }
}
