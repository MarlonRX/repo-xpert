// ── Config ───────────────────────────────────────────────────────────
// JSON en el config dir del SO (`dirs`). Del fork se heredan los campos
// de presentación; `skipped_version` (modal de updates) se fue con el
// corte de cordón read-only.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub language: String,
    pub nerd_font: bool,
    pub theme: String,
    /// Analítica: techo de commits indexados (F1+ la consume; F0 solo persiste).
    #[serde(default = "default_max_commits")]
    pub max_commits: u64,
    /// Analítica: patrones a excluir de métricas (F4+). Vacío = default interno.
    #[serde(default)]
    pub ignores: Vec<String>,
    /// No leer/escribir caché (F3). Equivale a --no-cache.
    #[serde(default)]
    pub no_cache: bool,
}

fn default_max_commits() -> u64 {
    50_000
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "es".into(),
            nerd_font: false,
            theme: "Advance Ink".into(),
            max_commits: default_max_commits(),
            ignores: Vec::new(),
            no_cache: false,
        }
    }
}

pub fn get_config_path() -> PathBuf {
    if let Some(mut path) = dirs::config_dir() {
        path.push("gadv");
        path.push("config.json");
        path
    } else {
        PathBuf::from(".config/gadv/config.json")
    }
}

/// Carga la config; cualquier problema (sin archivo, JSON inválido)
/// degrada al default: la TUI nunca muere por un config roto.
pub fn load_config() -> Config {
    let path = get_config_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

#[allow(dead_code, reason = "la consume el theme-modal desde F1+; F0 solo lee")]
pub fn save_config(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(config)?;
    fs::write(path, data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_works() {
        let c = Config {
            language: "es".into(),
            nerd_font: true,
            theme: "Nord".into(),
            max_commits: 1000,
            ignores: vec!["vendor/*".into()],
            no_cache: true,
        };
        let json = serde_json::to_string_pretty(&c).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn missing_optional_fields_use_defaults() {
        let back: Config =
            serde_json::from_str(r#"{"language":"en","nerd_font":false,"theme":"Nord"}"#).unwrap();
        assert_eq!(back.max_commits, 50_000);
        assert!(back.ignores.is_empty());
        assert!(!back.no_cache);
    }

    #[test]
    fn config_path_is_under_gadv() {
        assert!(get_config_path().to_string_lossy().contains("gadv"));
    }
}
