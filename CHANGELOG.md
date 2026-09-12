# Changelog

Formato de versiones: hitos del plan (`docs/plan/ROADMAP.md`).

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
