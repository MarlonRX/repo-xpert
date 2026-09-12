# ROADMAP — plan completo por fases (decisiones cerradas, DECISIONS.md)

Motor: **gix** · scope: **read-only puro** · caché: **JSON endurecido v1**.

Reglas transversales (aplican a TODAS las fases):

- **Timebox duro:** ninguna fase pasa de 4 h de código. Si una tarea se
  estira, se parte; si la fase explota 2 veces seguidas, se recorta alcance
  (nunca se "extiende el fin de semana").
- Un commit por tarea (≤200 LOC). Cada commit deja `cargo check`,
  `cargo test` y `cargo clippy --all-targets -- -D warnings` en verde.
- Cada fase termina corriendo su DoD contra un repo real propio Y contra el
  fixture, y anotando los ms medidos en `CHANGELOG.md` (números medidos, no
  estimados — VISION §RIESGOS #6).
- Dependencias: F0 → F1 → F2 → {F3, F4→F5}. F4 no depende de F3.

## F0 / Tarea 0 — Logística del fork (≈2 h, no cuenta timebox)

| # | Tarea | Est. |
|---|---|---|
| 0.1 | Copiar `D:\projects\git-hero` → `D:\projects\git-advance` (hoy vacía); borrar `.git` del origen, `git init` limpio | 15m |
| 0.2 | Renombrar crate/binario a `gadv`; limpiar `Cargo.toml` (repo, keywords, description de analítica) | 30m |
| 0.3 | Corte de cordón (DECISIONS §2A): borrar `git.rs`, `git_error.rs`, `cli.rs` operativo, `run_askpass_helper`, modales push/pull/remove/credentials/update, `check_latest_version`, sidebar repo-manager. Con `gix` como única dep nueva ya declarada en `Cargo.toml` pero SIN usar hasta F1 | 60m |
| 0.4 | Reconstruir `main.rs` mínimo: parse de args estilo fork (`-d`, `--no-cache`) y TUI mostrando solo nombre del repo + HEAD | 15m |

**DoD:** `cargo run` abre el TUI recortado sobre cualquier repo; 0 warnings.

## F1 / M0 — gix lee `.git`, sin métricas (timebox 3 h)

| # | Tarea | Est. |
|---|---|---|
| 1.1 | `engine/git/mod.rs` + `error.rs`: `gix::open()` con mapeo a `EngineError{NotGit, NoHead, ...}` (thiserror). Abrir, resolver HEAD, leer commit raíz | 30m |
| 1.2 | `engine/git/walk.rs`: recorrido worklist + `HashSet<[u8;20]>`, newest→oldest, con `max_commits` (default 50k). Extraer `(oid, time_i64, author{name,email})` | 60m |
| 1.3 | `engine/model.rs`: `History`, `AuthorInterner`, `PathInterner` (paths todavía vacíos) | 30m |
| 1.4 | Fixtures: `tests/fixtures/make.ps1|sh` — repo de 10 commits, 2 autores, 1 merge, 1 rename, 1 binario; test `walk` == 10 (merge incluido en el conteo, sin diffs aún) | 45m |
| 1.5 | `gadv scan [ruta]`: imprime `N commits · ms · top author`. Medir sobre git-hero y sobre un repo ≥10k | 15m |

**DoD:** scan de 10k commits en <2 s; fixture da los números exactos.
**Checkpoint de riesgo:** si 1.2 pide más de 2 h en la API de gix → usar el
ejemplo `gix/log` del repo del crate como plantilla; NO migrar a git2.
**Conceptos:** `Result`/`?`, thiserror, HashSet de visited, Cow.

## F2 / M1 — Diffs + churn con barras en TUI (timebox 4 h)

| # | Tarea | Est. |
|---|---|---|
| 2.1 | `engine/git/diff.rs`: tree vs primer padre con gix-diff → `Vec<FileStat>`; política: merge se saltea, rename se imputa a ruta nueva, binario = touch sin líneas (ALGORITHMS §0/§1) | 90m |
| 2.2 | Enruta diff en el walk: `CommitRecord.files` completo; `gadv scan` ahora imprime también `total_filestats` | 30m |
| 2.3 | `engine/metrics/churn.rs` (ALGORITHMS §1, single-thread) + 3 tests con fixture (suma por ruta, ventana, agregado por dir) | 60m |
| 2.4 | `ui/panels/churn.rs`: tecla `1` → top-20 con `BarChart`; re-escaneo síncrono a mano (botón `r`); el texto "escaneando…" puede congelar el frame 2-4 s: aceptable, F3 lo mata | 60m |
| 2.5 | Verificación manual contra `git log --numstat` (muestra 5 archivos) y registro de ms | 30m |

**DoD:** top-20 plausible en git-hero verificado a mano; tests verdes.
**Checkpoint Plan B (DECISIONS §1):** si 2.1 no funciona en 90 m, aislar el
diff detrás de `fn diff_commit()` y shellear SOLO eso (`git show --numstat`),
con TODO y número de issue. La migración de vuelta a gix es post-F5.
**Conceptos:** iteradores sobre trees, enums de cambio, HashMap con reduce.

## F3 / M2 — Caché JSON v1 + worker (timebox 3 h)

| # | Tarea | Est. |
|---|---|---|
| 3.1 | `engine/cache.rs`: escribir `History` con envelope `{format_version:1, head_oid, commit_count, paths[], authors[], records[]}` → `.git/git-advance/cache.json`. Cumplir las 6 reglas de DECISIONS §3c (hex oid, interning en disco, enteros only, `serde(default)`) + `cache_v1_sample.json` en tests/ | 60m |
| 3.2 | Load: validar envelope → `deny/rebuild` en mismatch; round-trip test `write→read→assert_eq` | 30m |
| 3.3 | Update incremental: walk desde HEAD hasta empalmar con oid cacheado; parsear solo delta; test: commit nuevo al fixture → delta=1 | 45m |
| 3.4 | Worker: `thread::spawn` + `mpsc` (`Scanned(n)`/`Done(History)`); UI pinta datos viejos + progreso; `--no-cache` | 30m |
| 3.5 | Tests de invalidación (borrar cache, HEAD sucio → rebuild) + medir carga | 15m |

**DoD:** segunda apertura <100 ms con el repo cerrado (prueba sin I/O de más).
**Conceptos:** derive(serde) con atributos `with`, channels, `'static` honesto.

## F4 / M3 — Hotspots + scatter (timebox 4 h)

| # | Tarea | Est. |
|---|---|---|
| 4.1 | `engine/git/blob.rs`: LOC de ruta en HEAD (contar `\n` del blob; 0 si borrado) | 30m |
| 4.2 | `engine/metrics/hotspots.rs` (ALGORITHMS §2): candidatos top-500, rank percentílico × rank LOC; tests con fixture (polvorín chico vs moleza estable) | 60m |
| 4.3 | `ui/panels/hotspots.rs`: `Chart` scatter x=LOC y=churn + ranking top-20 al lado; selección = punto más cercano (O(K)) | 90m |
| 4.4 | Ventanas temporales: `t` cicla todo/90d/30d como `Window{from:i64}` que filtra records (no re-ingesta); afecta a churn y hotspots | 45m |
| 4.5 | Ignorables (`Cargo.lock`, `vendor/`, `dist/`) en `config.rs` aplicados a candidatos | 15m |
| 4.6 | Medición + CHANGELOG | 15m |

**DoD:** el #1 del ranking es un archivo que el autor admitiría como
"peligroso" en una review; redibujado sin lag (acá se prueba que F3 no miente).
**Conceptos:** f32 con total ordering, sort_by, filtrado por config.

## F5 / M4 — Ownership + bus factor (timebox 4 h)

| # | Tarea | Est. |
|---|---|---|
| 5.1 | Extender `FileStat`/`CommitRecord` con el mapa por autor (adds/dels por `(FileId,AuthorId)`); **sube `format_version` a 2** (rebuild automático, sin migración) | 45m |
| 5.2 | `engine/metrics/ownership.rs` (ALGORITHMS §3): kept netos clampados, shares, bus factor θ=0.5; tests: fixture con 2 autores → bf 1 y bf 2 | 60m |
| 5.3 | `ui/panels/ownership.rs`: tabla top-30 (riesgo arriba), barra de shares, bf de repo en grande rojo si =1; etiqueta visible "heurística por commits (no blame)" | 75m |
| 5.4 | Doc en `--help` + medición | 30m |

**DoD:** owner primario de los 3 archivos más grandes coincide con
`git log --format=%an -- <archivo>` a ojo. **Conceptos:** HashMap anidado,
ordenamientos por tupla, serde v2.

## F6 / M5 — Coupling + rayon + cierre (timebox 4 h)

| # | Tarea | Est. |
|---|---|---|
| 6.1 | `engine/metrics/coupling.rs` (ALGORITHMS §4): pares ordenados u32, cota 200 archivos/commit, `min_co=3`, Jaccard; tests con fixture (par siempre-junto / par nunca-junto) | 60m |
| 6.2 | `ui/panels/coupling.rs`: vecinos del archivo seleccionado (estado global del AppState, desde cualquier panel) top-5 por Jaccard | 60m |
| 6.3 | rayon: `par_chunks`+reduce en ingesta/churn/ownership/coupling y LOC en hotspots; thread-local del handle gix. ANTES de tocar nada: registrar tiempos F6.1-single vs fixture-grande | 75m |
| 6.4 | Bench antes/después + números al CHANGELOG; `cargo clippy --all-features` | 30m |
| 6.5 | Evaluación de cierre: ¿sobrevive el proyecto? (regla VISION: sin analítica que impresione, se cierra y se vuelve a git-hero). Si sí: decidir nombre público, README con el scatter ASCII, y split a workspace (`crates/gadv-engine`) | 45m |

**DoD:** vecinos de `src/ui/state/mod.rs` de git-hero incluyen `panels.rs` y
`keyboard.rs`; speedup ≥2× en ≥4 cores sobre 10k commits.
**Conceptos:** `into_par_iter`, Send/Sync, `thread_local!`, criterio de cierre.

## Post-F6 (cola, por orden de valor — fuera de los timeboxes)

blame real para ownership exacto · export `--json`/CSV de reportes ·
activity/heatmap · staleness · modo `--first-parent` · crate público
`gadv-engine` consumido por git-hero (híbrido "por la puerta de atrás", el
único híbrido aprobado).

## Calendario mínimo

6 sesiones de ≤4 h + la logística: **F0 un sábado, F1–F6 una tarde cada una.**
Cualquier fase bloqueada >4 h se reporta y se recorta (nunca se encadena
madrugando).
