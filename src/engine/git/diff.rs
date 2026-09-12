// ── Diff por commit (gix) ────────────────────────────────────────────
// tree(commit) vs tree(primer padre) → Vec<FileStat>.
// Políticas de ALGORITHMS §0: merges sin diff; renames imputados a la
// ruta NUEVA; binarios = touch sin líneas.

use gix::bstr::BString;

use crate::engine::error::{EngineError, gix_err};
use crate::engine::model::{FileId, FileStat, PathInterner};

/// Difea un commit contra su primer padre y acumula FileStats en `out`.
pub fn diff_commit(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    interner: &mut PathInterner,
    out: &mut Vec<FileStat>,
) -> Result<(), EngineError> {
    let parents: Vec<gix::Id<'_>> = commit.parent_ids().collect();
    if parents.len() > 1 {
        return Ok(()); // merge: sin diff (política)
    }
    let new_tree = commit.tree().map_err(gix_err)?;
    let old_tree = if parents.is_empty() {
        None
    } else {
        let parent = parents[0]
            .object()
            .map_err(gix_err)?
            .try_into_commit()
            .map_err(gix_err)?;
        Some(parent.tree().map_err(gix_err)?)
    };

    let mut opts = gix::diff::Options::default(); // ya trackea path completo
    opts.track_rewrites(Some(gix::diff::Rewrites::default())); // renames como en `git -M`
    let changes = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(opts))
        .map_err(gix_err)?;

    use gix::object::tree::diff::ChangeDetached as C;
    for change in changes {
        match change {
            C::Addition {
                location,
                entry_mode,
                id,
                ..
            } => {
                if entry_mode.is_tree() {
                    continue; // gix ya reporta los archivos anidados individualmente
                }
                let path = path_of(interner, &location);
                push_blob_stat(repo, None, Some(id), path, out)?;
            }
            C::Deletion {
                location,
                entry_mode,
                id,
                ..
            } => {
                if entry_mode.is_tree() {
                    continue; // ídem: las eliminaciones llegan por archivo
                }
                let path = path_of(interner, &location);
                push_blob_stat(repo, Some(id), None, path, out)?;
            }
            C::Modification {
                location,
                previous_id,
                entry_mode,
                id,
                ..
            } => {
                let path = path_of(interner, &location);
                if entry_mode.is_tree() {
                    // Cambio de blob→tree (raro): lo tratamos como reemplazo total.
                    push_blob_stat(repo, None, Some(id), path, out)?;
                } else {
                    push_blob_stat(repo, Some(previous_id), Some(id), path, out)?;
                }
            }
            C::Rewrite {
                location,
                entry_mode,
                diff,
                ..
            } => {
                if entry_mode.is_tree() {
                    continue; // move de directorio: los hijos llegan individuales
                }
                // Rename/copy: se imputa a la ruta nueva (política F2).
                let path = path_of(interner, &location);
                let (adds, dels) = match diff {
                    Some(stats) => (stats.insertions, stats.removals),
                    None => (0, 0), // contenido idéntico: solo movimiento
                };
                out.push(FileStat {
                    file: path,
                    adds,
                    dels,
                    binary: false,
                });
            }
        }
    }
    Ok(())
}

fn location_str(location: &BString) -> String {
    String::from_utf8_lossy(location).into_owned()
}

fn path_of(interner: &mut PathInterner, location: &BString) -> FileId {
    interner.intern(&location_str(location))
}

/// Diff de un blob contra vacío/vacío contra blob (addition/deletion).
/// Un oid que no resuelve a blob (gitlink/submódulo, cambio tree↔blob)
/// se registra como touch binario: sin conteo, sin error.
fn push_blob_stat(
    repo: &gix::Repository,
    old: Option<gix::ObjectId>,
    new: Option<gix::ObjectId>,
    file: FileId,
    out: &mut Vec<FileStat>,
) -> Result<(), EngineError> {
    let old_bytes = old.map(|id| blob_bytes(repo, id)).transpose()?;
    let new_bytes = new.map(|id| blob_bytes(repo, id)).transpose()?;
    let non_blob = old_bytes.iter().any(|b| b.is_none()) || new_bytes.iter().any(|b| b.is_none());
    let mut stat = line_stats(
        file,
        old_bytes.as_ref().and_then(Option::as_deref),
        new_bytes.as_ref().and_then(Option::as_deref),
    );
    if non_blob {
        stat.binary = true;
        stat.adds = 0;
        stat.dels = 0;
    }
    out.push(stat);
    Ok(())
}

/// `None` = el objeto no es un blob (submódulo, tree, tag).
fn blob_bytes(repo: &gix::Repository, id: gix::ObjectId) -> Result<Option<Vec<u8>>, EngineError> {
    let obj = repo.find_object(id).map_err(gix_err)?;
    if obj.kind != gix::object::Kind::Blob {
        return Ok(None);
    }
    Ok(Some(obj.try_into_blob().map_err(gix_err)?.data.clone()))
}

/// Heurística binaria de git: NUL en los primeros 8000 bytes.
fn looks_binary(data: &[u8]) -> bool {
    data.iter().take(8000).any(|&b| b == 0)
}

/// Cuenta adds/dels con el algoritmo de gix (imara + slider heuristics,
/// mismo que usa git para numstat). Vacío = blob inexistente.
fn line_stats(file: FileId, old: Option<&[u8]>, new: Option<&[u8]>) -> FileStat {
    let (old, new) = (old.unwrap_or(b""), new.unwrap_or(b""));
    if looks_binary(old) || looks_binary(new) {
        return FileStat {
            file,
            adds: 0,
            dels: 0,
            binary: true,
        };
    }
    let input = gix::diff::blob::InternedInput::new(old, new);
    let diff = gix::diff::blob::diff_with_slider_heuristics(
        gix::diff::blob::Algorithm::Histogram,
        &input,
    );
    FileStat {
        file,
        adds: diff.count_additions() as u32,
        dels: diff.count_removals() as u32,
        binary: false,
    }
}

