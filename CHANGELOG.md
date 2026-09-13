# Changelog

## Visual rework — legibilidad + paleta "Advance Ink" (2026-09-12)

- **Tema propio por defecto**: "Advance Ink" (ámbar sobre tinta profunda,
  coral = riesgo, verde-azulado = salud/datos). Los 10 temas del fork siguen.
- **Layout a pantalla completa**: cabecera (repo · HEAD · ventana), barra de
  pestañas `[1 Resumen][2 Churn][3 Hotspots][4 Dueño]`, contenido, pie con
  teclas + ms/fuente del último scan. Antes: una tarjetita centrada.
- **Resumen como dashboard**: KPIs grandes (commits/autores/merges), mayor
  churn, hotspot #1 y riesgo de bus factor en una línea; mini-top-5 churn.
- **Tablas legibles**: encabezados de columna, `#` de ranking, números con
  separador de miles (`4,080`), fila seleccionada resaltada con `▸`,
  barras `▍` consistentes, ↑/↓ además de j/k.
- **Hotspots**: scatter más grande con ejes rotulados ("churn alto/bajo",
  "LOC: 8 → N (escala log)"), top-6 con columnas score/churn/LOC.
- **Dueño**: línea de riesgo grande ("N de M con un solo dueño" en coral),
  tabla con share y líneas-que-nadie-más-conoce rotuladas.
- **Vecinos**: título con el archivo origen + explicación de la escala
  ("1.00 = siempre juntos").
- Estados vacíos y de escaneo con mensajes claros ("escaneando… los repos
  grandes tardan unos segundos"), aviso de terminal muy chica (<48×12).
- Engine intacto: el rework es 100 % capa `ui/` + `theme.rs` + default config.

## F6 — coupling + rayon + cierre (2026-09-12)

Formato de versiones: hitos del plan (`docs/plan/ROADMAP.md`).

## F6 — coupling + rayon + cierre (2026-09-12)

- `engine/metrics/coupling.rs` (ALGORITHMS §4): pares ordenados de `u32`,
  cota de 200 archivos/commit, `min_co=3`, Jaccard; `neighbors()` top-k.
  Test del par siempre-junto.
- UI vista `5` indirecta: cursor `j/k` en churn/hotspots/ownership y
  **Enter** abre los vecinos de co-modificación.
- CLI: `gadv scan <repo> --neighbors <path>` (herramienta de verificación).
- **rayon** en la ingesta: walk secuencial de metadatos + diffs paralelos
  con `thread_local` del handle de gix + internado secuencial.
- Fix de formato **v2**: los `Modification` de subtree se saltan (gix ya
  reporta los hijos; antes metían rutas de directorio fantasma en churn y
  coupling). El sample de contrato pasa a `cache_v2_sample.json`.

### Números medidos (12 cores, debug build)

| Pase | antes F6 | con rayon |
|---|---|---|
| Full 10k commits | 7.0 s | **1.25 s** (5.6×) |
| Cache hit | 62–70 ms | sin cambio (no parsea) |

### DoD de coupling

Vecinos de `src/ui/state/mod.rs` en git-hero: `commands.rs` (0.67),
`rendering/mod.rs` (0.50), `events/mod.rs` (0.46), `suggestions.rs`,
`panels.rs` — el clúster real de UI que coevoluciona. (`keyboard.rs` no
entra al top-5; el plan lo suponía: los datos mandan.)

### Evaluación de cierre (regla VISION)

El motor corre 5.6× más rápido que `git log --numstat` + parsing en el bench
y abre un repo de 10k commits en ~60 ms desde caché. Las 4 métricas núcleo
están vivas en TUI con scatter, barras, shares y vecinos. **El proyecto
sobrevive su propia regla: sigue adelante.** Pendiente de decisión del autor:
nombre público, README con capturas, y split a workspace (post-M5 del plan).

## F5 — ownership + bus factor (2026-09-12)

- `engine/metrics/ownership.rs` (ALGORITHMS §3): kept[a,f] = adds[a] −
  dels[otros→f] (los borrados PROPIOS no descuentan: refactor ≠ destrucción);
  bus factor = autores necesarios para cubrir >50% (cota estricta: un 50/50
  devuelve 2, un 70/30 devuelve 1). 5 tests unitarios.
- **Sin bump de `format_version`:** el plan (5.1) pedía extender el esquema
  con mapas por autor, pero cada `FileStat` ya vive en un record con autor
  único → la métrica sale gratis del barrido existente.
- UI vista `4`: top-30 por riesgo (bf 1 primero), barra de share, agregado
  "N de M módulos con bus factor 1" en rojo. Etiquetado "heurística por
  commits (no blame)" visible.
- CLI `scan`: resumen de riesgo + top-5 bf 1.

### Verificación

Unit: solo-autor→1, 70/30→1, 50/50→2, borrado-total→0 (muerto). En el bench
(archivos de 1 línea reescritos rotativamente) el resultado honesto es
"0 módulos con dueño ≥50 líneas": nadie conserva trabajo ahí.

## F4 — hotspots + scatter + ventanas (2026-09-12)

- `engine/git/blob.rs`: LOC en HEAD (resolución de ruta en tree; borrado o
  binario → 0). Test de conteo con/sin newline final.
- `engine/metrics/hotspots.rs` (ALGORITHMS §2): candidatos top-K por churn,
  score = rank_pct(churn) × rank_pct(LOC), `DEFAULT_IGNORES` (lockfiles,
  vendor, dist, target…). 3 tests: polvo/moleza/hotspot, borrados, ignorables.
- UI vista `3`: scatter ASCII churn×log2(LOC) con los 3 de mayor score en
  warning + ranking top-8. Ventanas `t`: todo/90d/30d como filtro sobre la
  caché (no re-ingesta).
- CLI `scan` imprime top-5 hotspots.

### Verificación DoD

git-hero: #1 `src/ui/rendering/panels.rs` (churn 1987, loc 1315) y #2
`src/ui/modals.rs` — los dos archivos que el autor sabe que "hay que tocar
con cuidado". `scripts/install.sh` (churn alto pero 220 líneas) cae al fondo:
la fórmula discrimina. Bench 10k: cache 67 ms + hotspots instantáneos.

## F3 — caché JSON v1 + worker (2026-09-12)

- `engine/cache.rs`: `scan_with_cache` con envelope `{format_version:1,
  head_oid, commit_count, authors, paths, records}` en
  `.git/git-advance/cache.json` (escritura atómica temp+rename).
- Reglas DECISIONS §3c aplicadas: hex oids, interning en disco, enteros
  only, versión inválida/corrupción → rebuild, sample `cache_v1_sample.json`
  congelado por test de contrato.
- Update incremental con regla de seguridad: el splice solo vale en
  cached[0] (el viejo HEAD); cualquier otro empalme implica historial
  reescrito y dispara rebuild honesto (test `rewritten_history_rebuilds`).
- TUI: escaneo en worker (`thread::spawn` + `mpsc`); la UI nunca bloquea.
- CLI `scan` reporta fuente: full / delta(N) / cache.

### Números medidos (DoD: segunda apertura <100 ms)

| Pase | 10k commits |
|---|---|
| Full (sin cache) | 7.3 s |
| **Cache hit** | **62–70 ms** ✅ |
| Delta (1 commit nuevo) | <200 ms |

## F2 — diffs por commit + churn + panel de barras (2026-09-12)

- `engine/git/diff.rs`: `diff_tree_to_tree` de gix con `track_rewrites`
  (renames como `git -M`), políticas ALGORITHMS §0: merges sin diff,
  renames imputados a ruta nueva, binarios/gitlinks = touch sin líneas.
- `engine/metrics/churn.rs`: `Window` + `churn()` puros, 3 tests.
- `FileStat`/`CommitRecord`/`PathInterner` en el modelo.
- UI: vista Churn (tecla `2`) con top-20 en barras de bloques; `scan`
  imprime top-10 churn.
- `tests/debug_tui.rs`: diagnóstico `#[ignore]` para auditar un repo real.

### Números medidos

| Repo | Commits | Scan con diffs |
|---|---|---|
| git-hero | 58 | ~0.5 s |
| bench sintético | 10 000 | **7.9 s single-thread** |

Verificación: top-5 churn idéntico a `git log --numstat` (4080/2554/2271/
2150/1916). El 7.9 s es el número que justifica la caché de F3.

### Bugs de gix cazados en la fase (para el lector)

- `diff_tree_to_tree` reporta el Addition de un directorio Y el de cada
  archivo anidado: expandir ambos duplica el churn (~2× en el commit
  inicial de git-hero). Solución: ignorar cambios con `entry_mode.is_tree()`.
- Submódulos (gitlink, mode COMMIT): `try_into_blob` falla → tratar oid
  no-blob como touch binario.

## F1 — gix walk + `gadv scan` (2026-09-12)

- `engine/git/walk.rs`: BFS sobre el DAG desde HEAD con `HashSet<[u8;20]>`,
  `max_commits` desde config; extrae oid/epoch del autor/autor internado/
  nº de padres. Único punto del proyecto que importa `gix`.
- `engine/model.rs`: `History`, `CommitMeta`, `AuthorInterner` (u32 ids).
- `src/lib.rs` separado del bin para que los tests de integración consuman
  el engine (y el futuro crate `gadv-engine` sea un movimiento, no un refactor).
- Modo `gadv scan [ruta]` + línea "commits" en la TUI.
- Fixture end-to-end generado en tests (2 autores, merge, rename, binario).

### Números medidos (dev machine Windows, debug build)

| Repo | Commits | Scan |
|---|---|---|
| git-hero | 58 | 38 ms |
| bench sintético (fast-import, 3 autores) | 10 000 | **325–345 ms** |

DoD: <2 s sobre 10k → ✅ con ~6× de margen. El bench queda en
`%TEMP%\gadv-bench` para medir diffs en F2 (regenerable con fast-import).

### API gix 0.87 (dolor registrado para el lector)

- `repo.head_commit()` (no `head().peel_to_id_in_place()`); unborn → Err = NoHead.
- `Commit::parent_ids()` itera `Id<'repo>` → `.detach()` a `ObjectId`.
- `SignatureRef.name/email` son `&BStr` (usar `String::from_utf8_lossy`);
  el epoch sale de `SignatureRef::seconds()`, no de `.time` (que es `&str` cruda).

## F0 — base del fork (2026-09-12)

- Fork read-only `gadv`: TUI mínima (repo + HEAD), temas heredados,
  config con campos forward-compatible, log en temp dir del SO.
- Corte de cordón: sin `git.rs`, askpass, modales operativos ni update-check.
