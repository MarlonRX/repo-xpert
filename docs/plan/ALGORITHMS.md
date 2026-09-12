# ALGORITHMS — los 4 cálculos núcleo

Convenciones: `C` = commits en ventana, `F̄` = archivos tocados por commit
(típico 3–10, cola pesada en mega-commits), `F` = archivos distintos del repo,
`A` = autores. `FileId/AuthorId` son `u32` internados (DECISIONS §3a).
Todas las funciones son puras sobre `&[CommitRecord]`; I/O solo en la ingesta.

## 0. Ingesta (prerequisito compartido)

```text
ingest(repo_path, max_commits) -> History:
  head := repo.head_commit()
  # walk topológico con worklist + HashSet de oids vistos (DAG, no árbol)
  commits := topo_walk(head, max_commits)

  # los diffs se calculan en paralelo (ver nota rayon abajo):
  for c in commits, c no es merge:
      files := diff(c.tree, c.parents[0].tree)      # por ruta: adds, dels
      # renames: se imputan como adds/dels en la ruta NUEVA (no en la vieja)
      # binarios: adds=dels=0 pero cuentan como "touch"
      records.push(CommitRecord{oid, time, author, files})

  return History{ records(newest→oldest), paths: Vec<String>, authors: Vec<Author> }
```

**Decisión sobre merges:** los merge commits se **ignoran** para diffs
(evita contar dos veces el mismo cambio por ambos lados). Primero-parent
como bandera futura. Coste: O(C) walk + O(Σ F̄) difunteo.

## 1. Churn por archivo

**Entrada:** `History`, ventana temporal `W=[t0,∞)`, nivel (`file`|`dir`).
**Salida:** `Vec<ChurnRow{file, adds, dels, touches, last_time}>` ordenado
por `adds+dels` desc. El churn crudo es `adds+dels`; la variante relativa
(`churn / LOC_head`) se calcula en hotspots, no acá.

```text
churn(history, W):
  agg := HashMap<FileId, Acc>
  for r in history.records:
    if r.time < W.t0: continue            # la ventana es un filtro O(1)
    for f in r.files:
      agg[f.id].adds   += f.adds
      agg[f.id].dels   += f.dels
      agg[f.id].touches += 1
      agg[f.id].last   = max(agg[f.id].last, r.time)
  if nivel == dir: redistribuir cada fila a su prefijo de directorio
  return sort_desc(agg)
```

**Big-O:** O(C·F̄) tiempo, O(F) memoria. **Rayon:** `par_chunks` (chunk ≈ C/8n)
sobre `records`, cada tarea con `agg` local, `reduce` fusionando HashMaps.
Cero locks, merge O(F) por par de chunks. Sin rayon hasta M5: single-thread
debe dar para 10k commits (~2–4 s).

## 2. Hotspots (churn × complejidad)

**Entrada:** resultado de churn, HEAD (para leer blobs), `top_k`.
**Salida:** `Vec<HotspotRow{file, churn, loc, score, rank}>` + puntos para el
scatter (`x=loc`, `y=churn`, marcadores por score).

Complejidad en MVP = **LOC actual del archivo** (proxy, como promete el
anteproyecto §5.4). Los LOC solo se leen para los candidatos, no para todo el repo.

```text
hotspots(history, W, top_k=500):
  ch    := churn(history, W)
  cand  := ch.truncate(top_k)                    # los 500 con más churn
  for f in cand par:                             # rayon: blobs independientes
    loc[f] := read_blob(HEAD.tree / f.path).count(b'\n')   # 0 si ya no existe
  # normalización robusta a outliers: rank percentílico, NO min-max
  score[f] := rank_pct(churn[f]) * rank_pct(loc[f])        # (0,1] × (0,1]
  return sort_desc(score).take(50)
```

**Big-O:** O(C·F̄) compartido con churn + O(K) lecturas de blob +
O(K log K) de sort/ranks. **Rayon:** el mapa de LOC (`into_par_iter` sobre
K≈500 rutas) y la ingesta. La lectura de un blob en HEAD es O(1) con el
objeto ya indexado por gix.

**Trampa documentada:** un `vendor/` enorme infla el eje LOC; los patrones de
ignorados de §4 aplican acá también.

## 3. Ownership y bus factor (heurística por commits)

**Entrada:** `History`, umbral `θ=0.5`. **Salida:** por archivo/módulo:
`OwnershipRow{file, owner, shares: Vec<(AuthorId, f32)>, bus_factor}` y un
agregado de repo (peor K módulos con `lines > mínimo`).

> El anteproyecto pide blame (§5.2). Para M4 se usa la heurística por
> commits, que NO requiere blame y sale gratis del mismo barrido de ingesta.
> La versión por blame es upgrade post-M5 (ver RIESGOS de VISION #3).

```text
kept[a,f] := max(0, Σ adds[a,f]  −  Σ_{o≠a} dels[o,f])   # lo que aportó
                                                        # y nadie rompió
ownership(f):
  total  := Σ_a kept[a,f]
  shares := [(a, kept[a,f]/total) para a con kept>0] ordenado desc
  # bus factor: autores necesarios para cubrir θ del conocimiento
  acc := 0; n := 0
  for (a, s) in shares: acc += s; n += 1; if acc >= θ: break
  return shares, bus_factor := n        # kept==0 para todo → bus_factor := 0 (muerto)
```

`adds/dels` por (autor, archivo) se acumulan en el mismo barrido que el churn
(cuarto campo del `Acc`), así que M4 no re-ingesta nada.

**Big-O:** O(C·F̄) acumulación + O(F·A·log A) en los sorts (A es chico, <50
típico). **Rayon:** igual que churn (chunks + reduce del mapa `(FileId,
AuthorId)`).

## 4. Coupling (co-modificación)

**Entrada:** `History`, ventana, `min_co=3`, `max_files_por_commit=200`,
lista de patrones ignorable (`Cargo.lock`, `*.min.js`, `vendor/`, `dist/`).
**Salida:** aristas `(f_a, f_b, cooc, jaccard)` top-K por nodo + vecinos de un
archivo para el panel de detalle.

```text
coupling(history, W):
  cooc    := HashMap<(u32,u32), u32>     # clave SIEMPRE con a<b (ids internados)
  touches := HashMap<FileId, u32>
  for r in history.records filtrada por W:
    fs := ids únicos de r.files, sin ignorables, ORDENADOS
    if len(fs) > max_files_por_commit: continue   # mega-commit: sin señal
    touches[*fs] += 1
    for i in 0..len(fs):
      for j in i+1..len(fs):
        cooc[(fs[i], fs[j])] += 1
  edges := cooc.filter(|_,n| n >= min_co)
        .map(|pair, n| (pair, jaccard = n / (touches[a]+touches[b]-n)))
  return por nodo: top_5 vecinos por jaccard
```

**Big-O:** ingenuo es O(Σ F̄²) — de ahí la cota de 200 archivos/commit y el
descarte de ignorables: un `package-lock.json` que toca 300 archivos crearía
45k pares de ruido por commit. Con cota: O(C·m²), m=min(F̄,200). El HashMap de
aristas se filtra **in-situ** contra `min_co` al fusionar chunks para que no
explote en memoria.

**Rayon:** mismo patrón map(`par_chunks`)→reduce(HashMap) que churn; la clave
`u32,u32` hace el merge barato. Es el hito donde el ejercicio de optimización
O(n²) del anteproyecto (§5.5) se paga: índices enteros, dedupe por commit,
pares ordenados (mitad de claves), filtro tardío.

## Resumen de paralelización

| Fase | Paralelizable | Patrón | Cuándo |
|---|---|---|---|
| walk topológico | NO (DFS compartido) | secuencial, es barato | — |
| difunteo por commit | SÍ | chunks + thread-local de repo handle | M5 |
| churn / ownership / hotspots / coupling | SÍ | `par_chunks` + merge de HashMaps (reduce) | M5 |
| LOC en HEAD (blobs) | SÍ | `into_par_iter` (independientes) | M5 |
| sorts / ranks / bus factor | NO (ya O(n log n) con n chico) | — | — |

Nota gix: `gix::Repository` no es `Sync`; cada hilo de rayon abre su propio
handle (`thread_local! { repo }`). Es un patrón de 5 líneas, no un proyecto.
