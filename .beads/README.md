# Beads - AI-Native Issue Tracking

Welcome to Beads! This repository uses **Beads** for issue tracking — a modern, AI-native tool designed to live directly in your codebase alongside your code.

## What is Beads?

Beads is issue tracking that lives in your repo, making it perfect for AI coding agents and developers who want their issues close to their code. No web UI required — everything works through the CLI and integrates seamlessly with git.

**Learn more:** [github.com/steveyegge/beads](https://github.com/steveyegge/beads)

## Quick Start (this repo uses `bd`, not `bdh`)

```bash
# Find ready work
bd ready

# Create / update / close
bd create "Title" --type task --priority 2
bd update bd-xxxx --status in_progress
bd close bd-xxxx --reason "Done"

# Export is automatic; issues live in .beads/issues.jsonl (commit with code)
```

### `bd dolt push` (session / hook workflow)

`bd dolt push` sends the local Dolt history to the remote named **`origin`**. If nothing is configured, you will see:

`fatal: remote 'origin' not found`

**Fix (once per machine / clone):**

```bash
bash scripts/ops/setup-beads-dolt-remote.sh
bd dolt push
```

By default this creates a **local file remote** at  
`$XDG_DATA_HOME/rust-daq/beads-dolt-origin` (usually `~/.local/share/rust-daq/beads-dolt-origin`) so pushes succeed without Dolthub. That directory is **not** in git; it is only on your machine.

**Team / cloud remote:** create a database on [DoltHub](https://www.dolthub.com/) (or your Hosted Dolt instance), then:

```bash
BEADS_DOLT_ORIGIN='TheFermiSea/your-database' bash scripts/ops/setup-beads-dolt-remote.sh
# or a full https://doltremoteapi.dolthub.com/... URL if required
```

If `origin` already exists, the setup script preserves it. To switch remotes:  
`bd dolt remote remove origin --force` then re-run with the desired `BEADS_DOLT_ORIGIN`.

**Embedded backend repair:** on current beads releases this repo uses the embedded Dolt checkout at `.beads/embeddeddolt/beads/`. The setup script now targets that live checkout, not the old legacy `.beads/dolt/beads/` path. If an upgrade leaves the embedded checkout empty while local backup JSONLs still contain the real issue graph, the setup script auto-rehydrates the embedded checkout from `.beads/backup/*.jsonl` and `.beads/interactions.jsonl` before configuring the remote.

**Clone-local file remote hardening:** for the default `file://` remote, the setup script also seeds or reseeds `$XDG_DATA_HOME/rust-daq/beads-dolt-origin` from the live embedded checkout when the local backup remote is missing or has no common ancestor. That keeps `bd dolt push` / `bd dolt pull` on a stable native path without relying on the old hosted sync setup.

**Repo-local backup caveat:** beads auto-backup already uses the repo-local `.beads/backup` directory through a hidden Dolt backup named `backup_export`. Until [gastownhall/beads#2962](https://github.com/gastownhall/beads/issues/2962) is fixed upstream, do **not** point `bd backup init` at `.beads/backup`; use a different filesystem path or a cloud/DoltHub URL for any explicit manual backup destination.

### Worktree canonical DB (rust-daq)

In git worktrees, use the canonical DB wrapper to avoid stale local `.beads` drift:

```bash
scripts/bd-safe.sh where --json
scripts/bd-safe.sh ready --json
scripts/hygiene/beads-worktree-hygiene.sh status
```

Optional: run `bash scripts/ops/setup-beads-dolt-remote.sh` from the **primary** checkout first; worktrees share the same `.beads` data when not using a redirect file.

### Working with Issues

Issues in Beads are:

- **Git-native**: Stored in `.beads/issues.jsonl` and synced like code
- **AI-friendly**: CLI-first design works well with AI coding agents
- **Branch-aware**: Issues can follow your branch workflow
- **Dolt-backed**: Local database under `.beads/` (see `.gitignore`); push history with `bd dolt push` when `origin` is set

## Why Beads?

**AI-native design** — built for AI-assisted development workflows.

**Developer focused** — issues live in your repo; fast and lightweight.

**Git integration** — branch-aware issue tracking and JSONL merge resolution.

## Get Started with Beads (upstream)

```bash
curl -sSL https://raw.githubusercontent.com/steveyegge/beads/main/scripts/install.sh | bash
bd init
bd create "Try out Beads"
```

## Learn More

- **Documentation**: [github.com/steveyegge/beads/docs](https://github.com/steveyegge/beads/tree/main/docs)
- **Quick Start Guide**: Run `bd quickstart`
- **Examples**: [github.com/steveyegge/beads/examples](https://github.com/steveyegge/beads/tree/main/examples)

---

*Beads: Issue tracking that moves at the speed of thought*
