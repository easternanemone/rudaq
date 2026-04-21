# PR Workflow

<!--
last-ingested: 2026-04-19
sources:
  - CLAUDE.md §Git Workflow
  - CLAUDE.md §Landing the Plane
see-also:
  - ../invariants.md
  - ./beads.md
-->

**Never push directly to `main`.** Feature branch + draft PR, always.
(Exception: single-file fix ≤ 20 lines may go direct.)

## Branch → commit → PR

```
# Branch name: feat/<bead-id>-description or claude/<slug>
git checkout -b feat/bd-xxxx-short-description

# … edit, test …
cargo fmt --all
cargo clippy --workspace --all-targets --exclude ui --exclude comedi-sys --exclude driver-comedi -- -D warnings
cargo nextest run

git add <specific-files>       # NEVER `git add -A` — risks secrets / build artifacts
git commit -m "..."            # Reference (bd-xxxx) when applicable
git push -u origin HEAD
```

Then open a **draft** PR via the GitHub MCP server
(`mcp__github__create_pull_request`, `draft: true`). Never `gh` CLI — the
MCP server is the authorized channel.

## Commit message style

- Imperative first line, ≤ 72 chars.
- Body explains *why*, not *what*.
- Append trailer:

  ```
  https://claude.ai/code/session_<id>
  ```

- Never `--amend` a pushed commit. Never `--no-verify`.

## PR description template

```
## Summary
- <1-3 bullets: what, why>

## Test plan
- [ ] cargo fmt --all -- --check
- [ ] cargo clippy ... -- -D warnings
- [ ] cargo nextest run
- [ ] <domain-specific checks>

Closes bd-xxxx.
```

## When CI fails

1. Investigate root cause — never `--no-verify` to bypass hooks.
2. Fix forward with a new commit; do not amend a pushed commit.
3. If the failure is pre-existing and unrelated, file a bead and link it from the PR.

## Merging

- Review required. Use `mcp__github__pull_request_review_write` for reviews.
- `mcp__github__enable_pr_auto_merge` once checks are green and the PR is approved.
- Never force-push to `main`. Never force-push to any branch another reviewer is on without notifying them.
