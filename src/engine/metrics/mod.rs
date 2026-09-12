// ── Métricas ─────────────────────────────────────────────────────────
// Cada métrica es una función pura sobre &[CommitRecord] (DECISIONS §3).
// Sin trait Metric todavía: se extrae con la tercera repetición real.

pub mod churn;
pub mod hotspots;

/// Ventana temporal: `from` es epoch seconds; `None` = todo el historial.
/// Las ventanas se aplican como filtro sobre la caché, nunca re-ingesta.
#[derive(Debug, Clone, Copy, Default)]
pub struct Window {
    pub from: Option<i64>,
}

impl Window {
    pub const ALL: Window = Window { from: None };

    pub fn last_days(days: i64, now: i64) -> Window {
        Window {
            from: Some(now - days * 86_400),
        }
    }

    pub fn includes(&self, time: i64) -> bool {
        self.from.is_none_or(|from| time >= from)
    }
}
