// ── Version info ─────────────────────────────────────────────────────
// Versión mínima: solo lo que Cargo conoce en compile time.
// (El build.rs con git hash del fork queda afuera: es superficie
// operativa que F0 corta. Si hace falta, vuelve con el release real.)

pub const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Versión corta para el badge de la UI: `v0.0.1`.
pub fn short() -> String {
    format!("v{PKG_VERSION}")
}

/// Versión completa; marca builds de debug.
pub fn full() -> String {
    if cfg!(debug_assertions) {
        format!("{}-dev", short())
    } else {
        short()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_has_v_prefix() {
        assert!(short().starts_with('v'));
    }

    #[test]
    fn full_starts_with_short_in_both_profiles() {
        assert!(full().starts_with(&short()));
    }
}
