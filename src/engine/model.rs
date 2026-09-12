// ── Modelo de datos del engine ───────────────────────────────────────
// El motor no guarda árboles ni diffs completos: por commit guarda oid,
// epoch, autor internado y (desde F2) Vec<FileStat>. Los ids son u32
// internados: las métricas mueven millones de pares y `String` como
// clave lo haría todo 10× más lento (DECISIONS §3a).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Oid crudo SHA-1 (20 bytes). En disco se serializa hex (F3, regla 2).
pub type Oid = [u8; 20];

/// Encoding del formato de caché (regla 1): subirlo invalida todo rebuild.
pub const CACHE_FORMAT_VERSION: u32 = 1;

mod hex_oid {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(oid: &[u8; 20], s: S) -> Result<S::Ok, S::Error> {
        let mut hex = String::with_capacity(40);
        for b in oid {
            hex.push_str(&format!("{b:02x}"));
        }
        s.serialize_str(&hex)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 20], D::Error> {
        let s = String::deserialize(d)?;
        if s.len() != 40 {
            return Err(D::Error::custom("oid hex invalido"));
        }
        let mut out = [0u8; 20];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(D::Error::custom)?;
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthorInfo {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthorId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(pub u32);

/// Cambio que un commit introdujo en una ruta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStat {
    pub file: FileId,
    pub adds: u32,
    pub dels: u32,
    /// Archivo binario: sin conteo de líneas, solo "touch" (ALGORITHMS §0).
    pub binary: bool,
}

/// Commit ya reducido a lo que consumen las métricas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRecord {
    #[serde(with = "hex_oid")]
    pub oid: Oid,
    /// Epoch seconds del AUTOR (ventanas temporales = aritmética entera).
    pub time: i64,
    pub author: AuthorId,
    /// 0 = raíz, 1 = normal, ≥2 = merge.
    pub n_parents: u8,
    /// Vacío en merges: los diffs de merge se saltean (ALGORITHMS §0).
    pub files: Vec<FileStat>,
}

/// Historial completo ya parseado: la entrada única de todas las métricas.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    pub commits: Vec<CommitRecord>,
    pub authors: Vec<AuthorInfo>,
    /// Tabla de rutas internadas.
    pub paths: Vec<String>,
}

impl History {
    pub fn n_merges(&self) -> usize {
        self.commits.iter().filter(|c| c.n_parents > 1).count()
    }

    /// Autor con más commits (desempate: el primero en aparecer).
    pub fn top_author(&self) -> Option<(usize, &AuthorInfo)> {
        let mut counts: HashMap<AuthorId, usize> = HashMap::new();
        for c in &self.commits {
            *counts.entry(c.author).or_insert(0) += 1;
        }
        let (best, count) = counts
            .into_iter()
            .max_by_key(|(id, n)| (*n, std::cmp::Reverse(id.0)))?;
        self.authors.get(best.0 as usize).map(|a| (count, a))
    }
}

#[derive(Debug, Default)]
pub struct AuthorInterner {
    map: HashMap<(String, String), u32>,
    list: Vec<AuthorInfo>,
}

impl AuthorInterner {
    pub fn intern(&mut self, name: String, email: String) -> AuthorId {
        let next = self.list.len() as u32;
        let id = *self
            .map
            .entry((name.clone(), email.clone()))
            .or_insert_with(|| {
                self.list.push(AuthorInfo { name, email });
                next
            });
        AuthorId(id)
    }

    pub fn into_list(self) -> Vec<AuthorInfo> {
        self.list
    }
}

#[derive(Debug, Default)]
pub struct PathInterner {
    map: HashMap<String, u32>,
    list: Vec<String>,
}

impl PathInterner {
    pub fn intern(&mut self, path: &str) -> FileId {
        let next = self.list.len() as u32;
        let id = *self.map.entry(path.to_string()).or_insert_with(|| {
            self.list.push(path.to_string());
            next
        });
        FileId(id)
    }

    pub fn into_list(self) -> Vec<String> {
        self.list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interner_dedups_by_name_and_email() {
        let mut it = AuthorInterner::default();
        let a = it.intern("Ana".into(), "ana@x".into());
        let b = it.intern("Ana".into(), "ana@x".into());
        let c = it.intern("Ana".into(), "ana@otro".into());
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(it.into_list().len(), 2);
    }

    #[test]
    fn path_interner_dedups() {
        let mut it = PathInterner::default();
        assert_eq!(it.intern("a.rs"), it.intern("a.rs"));
        assert_ne!(it.intern("a.rs"), it.intern("b.rs"));
        assert_eq!(it.into_list().len(), 2);
    }

    fn hist_with(counts: &[(u32, usize)]) -> History {
        let mut authors = Vec::new();
        let mut commits = Vec::new();
        for &(id, n) in counts {
            let author = AuthorId(id);
            authors.push(AuthorInfo {
                name: format!("a{id}"),
                email: String::new(),
            });
            for _ in 0..n {
                commits.push(CommitRecord {
                    oid: [0; 20],
                    time: 0,
                    author,
                    n_parents: 1,
                    files: Vec::new(),
                });
            }
        }
        History {
            commits,
            authors,
            paths: Vec::new(),
        }
    }

    #[test]
    fn top_author_picks_max_and_merges_count() {
        let h = hist_with(&[(0, 2), (1, 5)]);
        let (n, a) = h.top_author().unwrap();
        assert_eq!((n, a.name.as_str()), (5, "a1"));

        let mut h2 = h;
        h2.commits[0].n_parents = 2;
        assert_eq!(h2.n_merges(), 1);
    }
}
