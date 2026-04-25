# Beads / Dolt sync — architecture and recovery

This guide covers how rust-daq's `bd` (beads) issue tracker syncs to the shared Dolt remote, the two failure modes we've hit in production, and how to recover from each.

## Architecture

```
┌────────────────────┐         ┌──────────────────────────────────┐
│ Local machine      │         │ ai-proxy (Tailscale 100.105.…)   │
│                    │         │                                  │
│ bd (embedded mode) │         │ dolt sql-server                  │
│ ↓ writes to        │         │   port 3308 (MySQL protocol)     │
│ .beads/embeddeddolt│   push  │   port 8001 (RemotesAPI)         │
│                    │ ───────►│                                  │
│ .beads/issues.jsonl│         │ ~/.beads/shared-server/rust_daq  │
│  (auto-exported    │   pull  │   ← always checked out for       │
│   git-tracked)     │ ◄───────│     query serving                │
└────────────────────┘         └──────────────────────────────────┘
```

- **Local**: `bd` runs in *embedded mode*, storing data in `.beads/embeddeddolt/`. After each write command, bd auto-pushes commits to the remote and auto-exports `.beads/issues.jsonl` so the JSONL view is committable to git.
- **Remote**: A shared dolt sql-server on `ai-proxy` serves several beads databases (rust_daq, beefcake_swarm, vasp_lsp, …) by holding each one *checked out*. The server writes mutations to working tree, then bd's push protocol updates the branch ref.

## Failure modes

### 1. Orphaned WIP from mode-switch

**Symptom**: `bd dolt push` fails with `Error 1105: target has uncommitted changes. --force required to overwrite`.

**Root cause**: When bd runs in *shared-server mode* (connecting directly to the remote), `bd remember` and other writes go straight into the remote working tree. If bd is later switched back to *embedded mode* without a final `bd dolt commit`, that WIP becomes orphaned — the remote working tree shows tens of unwritten kv.memory rows and (sometimes) a schema migration that no commit references. This blocks any subsequent push because the push protocol refuses to overwrite uncommitted state.

**Recovery**:

```bash
# Inspect what's dirty (requires Python with pymysql)
uv run --with pymysql --no-project python3 - <<'PY'
import pymysql
conn = pymysql.connect(host="100.105.113.58", port=3308, user="root", database="rust_daq")
cur = conn.cursor()
cur.execute("SELECT * FROM dolt_status")
for r in cur.fetchall(): print(r)
cur.execute("SELECT * FROM dolt_diff_summary('HEAD','WORKING')")
for r in cur.fetchall(): print(r)
PY

# If the WIP is real data (e.g., orphaned bd remember entries), preserve it:
uv run --with pymysql --no-project python3 - <<'PY'
import pymysql
conn = pymysql.connect(host="100.105.113.58", port=3308, user="root", database="rust_daq", autocommit=True)
cur = conn.cursor()
cur.execute("CALL DOLT_ADD('-A')")
cur.execute("CALL DOLT_COMMIT('-m', 'preserve orphaned WIP', '--author', 'Brian Squires <squires.b@gmail.com>')")
print("preserved as", cur.fetchone())
PY

# Then sync local
bd dolt pull
bd dolt push
```

If the WIP is empty noise (just stale checkout state from failure mode #2), use `CALL DOLT_RESET('--hard')` instead of preserving it.

### 2. Stale-checkout cycle (recurring)

**Symptom**: After fixing failure mode #1, every `bd dolt push` succeeds — but the *next* push fails with the same error.

**Root cause**: The dolt sql-server holds rust_daq checked out for serving. When a push updates the branch ref to a new HEAD, the server's working tree (which still reflects the old HEAD) is left showing the diff between old-HEAD and new-HEAD. That stale-checkout state shows up as "uncommitted changes" and blocks the next push. The actual data is fine; only the server's view of "what's the working tree" is stale.

**Recovery**: A cron-driven daemon on `ai-proxy` runs `CALL DOLT_RESET('--hard')` on rust_daq every ~10s, discarding the stale checkout state so subsequent pushes succeed. Install via:

```bash
ssh ai-proxy bash < scripts/ops/setup-dolt-wt-keeper.sh
```

The script is idempotent — re-running it just reinstalls cron/script. The reset is safe because the working tree is purely a side-effect of the server holding the database checked out; no real data lives there.

**Verification**:

```bash
# Should succeed every time after a few seconds
for i in 1 2 3 4 5; do
  bd remember "sync test $i"
  sleep 12
  bd dolt push
done
```

Expect 4–5/5 success rate. The remaining race window (the ~10s between cron iterations) self-heals on the next bd command.

### 3. `.beads/` in root `.gitignore` blocking auto-export

**Symptom**: Every `bd` write command prints `Warning: auto-export: git add failed: exit status 1`.

**Root cause**: bd's auto-export tries to `git add .beads/issues.jsonl` so the JSONL view ships with the repo. If the root `.gitignore` blocks `.beads/`, that `git add` fails with "paths are ignored by one of your .gitignore files".

**Recovery**: Don't add `.beads/` to the root `.gitignore`. The `.beads/.gitignore` file is the single source of truth for what's runtime-only inside that directory (dolt working tree, sqlite, credential key, locks, …). Trust it.

If a contributor re-adds `.beads/` to the root `.gitignore`, remove it. The comment block left in `.gitignore` next to the deletion explains why.

## Operational checklist

When `bd` starts emitting warnings:

1. **`auto-export` failures** → check root `.gitignore` for `.beads/` line; remove if found.
2. **`dolt push` failures** → run the inspect snippet above. If WIP is real data, preserve via DOLT_COMMIT; if it's stale checkout state, run DOLT_RESET. Verify the wt-keeper cron is alive on ai-proxy:
   ```bash
   ssh ai-proxy 'crontab -l | grep reset-rust-daq; tail -5 ~/.beads/shared-server/reset-wt.log'
   ```
3. **`bd dolt status` says "embedded mode"** → that's expected for this project. Use direct SQL to inspect remote state when needed.

## See also

- `scripts/ops/setup-dolt-wt-keeper.sh` — installs the wt-keeper cron on the dolt host
- `scripts/ops/setup-beads-dolt-remote.sh` — initial remote setup (run once per clone)
- `.beads/.gitignore` — authoritative list of beads runtime/local files
- bd-pdvjv (closed) — this fix's tracking issue
