# Repo Xpert

**Git repository analytics in your terminal.**

Which files change the most? Where does high-churn meet high-complexity?
Who owns what code — and how fragile is that knowledge? Which files
secretly co-change together?

Existing Git TUIs (lazygit, gitui, tig) answer *"what changed?"*.
Repo Xpert answers *"what does this repository mean?"* — locally, in
pure Rust, read-only.

```
 repo-xpert   my-repo   HEAD main @ 770655d   [historial completo]
──────────────────────────────────────────────────────────────────────
 [1 Resumen]  [2 Churn]  [3 Hotspots]  [4 Dueño]

 COMMITS                  AUTORES                  MERGES
 10,000                   3                        0

 MAYOR CHURN
 src/tui.rs · 4,080

 HOTSPOT #1
 src/ui/rendering/panels.rs · score 0.80

 RIESGO DE CONOCIMIENTO
 43 de 43 modulos con un solo dueno
```

## What it is

- **A real analytics engine.** The history is read from `.git` objects
  directly with [`gix`](https://github.com/GitoxideLabs/gitoxide) —
  no `git` subprocess, no parsing of porcelain output. Commit diffs,
  renames, binary detection and line counting are computed in Rust.
- **Four core metrics**, each a pure function `history → report`:
  - **Churn** — added+deleted lines per file, the most predictive
    metric of defects.
  - **Hotspots** — churn × complexity (LOC at HEAD, rank-normalized),
    shown as an ASCII scatter: top-right = where bugs live.
  - **Ownership / bus factor** — who wrote what and nobody broke it.
    A repo-level "N of M modules have a single owner" risk number.
  - **Coupling** — files that keep changing in the same commits
    (logical dependencies your `import`s don't show), with Jaccard
    scores.
- **Fast and incremental.** A 10,000-commit repo indexes in ~1.3 s
  (parallel via rayon) and reopens in ~60 ms from a versioned cache
  inside `.git/repo-xpert/`. New commits since last run are indexed
  incrementally.
- **Strictly read-only.** No network, no credentials, no writes to
  your working tree. If you want to *operate* on Git, that's what
  [git-hero](https://github.com/MarlonRX/git-hero) — the sibling
  project this was forked from — is for.

## What it is not

- Not a Git client (no stage/commit/push/pull).
- Not a DORA/process dashboard.
- Not a SaaS. Everything stays on your machine.

## Quick start

```sh
cargo install --path .     # or: cargo run --release
gadv                       # TUI on the current repo
gadv scan ~/code/my-repo    # headless report: churn, hotspots, risk
gadv scan . --neighbors src/ui/state/mod.rs   # co-change partners
```

### TUI keys

| Key | Action |
|---|---|
| `1`–`4` | Resumen · Churn · Hotspots · Dueño |
| `j/k` or `↑/↓` | select a file |
| `Enter` | coupling neighbors of the selected file |
| `t` | cycle time window (all / 90d / 30d) |
| `r` | rescan |
| `q` / `Esc` | back / quit |

## Methodology (the honest fine print)

- Merge commits are skipped when diffing (no double counting).
- Renames are attributed to the **new** path, like `git log -M`.
- Binary files and submodules register as touches, not lines.
- Complexity is an LOC proxy in v0 — cyclomatic via tree-sitter is on
  the roadmap.
- Ownership is a commit-based heuristic, **not blame**. Blame-accurate
  ownership is planned.
- Lockfiles, `vendor/`, `dist/` and mega-commits (>200 files) are
  filtered out of hotspots and coupling by default.

Numbers in this README were measured on a 12-core dev machine with a
synthetic 10k-commit repo (`CHANGELOG.md` keeps the trail).

## Roadmap

Blame-based ownership · cyclomatic complexity (tree-sitter) · export
JSON/CSV · activity heatmap · staleness · multi-repo · publishable
`gadv-engine` crate.

## Credits

Forked from [git-hero](https://github.com/MarlonRX/git-hero) (TUI
scaffolding, themes). Metric definitions follow Adam Tornhill's
*Your Code as a Crime Scene* and *Software Design X-Rays*.

## License

MIT — see [LICENSE](LICENSE).
