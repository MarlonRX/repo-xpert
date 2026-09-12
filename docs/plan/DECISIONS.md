# DECISIONS — decisiones de arquitectura (CERRADAS 2026-09-12)

| # | Decisión | Resultado |
|---|---|---|
| 1 | Lector del historial | ✅ **`gix`** con Plan B acotado (solo diff) |
| 2 | Corte con git-hero | ✅ **Opción A, corte duro read-only** (criterio premio/esfuerzo) |
| 3 | Modelo de datos / caché | ✅ Esquema propuesto + **JSON con endurecimiento de formato** (§3c) |

Las secciones quedan como registro del razonamiento; no reabrir sin una
nota de cambio aquí arriba.

---

## 1. Lector del historial: `gix` vs `git2` vs seguir shellando a `git`

**Contexto.** El motor necesita: abrir el repo, caminar el DAG de commits,
difuntear trees por commit (adds/dels por ruta) y leer blobs en HEAD (LOC).
El autor está aprendiendo Rust; el dev machine es Windows.

### Opción A — `gix` (puro Rust)

- ✅ Sin C: `cargo build` funciona en Windows sin toolchain extra. Esto solo
  ya pesa mucho para un novato.
- ✅ API idiomática Rust: `Iterator`, `Cow`, `thiserror` propio; el código del
  motor queda "rustiano" (lifetimes honestos, cero `Box<dyn>` escondidos).
- ✅ Diff con rename y blame incluidos en el ecosistema (`gix-diff`,
  `gix::File::blame`).
- ❌ Documentación escasa y API que se mueve entre versiones; ejemplos en la
  web pocos. Curva: dura las primeras 2 semanas, luego agradecerás la pureza.
- ❌ Más tipos con lifetime parameters → más peleas con el borrow-checker.

### Opción B — `git2` (bindings de libgit2)

- ✅ La API es la de libgit2: muchísimos ejemplos (es la base de mucho tooling
  conocido, incluyendo `gitui`), estable y probada.
- ❌ En Windows requiere C toolchain o feature `vendored` con `cc`; el primer
  `cargo build` puede volverse un calvario de errores de linker. Para aprender
  Rust, configurar el build del binding es ruido que no enseña nada.
- ❌ API con semantics de C: callbacks, refcounting interno (`Repository` con
  Drop), más `unwrap()` en examples del mundo real.

### Opción C — seguir shellando a `git` (`git log -p`, `git blame --porcelain`)

- ✅ Curva de aprendizaje plana; el fork ya sabe spawnear procesos.
- ✅ `git` en C es lo más rápido que existe para difuntear.
- ❌ Rompe el enunciado del proyecto: el diferencial es "motor propio", y
  portafolio-wise shelling a un binario no demuestra lectura de objetos.
- ❌ Parsing de `--porcelain` es un pantano propio (delimitadores, renames,
  binary files, encoding) y un segundo "lenguaje" que aprender.

**Recomendación: A (`gix`), con un plan B explícito.** Confiar en que la
fricción del borrow-checker es *el ejercicio*, porque el motor es pequeño y
abierto. Plan B de emergencia: si al final de M1 `gix::diff` bloquea más de
una tarde, se shellea **solo el diff por commit** (Opción C acotada a una
función `diff_commit()` detrás de un trait) y el resto del motor sigue en
`gix`. Nunca se migra a `git2` (Windows + novato = peor error).

---

## 2. Corte de cordón: qué se elimina del fork

**Contexto.** El fork trae un gestor de repos completo: push/pull/fetch,
stage/commit, stash, branches, credenciales vía askpass, CLI operativa.
Todo eso son ~4 módulos y los 900 líneas de `git.rs`.

### Opción A — corte duro (recomendada)

git-advance es **solo lectura**: se eliminan `git.rs` (entero),
`git_error.rs` (se reemplaza por errores del engine), la rama operativa de
`cli.rs`, `run_askpass_helper` en `main.rs`, los modales de confirmación
push/pull/remove, el modal de credenciales, y `check_latest_version` (usa
`git ls-remote`: red + subproceso, dos cosas que el producto promete no hacer).
El usuario que quiere operar abre git-hero.

- ✅ El programa es read-only → sin threading de mutación, sin credenciales,
  sin estados de operación a medio terminar. Un novato modela esto sin morir.
- ✅ Coincide con VISION: el fork existe por la analítica o no existe.
- ❌ Se "desperdicia" código heredado. Contraargumento: lo valioso del fork
  no es `git.rs` (que es un wrapper), es el andamiaje TUI (loop de eventos,
  themes, i18n, paneles).

### Opción B — híbrido: gestor + pestaña de analítica

- ✅ Un solo binario, "más completo" en el README.
- ❌ Dos productos en uno: hay que mantener la UI operativa Y la analítica,
  y la analítica —que es la parte difícil y nueva— compite por las tardes
  con bugs de credenciales y push force.
- ❌ Rompe la promesa de no-red/no-mutación de VISION.
- ❌ Para un novato: mantener el plano de operaciones funcionando mientras
  aprendes grafos y rayon es la receta del proyecto abandonado.

**CERRADA en A (corte duro) — criterio premio/esfuerzo.** El autor eligió
maximizar premio por hora, no funcionalidad total. La cuenta: el 90 % del
premio (analítica) vive en el engine; el plano operativo solo aporta el 10 %
de "no cambiar de app para hacer un commit", pero multiplica ×2 el
mantenimiento y reabre la deuda de threading/credenciales que el novato
tendría que pisar en cada milestone. El híbrido puede existir algún día
publicando `gadv-engine` como crate que git-hero consuma — eso es un proyecto
2027, no un alcance M0–M5. Lo que SÍ se conserva del fork: `ui/` (esqueleto de
eventos/render/estado), `theme.rs`, `i18n.rs` (recortado), `config.rs` (con
`dirs` ya es cache-path seguro), `log.rs`, `version.rs`. Ver SKELETON.md.

---

## 3. Modelo de datos y caché

**Contexto.** Analizar 10k commits con diffs no puede pasar por el event loop.
Hay que decidir (a) qué se guarda por commit, (b) dónde vive el grafo, (c)
cómo se serializa e invalida la caché.

### (a) Commit analizado — representación mínima

El motor NO guarda árboles ni diffs completos en memoria: guarda por commit:

```text
CommitRecord {
  oid: [u8; 20],            // hash crudo, sin String hex (lo hex-disease come)
  time: i64,                // epoch seconds del autor (sin chrono: aritmética entera)
  author: AuthorId(u32),    // interned: tabla paralela de (nombre, email)
  files: Vec<FileStat>,     // FileStat { file: FileId(u32), adds: u32, dels: u32 }
}
```

Las claves de archivo y autor son `u32` internados en tablas (`PathInterner`,
`AuthorInterner`): el coupling manipula millones de pares y `PathBuf` como
clave lo haría 10× más lento y más gordo en caché. Toda métrica es una
reducción sobre `&[CommitRecord]` — puro y paralelizable.

### (b) El grafo de archivos

No existe un "grafo" materializado: **el grafo vive en los datos de churn y
coupling**. Estructuras derivadas tras el parseo:

- `file_stats: HashMap<FileId, {adds, dels, commits, last_time}>` → churn/staleness.
- `ownership: HashMap<(FileId, AuthorId), lines>` → ownership/bus factor.
- `cooc: HashMap<(FileId, FileId), u32>` ordenado (a<b) → coupling; se mantiene
  sparse (HashMap), nunca matriz densa F×F (20k archivos = 400M de celdas).

### (c) Serialización de caché — ✅ JSON aprobado, con endurecimiento de formato

`serde_json` (dependencia ya presente). El autor pidió JSON por comodidad;
los 6 puntos que siguen son el precio de esa comodidad y **son parte de la
decisión**, no detalles de implementación:

1. **Envelope con versión:** el archivo siempre empieza con
   `{ "format_version": N, "head_oid": "...", "commit_count": N, ... }`.
   Cualquier `format_version` distinto del esperado → **descartar y rebuild**
   (NUNCA migrar entre versiones: `--no-cache` hoy es gratis, un parser de
   migraciones no). Subir N es un commit de una línea + rebuild para todos.
2. **Oids SIEMPRE hex en disco** (`serde` con `#[serde(with = "hex_oid")]`).
   Un `Vec<u8>`/`[u8;20]` sin tratar se serializa como array de números:
   3–4× tamaño y parseo lento. En memoria el oid sigue siendo `[u8;20]`
   (§3a); el hex es solo el encoding del formato.
3. **Interning también en disco:** las cadenas (`paths`, `(nombre,email)` de
   autores) aparecen UNA vez como tablas top-level; los records solo llevan
   los `u32`. JSON sin internar repite cada ruta miles de veces: 10k commits
   pasan de ~3 MB a ~40 MB y el load deja de ser <100 ms.
4. **Solo enteros en el formato:** `time` en epoch `i64`, contadores `u32`,
   sin floats y sin fechas string. Todo lo derivado (ratios, scores, shares)
   se recalcula en memoria al cargar: la caché guarda HECHOS, no resultados.
   Así M3/M4/M5 no tocan el formato y el `format_version` rara vez sube.
5. **Compatibilidad de lectura:** todo campo con `#[serde(default)]` + el
   struct con `#[serde(deny_unknown_fields)]` invertido: ignorar campos
   desconocidos (binario viejo leyendo archivo nuevo) pero nunca asumir un
   campo ausente crítico (lo crítico: version y head_oid → sin ellos, error
   explícito y rebuild).
6. **Test de contrato desde M2:** round-trip `write → read → assert_eq` sobre
   el fixture, y un test con un JSON *editable a mano* (`cache_v1_sample.json`
   en `tests/`) que congele el formato: si un cambio rompe ese sample, rompe
   un test — el formato queda documentado en un archivo legible, que es justo
   la ventaja del JSON sobre bincode.

Tamaño esperado: 10k commits ≈ 2–4 MB, load <100 ms. Si algún repo real pasa
de ~20 MB, bincode queda como optimización autorizada (única dep nueva además
de gix/rayon).
- **Ubicación:** `<repo>/.git/git-advance/cache.json` (y un `cache.lock`
  best-effort). Vivir dentro de `.git` lo hace automático por repo y
  sobrevive a `git clean`. Riesgo anotado: `.git` no es tuyo; se documenta y
  se ofrece `--no-cache` / path alternativo en config.
- **Invalidación:** clave = `oid de HEAD` + `version de formato` + `filtro de
  ventana` usado. Si HEAD cambió, se hace **update incremental**: se camina
  desde HEAD hasta encontrar el primer commit presente en caché (walk de
  padres con HashMap de oid vistos — O(delta), sin `merge-base`) y se
  parsean solo los nuevos. Si el historial fue reescrito (rebase/gc) y algún
  oid esperado ya no existe, rebuild total.
- **Cap:** `max_commits` configurable (default 50_000) con aviso en UI.

**Recomendación:** el esquema anterior, con una variante de gusto: en el
motor **no** se introduce todavía el `trait Metric` del anteproyecto. Cada
métrica es una `pub fn churn(&History, Window) -> ChurnReport` concreta; el
trait se extrae solo cuando haya 3 métricas que pidan la misma interfaz
(regla: no abstractizar antes de la tercera repetición). Aprenderás traits
con evidencia, no con anticipación.
