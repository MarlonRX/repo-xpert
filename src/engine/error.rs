// ── Errores del engine ───────────────────────────────────────────────
// Tipos de error del dominio, no de gix: la UI y el CLI deciden por
// variante (p. ej. NoHead no es un fallo, es un repo vacío).

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("no es un repositorio git: {}", .0.display())]
    NotGit(PathBuf),

    #[error("el repositorio no tiene HEAD (¿repo recién inicializado?)")]
    NoHead,

    #[error("hash no-SHA1 no soportado por ahora")]
    UnsupportedHash,

    #[error("error leyendo objetos: {0}")]
    Gix(String),
}

/// Convenio: todo error de gix que no merezca variante propia colapsa
/// a `Gix(mensaje)` sin `#[from]` (evita capturar por accidente errores
/// que sí deberían tener variante).
pub(crate) fn gix_err(err: impl std::fmt::Display) -> EngineError {
    EngineError::Gix(err.to_string())
}
