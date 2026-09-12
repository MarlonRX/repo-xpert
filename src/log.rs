// ── Debug logging ────────────────────────────────────────────────────
// Con `-d` el binario escribe líneas legibles en un archivo de log en
// el temp dir del SO (no `/tmp`: el dev machine es Windows). El handle
// se abre una vez y se cachea en OnceLock<Mutex<File>>.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Path del log de debug: `<temp>/gadv-debug.log`. Estable para `tail -f`.
pub fn debug_log_path() -> &'static PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| std::env::temp_dir().join("gadv-debug.log"))
}

static LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();

fn log_file() -> Option<&'static Mutex<File>> {
    LOG_FILE
        .get_or_init(|| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(debug_log_path())
                .ok()
                .map(Mutex::new)
        })
        .as_ref()
}

/// Vacía el log. Se llama una vez al arrancar con debug, para que cada
/// sesión empiece limpia. El handle cacheado sigue escribiendo en append
/// sobre el archivo vacío, que es lo que queremos.
pub fn clear() {
    let _ = std::fs::write(debug_log_path(), "");
}

/// Agrega una línea. No-op silencioso si el logging está deshabilitado o
/// el archivo no se puede abrir (un problema de permisos no debe tumbar
/// la TUI).
pub fn log_debug(msg: &str) {
    if let Some(file) = log_file()
        && let Ok(mut f) = file.lock()
    {
        let _ = writeln!(f, "[{:.3}] {}", timestamp_secs(), msg);
    }
}

fn timestamp_secs() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:.3}", d.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_debug_does_not_panic_when_file_is_unwritable() {
        log_debug("test message");
    }

    #[test]
    fn clear_does_not_panic() {
        clear();
    }
}
