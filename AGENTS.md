# AGENTS.md

Repository instructions for AI coding agents working in `rust-daq`.

## Startup Protocol

Before non-trivial work:

1. Run `bd prime` and `bd memories`.
2. Read the LLM wiki entry points:
   - [`llm-wiki/index.md`](llm-wiki/index.md)
   - [`llm-wiki/invariants.md`](llm-wiki/invariants.md)
   - the relevant concept, crate, driver, hardware, or workflow pages for the task.
3. Use the wiki to orient, then verify implementation details against source before editing behavior.

The wiki is the agent context index. The codebase is the source of truth. If they conflict, trust the code and update the wiki or file a bead.

## LLM Wiki Rules

- Use [`llm-wiki/index.md`](llm-wiki/index.md) as the first stop for codebase orientation.
- Use [`llm-wiki/schema.md`](llm-wiki/schema.md) before editing the wiki.
- Update affected wiki pages when a durable fact changes, such as crate ownership, feature flags, hardware layout, workflows, or architectural invariants.
- Append an entry to [`llm-wiki/log.md`](llm-wiki/log.md) for wiki ingests or substantial corrections.
- Do not let the wiki replace source inspection for risky changes. Verify against `Cargo.toml`, crate source, configs, workflows, and tests as appropriate.

## Beads Tracking

This repo uses `bd` for all task tracking.

```bash
bd ready
bd show <id>
bd update <id> --claim
bd comments add <id> "evidence or handoff"
bd close <id> --reason "done because..."
```

Rules:

- Create or claim a bead before editing.
- Do not use markdown TODO files, `TodoWrite`, `TaskCreate`, or `MEMORY.md`.
- Use `bd remember` for durable cross-session knowledge.
- Do not use `bd edit`; it opens `$EDITOR` and blocks agents.
- Work is not complete until code and bead state have been pushed. If `bd dolt push` is blocked by remote state, report that explicitly.

## Worktree And Git Workflow

Agents must not work directly on `main` in the primary checkout.

```bash
git worktree add .claude/worktrees/<name> -b <type>/<issue-id>-<slug>
# work, test, commit
git push -u origin HEAD
```

Use a PR for broad docs changes, cross-crate changes, or anything touching more than a small single-file fix. Prefer conventional commit messages such as `docs:`, `fix:`, `feat:`, and `refactor:`.

Never run destructive git commands such as `git reset --hard` or `git checkout -- <path>` unless explicitly asked. Do not revert unrelated user changes.

## Search And Code Reading

- Use `colgrep` first for semantic code search.
- Use `rg` for exact text and file listing.
- Use `sg` / ast-grep for structural Rust searches.
- Prefer `cargo metadata`, `Cargo.toml`, workflow YAML, and source files over stale prose when checking facts.

Examples:

```bash
colgrep "driver registry factory registration" -k 20
rg -n "db-surreal|kv-rocksdb|PulseGenerator" docs llm-wiki
sg -p '$EXPR.unwrap()' --lang rust
```

## Verification

Scale checks to the change:

- Docs-only: `git diff --check`, internal markdown link check, and relevant drift scripts.
- Feature/docs sync: `bash scripts/generate-feature-matrix.sh --check`, `bash scripts/hygiene/check-inventory-drift.sh`, `bash scripts/hygiene/check-doc-drift.sh`.
- Rust changes: `cargo fmt --all -- --check`, `cargo clippy ... -D warnings`, and targeted `cargo nextest`.
- Hardware-sensitive work: use the runbooks in [`llm-wiki/workflows/hardware-testing.md`](llm-wiki/workflows/hardware-testing.md).

## Session Close

Before final response:

1. `git status --short --branch`
2. Run appropriate verification.
3. Commit.
4. Push with `git push -u origin HEAD` for new branches.
5. Update or close beads.
6. Report any blocked bead sync or pending CI plainly.

## Fast References

- Claude guide: [`CLAUDE.md`](CLAUDE.md)
- Gemini guide: [`GEMINI.md`](GEMINI.md)
- LLM entry point: [`llm-wiki/index.md`](llm-wiki/index.md)
- Human docs: [`docs/README.md`](docs/README.md)
- Build/test workflow: [`llm-wiki/workflows/build-test-lint.md`](llm-wiki/workflows/build-test-lint.md)
- PR workflow: [`llm-wiki/workflows/pr-workflow.md`](llm-wiki/workflows/pr-workflow.md)
