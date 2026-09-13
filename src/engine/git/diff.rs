// ── Diff por commit (gix) ────────────────────────────────────────────
// tree(commit) vs tree(primer padre) → FileStats crudos (path String).
// El internado de rutas pasa en walk (fase secuencial tras el par).
// Políticas de ALGORITHMS §0: merges sin diff; renames imputados a la
// ruta NUEVA; binarios = touch sin líneas.

use gix::bstr::BString;

use crate::engine::error::{EngineError, gix_err};

/// Un FileStat sin internar: (path, adds, dels, binary).
pub type RawFileStat = (String, u32, u32, bool);

/// Difea un commit contra su primer padre. Vacío = merge (política).
pub fn diff_commit_raw(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
) -> Result<Vec<RawFileStat>, EngineError> {
    let parents: Vec<gix::Id<'_>> = commit.parent_ids().collect();
    if parents.len() > 1 {
        return Ok(Vec::new()); // merge: sin diff (política)
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
    opts.track_rewrites(Some(gix::diff::Rewrites::default())); // renames como `git -M`
    let changes = repo
        .diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(opts))
        .map_err(gix_err)?;

    let mut out = Vec::new();
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
                push_blob_stat(repo, None, Some(id), &location, &mut out)?;
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
                push_blob_stat(repo, Some(id), None, &location, &mut out)?;
            }
            C::Modification {
                location,
                previous_id,
                entry_mode,
                id,
                ..
            } => {
                if entry_mode.is_tree() {
                    // Modification de un subtree: gix desciende y reporta
                    // los archivos individuales; la ruta del directorio no
                    // es un archivo (era el fantasma "src/ui/state").
                    let _ = (location, previous_id, id);
                    continue;
                }
                push_blob_stat(repo, Some(previous_id), Some(id), &location, &mut out)?;
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
                let (adds, dels) = match diff {
                    Some(stats) => (stats.insertions, stats.removals),
                    None => (0, 0), // contenido idéntico: solo movimiento
                };
                out.push((path_of(&location), adds, dels, false));
            }
        }
    }
    Ok(out)
}

fn path_of(location: &BString) -> String {
    String::from_utf8_lossy(location).into_owned()
}

/// Diff de un blob contra vacío/vacío contra blob (addition/deletion).
/// Un oid que no resuelve a blob (gitlink/submódulo, cambio tree↔blob)
/// se registra como touch binario: sin conteo, sin error.
fn push_blob_stat(
    repo: &gix::Repository,
    old: Option<gix::ObjectId>,
    new: Option<gix::ObjectId>,
    location: &BString,
    out: &mut Vec<RawFileStat>,
) -> Result<(), EngineError> {
    let old_bytes = old.map(|id| blob_bytes(repo, id)).transpose()?;
    let new_bytes = new.map(|id| blob_bytes(repo, id)).transpose()?;
    let non_blob = old_bytes.iter().any(|b| b.is_none()) || new_bytes.iter().any(|b| b.is_none());
    let (mut adds, mut dels, mut binary) = line_stats(
        old_bytes.as_ref().and_then(Option::as_deref),
        new_bytes.as_ref().and_then(Option::as_deref),
    );
    if non_blob {
        adds = 0;
        dels = 0;
        binary = true;
    }
    out.push((path_of(location), adds, dels, binary));
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
fn line_stats(old: Option<&[u8]>, new: Option<&[u8]>) -> (u32, u32, bool) {
    let (old, new) = (old.unwrap_or(b""), new.unwrap_or(b""));
    if looks_binary(old) || looks_binary(new) {
        return (0, 0, true);
    }
    let input = gix::diff::blob::InternedInput::new(old, new);
    let diff = gix::diff::blob::diff_with_slider_heuristics(
        gix::diff::blob::Algorithm::Histogram,
        &input,
    );
    (diff.count_additions() as u32, diff.count_removals() as u32, false)
}
