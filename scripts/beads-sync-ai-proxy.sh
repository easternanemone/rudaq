#!/usr/bin/env bash
# Sync beads data to the ai-proxy BeadHub + Dolt instances.
#
# Syncs two targets:
#   1. BeadHub API (PostgreSQL) — coordination layer (dashboard, chat, mail)
#   2. Dolt SQL server — issue database (what bd/bdh agents query directly)
#
# Designed to run:
#   1. As a launchd WatchPaths agent (triggers on issues.jsonl change)
#   2. Manually: bash scripts/beads-sync-ai-proxy.sh
#
# Requires: curl, python3, tailscale connectivity to ai-proxy
# Python venv with pymysql: /tmp/dolt-sync-venv

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ISSUES_JSONL="$REPO_ROOT/.beads/backup/issues.jsonl"
LOCK_FILE="/tmp/beads-sync-ai-proxy.lock"
LOG_FILE="${HOME}/Library/Logs/beads-sync-ai-proxy.log"

# ai-proxy endpoints
BEADHUB_URL="${BEADHUB_URL:-http://100.105.113.58:8080}"
BEADHUB_API_KEY="${BEADHUB_API_KEY:-}"
BEADHUB_WORKSPACE_ID="${BEADHUB_WORKSPACE_ID:-}"
BEADHUB_HUMAN_NAME="${BEADHUB_HUMAN_NAME:-$(id -un 2>/dev/null || echo "${USER:-unknown}")}"
BEADHUB_ROLE="${BEADHUB_ROLE:-developer}"
BEADHUB_ALIAS="${BEADHUB_ALIAS:-alice}"
REPO_ORIGIN="${BEADHUB_REPO_ORIGIN:-$(git -C "$REPO_ROOT" remote get-url origin 2>/dev/null || true)}"

# Dolt SQL servers
LOCAL_DOLT_HOST="${LOCAL_DOLT_HOST:-127.0.0.1}"
LOCAL_DOLT_PORT="${LOCAL_DOLT_PORT:-13418}"
REMOTE_DOLT_HOST="${REMOTE_DOLT_HOST:-100.105.113.58}"
REMOTE_DOLT_PORT="${REMOTE_DOLT_PORT:-3307}"
DOLT_DB="${DOLT_DB:-beads}"

# Python with pymysql
PYTHON="${PYTHON:-/tmp/dolt-sync-venv/bin/python3}"

log() {
    echo "$(date '+%Y-%m-%d %H:%M:%S') $*" | tee -a "$LOG_FILE"
}

require_env() {
    local name="$1"
    if [ -z "${!name:-}" ]; then
        log "ERROR: required environment variable $name is not set"
        exit 1
    fi
}

# Prevent concurrent syncs
cleanup() { rm -f "$LOCK_FILE"; }
trap cleanup EXIT

if [ -f "$LOCK_FILE" ]; then
    pid=$(cat "$LOCK_FILE" 2>/dev/null || echo "")
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        log "SKIP: sync already running (pid=$pid)"
        exit 0
    fi
    log "WARN: stale lock file, removing"
    rm -f "$LOCK_FILE"
fi
echo $$ > "$LOCK_FILE"

if [ ! -f "$ISSUES_JSONL" ]; then
    log "ERROR: $ISSUES_JSONL not found"
    exit 1
fi

require_env BEADHUB_API_KEY
require_env BEADHUB_WORKSPACE_ID

if [ -z "$REPO_ORIGIN" ]; then
    log "ERROR: could not determine REPO_ORIGIN; set REPO_ORIGIN explicitly"
    exit 1
fi

# Check connectivity
if ! curl -s --connect-timeout 5 "$BEADHUB_URL/health" | grep -q '"ok"'; then
    log "ERROR: ai-proxy BeadHub unreachable at $BEADHUB_URL"
    exit 1
fi

ISSUE_COUNT=$(wc -l < "$ISSUES_JSONL" | tr -d ' ')

# ── Phase 1: BeadHub API sync (PostgreSQL — coordination layer) ──
log "Phase 1: Syncing $ISSUE_COUNT issues to BeadHub API..."

RESPONSE=$(python3 -c "
import json, sys
issues_jsonl = open('$ISSUES_JSONL').read().strip()
payload = {
    'workspace_id': '$BEADHUB_WORKSPACE_ID',
    'repo_origin': '$REPO_ORIGIN',
    'sync_mode': 'full',
    'issues_jsonl': issues_jsonl,
    'human_name': '$BEADHUB_HUMAN_NAME',
    'role': '$BEADHUB_ROLE',
    'alias': '$BEADHUB_ALIAS'
}
json.dump(payload, sys.stdout)
" | curl -s --connect-timeout 10 --max-time 120 \
    -X POST "$BEADHUB_URL/v1/bdh/sync" \
    -H "Authorization: Bearer $BEADHUB_API_KEY" \
    -H "Content-Type: application/json" \
    -d @- 2>&1)

if echo "$RESPONSE" | python3 -c "import sys,json; d=json.load(sys.stdin); sys.exit(0 if d.get('synced') else 1)" 2>/dev/null; then
    STATS=$(echo "$RESPONSE" | python3 -c "import sys,json; s=json.load(sys.stdin)['stats']; print(f'inserted={s[\"inserted\"]} updated={s[\"updated\"]} deleted={s[\"deleted\"]}')" 2>/dev/null)
    log "Phase 1 OK: BeadHub synced ($STATS)"
else
    log "Phase 1 ERROR: BeadHub sync failed: $RESPONSE"
fi

# ── Phase 2: Dolt SQL sync (what bd/bdh agents query directly) ──

# Ensure pymysql venv exists
if [ ! -x "$PYTHON" ]; then
    log "Phase 2: Creating pymysql venv..."
    uv venv /tmp/dolt-sync-venv 2>/dev/null
    uv pip install pymysql --python "$PYTHON" 2>/dev/null
fi

# Check local Dolt server is running
if ! nc -z "$LOCAL_DOLT_HOST" "$LOCAL_DOLT_PORT" 2>/dev/null; then
    log "Phase 2 SKIP: local Dolt server not running on $LOCAL_DOLT_HOST:$LOCAL_DOLT_PORT"
    exit 0
fi

# Check remote Dolt server is reachable
if ! nc -z "$REMOTE_DOLT_HOST" "$REMOTE_DOLT_PORT" 2>/dev/null; then
    log "Phase 2 SKIP: remote Dolt server unreachable at $REMOTE_DOLT_HOST:$REMOTE_DOLT_PORT"
    exit 0
fi

log "Phase 2: Syncing Dolt database to ai-proxy..."

DOLT_RESULT=$("$PYTHON" << 'PYEOF'
import pymysql
import sys

LOCAL_HOST, LOCAL_PORT = "127.0.0.1", 13418
REMOTE_HOST, REMOTE_PORT = "100.105.113.58", 3307
DB = "beads"

TABLES = [
    "issues", "events", "dependencies", "labels", "comments", "config",
    "interactions", "metadata", "issue_counter", "child_counters",
    "routes", "federation_peers", "repo_mtimes",
]

try:
    local = pymysql.connect(host=LOCAL_HOST, port=LOCAL_PORT, user="root", db=DB,
                            charset="utf8mb4", cursorclass=pymysql.cursors.DictCursor)
    remote = pymysql.connect(host=REMOTE_HOST, port=REMOTE_PORT, user="root", db=DB,
                             charset="utf8mb4", cursorclass=pymysql.cursors.DictCursor,
                             autocommit=False)
except Exception as e:
    print(f"CONNECTION_ERROR: {e}", file=sys.stderr)
    sys.exit(1)

total = 0
errors = []

for table in TABLES:
    try:
        lcur = local.cursor()
        lcur.execute(f"SELECT * FROM `{table}`")
        rows = lcur.fetchall()

        if not rows:
            continue

        cols = list(rows[0].keys())
        placeholders = ", ".join(["%s"] * len(cols))
        col_names = ", ".join([f"`{c}`" for c in cols])

        rcur = remote.cursor()
        rcur.execute(f"DELETE FROM `{table}`")

        batch_size = 200
        for i in range(0, len(rows), batch_size):
            batch = rows[i : i + batch_size]
            values = [tuple(row[c] for c in cols) for row in batch]
            rcur.executemany(
                f"INSERT INTO `{table}` ({col_names}) VALUES ({placeholders})",
                values,
            )

        remote.commit()
        total += len(rows)
    except Exception as e:
        remote.rollback()
        errors.append(f"{table}: {e}")

# Dolt commit
try:
    rcur = remote.cursor()
    rcur.execute("CALL dolt_commit('-Am', 'sync from local beads database')")
    remote.commit()
except Exception:
    pass  # No changes to commit is fine

local.close()
remote.close()

if errors:
    print(f"PARTIAL: {total} rows, errors: {'; '.join(errors)}")
    sys.exit(1)
else:
    print(f"OK: {total} rows")
PYEOF
)

log "Phase 2 ${DOLT_RESULT}"
log "Sync complete."
