# Wiki Schema & Workflows

Rules of the road for agents editing `llm-wiki/`. Adapted from
<https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f> to a
codebase context.

## Three-layer model

- **Raw sources** (immutable from the wiki's perspective): code under
  `crates/`, TOML under `config/`, human docs under `docs/`, ADRs under
  `docs/adr/`, handoffs under `.claude/handoffs/`, beads under `.beads/`,
  external vendor SDK docs, merged PRs, and commits.
- **The wiki** (this directory): LLM-generated markdown — summaries, entity
  pages, concept pages, cross-references. Maintained entirely by agents.
- **The schema** (this file + `CLAUDE.md` + `AGENTS.md`): specifies
  conventions, workflows, and invariants.

## Page conventions

Every wiki page starts with a metadata block:

```
# <Page Title>

<!--
last-ingested: 2026-04-19
sources:
  - <repo-relative paths>
see-also:
  - <repo-relative wiki paths>
-->
```

- Keep each page **≤ 200 lines**. Detail belongs in linked sources.
- Links are **repo-relative** so the wiki is portable (`../concepts/parameter.md`, `../../docs/…`).
- No emojis, no AUTO-GENERATED banners, no decorative art.
- Use inline code for identifiers; prefer tables over prose lists when describing capability/factory/feature matrices.
- When a page contradicts a newer source, **update the page**, don't append
  a "note: actually…" section. The wiki is append-**and-revise**.

## Workflows

Three core workflows, triggered by agents (not CI, not hooks — yet).

### 1. Ingest

**When:** a new authoritative source lands: merged PR, new ADR, handoff,
new crate, hardware commissioning note, vendor SDK version bump.

**Steps:**

1. Read the source end-to-end.
2. Identify affected wiki pages — usually 3–10: one concept, one or more
   crates, maybe a driver, maybe an invariant.
3. Update each affected page in place. Preserve back-links.
4. If new entities appear (new crate, new capability trait, new hardware
   machine), create the page(s) and link from `index.md`.
5. Append an entry to `log.md` dated today: source path(s), list of pages
   touched, any new gaps noted.

### 2. Query

**When:** an agent needs context to answer a question or perform a task.

**Steps:**

1. Read `index.md`.
2. Follow the relevant category links (concepts / crates / drivers / hardware / workflows).
3. Use the wiki to target source inspection. Verify implementation behavior
   against source before editing or answering risky questions.
4. Fall back to full-repo semantic/text search when the wiki is silent,
   stale, or too coarse for the question.
5. If the query produces a durable, reusable answer that is not already
   captured, **ingest it** (workflow 1) before closing the task.

### 3. Lint

**When:** opportunistically, or when starting a new epic. Not a CI gate
(yet — that's a follow-up bead).

**Checks:**

- `index.md` reaches every other page under `llm-wiki/` at least once.
- Invariants in `invariants.md` don't contradict `CLAUDE.md`, `AGENTS.md`,
  or `rust-toolchain.toml`.
- Each `crates/<name>.md` corresponds to a workspace member in the root
  `Cargo.toml`; no orphan crate pages; no missing ones.
- No `last-ingested` dates older than 180 days without a refresh entry
  in `log.md`.
- Driver capability matrix in `drivers/*.md` matches
  `docs/reference/driver-capability-matrix.md`.

**Output:** one bead per finding (`bd create --title "llm-wiki lint: …"`).
Don't silently fix large discrepancies — file an issue.

## Branching & commits

The wiki is **append-and-revise in place**, but normal repo workflow still
applies: agents use feature branches and worktrees, never direct edits on
`main` in the primary checkout. `log.md` preserves wiki ingest history
independent of git, and git preserves it too. A wiki-only branch is fine when
the task is specifically documentation or wiki maintenance.

## What belongs here vs `docs/`

- **`llm-wiki/`**: dense, cross-linked, optimized for LLM context windows.
  Entities, concepts, invariants, per-crate pages.
- **`docs/`** (Diataxis: reference / how-to / explanation / tutorials):
  human-facing, narrative, rendered via `book.toml`. Source of truth for
  polished prose.

When both need the same content, `llm-wiki/` links to `docs/` rather than
copies. The wiki's value is *aggregation and cross-linking*, not original
prose generation.
