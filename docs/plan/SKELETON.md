# SKELETON — árbol de módulos propuesto

Una sola decisión de fondo: **empezar con un crate binario único con dos
mundos que no se tocan** (`engine/` puro y `ui/` de presentación) y dividir
en workspace (`crates/gadv-engine` + `crates/gadv-bin`) **solo después de
M3**, cuando la API del engine haya estabilizado. Para un novato, un crate
con `mod engine` y `mod ui` es 10× menos fricción que un workspace con
dependencias cruzadas; el límite igual de duro lo impone la regla de que
`engine/` no tiene `use ratatui` y `ui/` no tiene `use gix`.

## Árbol

```text
gadv/                          (nombre de crate transitorio)
├── Cargo.toml                 # deps nuevas SOLO: gix, rayon (en M5)
│                              # ya del fork: ratatui, crossterm, serde,
│                              # serde_json, thiserror, dirs, phf, unicode-width
├── src/
│   ├── main.rs                # CLI de arranque: gadv [scan] [ruta] [--no-cache]
│   │                          #   modo por defecto = TUI; sin askpass, sin --cli
│   ├── engine/                # ◄── NÚCLEO PURO: ni I/O de terminal ni gix
│   │   ├── mod.rs             #   API pública: open()/scan()/métricas (tipos propios)
│   │   ├── error.rs           #   EngineError (thiserror): RepoNotFound, NotGit,
│   │   │                      #     CacheCorrupt, MaxCommitsExceeded
│   │   ├── git/               #   ÚNICO lugar con `use gix`
│   │   │   ├── mod.rs         #     Repo handle + thread_local para rayon (M5)
│   │   │   ├── walk.rs        #     topo walk HEAD→padres (M0)
│   │   │   ├── diff.rs        #     tree-vs-parent1 -> FileStat (M1)
│   │   │   └── blob.rs        #     LOC en HEAD, lazy (M3)
│   │   ├── model.rs           #   CommitRecord, FileId/AuthorId, PathInterner,
│   │   │   │                  #     History (serde-friendly) — DECISIONS §3a
│   │   │   │                  #     + los Acc por métrica
│   │   ├── cache.rs           #   JSON en .git/git-advance/, incremental (M2)
│   │   └── metrics/
│   │       ├── mod.rs         #   Window { from: i64 } + helpers de ranking
│   │       ├── churn.rs       #   M1
│   │       ├── hotspots.rs    #   M3
│   │       ├── ownership.rs   #   M4   (sin trait Metric todavía — DECISIONS §3)
│   │       └── coupling.rs    #   M5
│   ├── ui/                    # ◄── heredado del fork, recortado
│   │   ├── mod.rs             #   run(): setup/restore de terminal, event loop
│   │   ├── app.rs             #   AppState analítico: vista activa, repo,
│   │   │   │                  #     History cargado, selección, progreso
│   │   │   │                  #     (worker M2 + mpsc)
│   │   │   ├── events/        #   keyboard.rs / mouse.rs (del fork, sin
│   │   │   │                  #     modales de operación)
│   │   │   └── panels/        #   churn.rs (M1) · hotspots.rs (M3) ·
│   │   │                      #   ownership.rs (M4) · coupling.rs (M5) ·
│   │   │                      #   summary.rs (barra de estado con n/oid/ms)
│   │   └── widgets/           #   barras, scatter (Chart), tabla-shares —
│   │                          #     funciones puras (area, data, theme)
│   ├── theme.rs               #   del fork tal cual (10 temas, phf lookup)
│   ├── i18n.rs                #   del fork, recortado a las keys de analítica
│   ├── config.rs              #   del fork (dirs): max_commits, ignorables,
│   │                          #     no_cache, theme, lang
│   ├── log.rs                 #   del fork tal cual (debug a archivo)
│   └── version.rs             #   del fork, sin check_latest_version (es red)
└── tests/
    ├── fixtures/              #   repo git generado por script (M0): 2 autores,
    │   └── make.sh            #   merges, rename, binario, mega-commit
    └── engine_*               #   un archivo de tests por métrica (unit, puros)
```

## Qué se trae del fork (git-hero)

| Del fork | Destino | Nota |
|---|---|---|
| `ui/mod.rs` (loop crossterm/ratatui, draw, poll) | `ui/mod.rs` | quitar `check_git_changes`, askpass drain, console_output |
| `ui/events/` (keyboard, mouse) | `ui/app.rs::events` | solo navegación; borrar handlers de modales operativos |
| `ui/rendering/` (layout, borders, soften, dim) | `ui/` genérico | la base visual vale; los paneles de dashboard NO |
| `theme.rs`, `i18n.rs`, `config.rs`, `log.rs`, `version.rs` | mismo nombre | recortes chicos |
| patrón worker+`mpsc` de `run_git_async` | `ui/app.rs` (M2) | como modelo de arquitectura, reimplementado chico |

## Qué se elimina del fork (DECISIONS §2A)

`git.rs` completo (900 líneas de shell), `git_error.rs`, `cli.rs` (flujo
operativo), `run_askpass_helper`, modales push/pull/remove/credentials/
update-modal, `check_latest_version`, sidebar de repo-manager y sus
flat-entries/file-tree/diff-viewer, CI de empaquetado gith (homebrew/AUR)
hasta que exista nombre público.

## Reglas de frontera (hacen clippy/CI desde M1)

1. `engine/` no importa `ratatui`, `crossterm` ni `std::process`.
2. `ui/` no importa `gix` ni nada de `engine::git` — solo `engine::*` público.
3. Todo parseo de datos Git termina como tipos del engine (`CommitRecord`),
   nunca `String` sueltos hacia la UI.
4. Un test de arquitectura lo enforcea: `tests` que grepea `use gix::` fuera
   de `engine/git/` (3 líneas de Rust, lo escribe M1).

## Orden de creación sugerido (calza con ROADMAP)

Tarea 0: copiar fork + borrado + renombre → M0: `engine/git/{mod,walk}` +
`model.rs` + `main.rs` scan → M1: `engine/git/diff` + `metrics/churn` +
`ui/panels/churn` → M2: `cache.rs` + worker → M3: `git/blob` +
`metrics/hotspots` + scatter → M4: `ownership` → M5: `coupling` + rayon +
migrar a workspace si la API del engine ya no cambia.
