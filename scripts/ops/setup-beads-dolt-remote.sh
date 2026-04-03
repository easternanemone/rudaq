#!/usr/bin/env bash
# One-time (per clone / machine) setup so `bd dolt push` works reliably.
#
# Background:
# - `bd dolt push` requires a Dolt remote named `origin`.
# - Newer beads uses embedded Dolt at `.beads/embeddeddolt/beads/`.
# - Older clones may still have legacy state under `.beads/dolt/beads/`.
# - A broken upgrade can leave the embedded checkout empty even though backup
#   JSONLs still hold the real issue graph. This script repairs that case.
#
# Default: add `origin` as a local file remote under
#   $XDG_DATA_HOME/rust-daq/beads-dolt-origin (fallback: ~/.local/share/...).
# Override for Dolthub / Hosted Dolt:
#   BEADS_DOLT_ORIGIN='https://doltremoteapi.dolthub.com/org/db' bash scripts/ops/setup-beads-dolt-remote.sh
#   BEADS_DOLT_ORIGIN='TheFermiSea/your-db'  # Dolthub short form also works
#
# Hosted Dolt auth: set DOLT_REMOTE_USER and DOLT_REMOTE_PASSWORD if required.
#
set -euo pipefail

for cmd in git bd dolt python3; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: required command '$cmd' is not on PATH." >&2
    echo "       Install it and retry, or use your package manager / https://github.com/steveyegge/beads" >&2
    exit 1
  fi
done

json_field() {
  local field="$1"
  python3 -c '
import json
import sys

field = sys.argv[1]
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(1)
value = data.get(field, "")
if isinstance(value, str):
    print(value)
' "$field"
}

count_jsonl_rows() {
  local path="$1"
  if [[ -f "$path" ]]; then
    wc -l <"$path" | tr -d '[:space:]'
  else
    echo 0
  fi
}

count_table_rows() {
  local repo="$1"
  local table="$2"
  if [[ ! -d "$repo/.dolt" ]]; then
    echo 0
    return
  fi

  (
    cd "$repo"
    dolt sql -r csv -q "select count(*) as n from $table;" 2>/dev/null \
      | tail -n +2 \
      | tr -d '\r[:space:]'
  ) || echo 0
}

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "error: run from a git checkout (git rev-parse --show-toplevel failed)" >&2
  exit 1
fi

BEADS_DIR="$(
  cd "$REPO_ROOT" && bd where --json 2>/dev/null | json_field path || true
)"
if [[ -z "$BEADS_DIR" ]]; then
  BEADS_DIR="$REPO_ROOT/.beads"
fi

discover_live_cli_dir() {
  local db_root=""
  db_root="$(
    cd "$REPO_ROOT" && bd where --json 2>/dev/null | json_field database_path || true
  )"

  local candidates=()
  if [[ -n "$db_root" ]]; then
    candidates+=("$db_root/beads")
  fi
  candidates+=(
    "$REPO_ROOT/.beads/embeddeddolt/beads"
    "$REPO_ROOT/.beads/dolt/beads"
  )

  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -d "$candidate/.dolt" ]]; then
      echo "$candidate"
      return
    fi
  done

  if [[ -n "$db_root" ]]; then
    echo "$db_root/beads"
  else
    echo "$REPO_ROOT/.beads/embeddeddolt/beads"
  fi
}

LIVE_CLI_DIR="$(discover_live_cli_dir)"
mkdir -p "$LIVE_CLI_DIR"
(cd "$LIVE_CLI_DIR" && [[ -d .dolt ]] || dolt init >/dev/null)

restore_embedded_checkout_if_empty() {
  local live_issue_count
  live_issue_count="$(count_table_rows "$LIVE_CLI_DIR" issues)"
  if [[ "$live_issue_count" != "0" ]]; then
    return
  fi

  local backup_issue_rows
  local tracked_issue_rows
  backup_issue_rows="$(count_jsonl_rows "$BEADS_DIR/backup/issues.jsonl")"
  tracked_issue_rows="$(count_jsonl_rows "$BEADS_DIR/issues.jsonl")"

  if [[ "$backup_issue_rows" == "0" && "$tracked_issue_rows" == "0" ]]; then
    return
  fi

  echo "Embedded beads database is empty; restoring from local JSONL backups..."

  local recovery_stamp
  recovery_stamp="$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$BEADS_DIR/recovery"
  cp -a "$LIVE_CLI_DIR/.." "$BEADS_DIR/recovery/embeddeddolt-pre-setup-$recovery_stamp"

  local tmpdir
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' RETURN

  python3 - "$BEADS_DIR" "$tmpdir" <<'PY'
import json
import sys
from pathlib import Path

beads_dir = Path(sys.argv[1])
tmpdir = Path(sys.argv[2])

def load_jsonl(path, transform=None):
    rows = []
    if not path.exists():
        return rows
    with open(path) as f:
        for line in f:
            obj = json.loads(line)
            if transform:
                obj = transform(obj)
            if obj is not None:
                rows.append(obj)
    return rows

def parse_metadata_field(obj):
    if "metadata" in obj and isinstance(obj["metadata"], str):
        raw = obj["metadata"].strip()
        if raw:
            try:
                obj["metadata"] = json.loads(raw)
            except Exception:
                obj["metadata"] = {"_raw": raw}
        else:
            obj["metadata"] = {}
    return obj

def normalize_issue(obj):
    if "_type" in obj:
        return None
    if "ephemeral" in obj:
        obj["ephemeral"] = bool(obj["ephemeral"])
    if "is_template" in obj:
        obj["is_template"] = bool(obj["is_template"])
    if "pinned" in obj:
        obj["pinned"] = bool(obj["pinned"])
    if "no_history" in obj:
        obj["no_history"] = bool(obj["no_history"])
    return parse_metadata_field(obj)

issues_src = beads_dir / "backup" / "issues.jsonl"
if not issues_src.exists():
    issues_src = beads_dir / "issues.jsonl"

payloads = {
    "issues.json": load_jsonl(issues_src, normalize_issue),
    "comments.json": load_jsonl(beads_dir / "backup" / "comments.jsonl"),
    "dependencies.json": load_jsonl(beads_dir / "backup" / "dependencies.jsonl", parse_metadata_field),
    "labels.json": load_jsonl(beads_dir / "backup" / "labels.jsonl"),
    "config.json": load_jsonl(beads_dir / "backup" / "config.jsonl"),
    "events.json": load_jsonl(beads_dir / "backup" / "events.jsonl"),
    "interactions.json": load_jsonl(
        beads_dir / "interactions.jsonl",
        lambda obj: {
            **obj,
            "extra": (
                json.loads(obj["extra"])
                if isinstance(obj.get("extra"), str) and obj["extra"].strip()
                else obj.get("extra")
            ),
        },
    ),
}

for filename, rows in payloads.items():
    if not rows:
        continue
    with open(tmpdir / filename, "w") as out:
        json.dump({"rows": rows}, out)
PY

  local name
  (
    cd "$LIVE_CLI_DIR"
    for name in issues comments dependencies labels config events interactions; do
      if [[ -f "$tmpdir/$name.json" ]]; then
        if dolt schema show "$name" >/dev/null 2>&1; then
          dolt table import -u --file-type json "$name" "$tmpdir/$name.json" >/dev/null
        else
          dolt table import -c --file-type json "$name" "$tmpdir/$name.json" >/dev/null
        fi
      fi
    done
  )

  (
    cd "$REPO_ROOT"
    bd --sandbox vc commit -m "repair embedded beads state during setup" >/dev/null
  )

  trap - RETURN
  rm -rf "$tmpdir"
}

restore_embedded_checkout_if_empty

report_repo_local_backup_reservation() {
  local repo_backup_dir="$BEADS_DIR/backup"
  if [[ ! -d "$repo_backup_dir" ]]; then
    return
  fi

  local repo_backup_url
  repo_backup_url="file://$(cd "$repo_backup_dir" && pwd -P)"

  local hidden_backup_url
  hidden_backup_url="$(
    cd "$LIVE_CLI_DIR" && dolt backup -v 2>/dev/null | awk '$1 == "backup_export" { print $2 }'
  )"

  if [[ "$hidden_backup_url" == "$repo_backup_url" ]]; then
    cat <<EOF
Note: beads auto-backup already reserves $repo_backup_url as Dolt backup 'backup_export'.
      Keep using origin for normal sync. If you need an explicit 'bd backup init'
      destination, choose a different path or cloud URL until gastownhall/beads#2962
      is fixed upstream.
EOF
  fi
}

# Derive the sync branch from .beads/config.yaml (or fall back to "main").
BEADS_CONFIG="$REPO_ROOT/.beads/config.yaml"
SYNC_BRANCH="main"
if [[ -f "$BEADS_CONFIG" ]]; then
  _parsed_branch="$(python3 -c '
import sys, re
for line in open(sys.argv[1]):
    m = re.match(r"^sync-branch:\s*[\"'"'"']?([^\"'"'"'#\s]+)", line)
    if m:
        print(m.group(1))
        break
' "$BEADS_CONFIG" 2>/dev/null || true)"
  if [[ -n "$_parsed_branch" ]]; then
    SYNC_BRANCH="$_parsed_branch"
  fi
fi

ORIGIN_URL="${BEADS_DOLT_ORIGIN:-}"
if [[ -z "$ORIGIN_URL" ]]; then
  DATA_ROOT="${XDG_DATA_HOME:-$HOME/.local/share}"
  DATA_DIR="$DATA_ROOT/rust-daq/beads-dolt-origin"
  mkdir -p "$DATA_DIR"
  ORIGIN_URL="file://$(cd "$DATA_DIR" && pwd -P)"
fi

cd "$REPO_ROOT"

if bd dolt remote list --json 2>/dev/null | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(1)
sys.exit(0 if any(r.get("name") == "origin" for r in data) else 1)
' 2>/dev/null; then
  echo "bd dolt remote 'origin' is already configured."
else
  bd dolt remote add origin "$ORIGIN_URL"
  echo "Added bd dolt remote 'origin' -> $ORIGIN_URL"
fi

ORIGIN_URL="$(
  bd dolt remote list --json 2>/dev/null | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(1)
for remote in data:
    if remote.get("name") == "origin":
        print(remote.get("url", ""))
        break
'
)"

if [[ "$ORIGIN_URL" == file://* ]]; then
  DATA_DIR="${ORIGIN_URL#file://}"
  mkdir -p "$DATA_DIR"

  if [[ ! -d "$DATA_DIR/.dolt" ]]; then
    cp -a "$LIVE_CLI_DIR/.dolt" "$DATA_DIR/.dolt"
    echo "Seeded local file remote from $LIVE_CLI_DIR"
  fi

  push_output="$(
    cd "$LIVE_CLI_DIR" && dolt push -u origin "$SYNC_BRANCH" 2>&1
  )" || push_status=$?
  push_status="${push_status:-0}"

  if [[ "$push_status" -ne 0 ]]; then
    if grep -Eq 'no common ancestor|non-fast-forward' <<<"$push_output"; then
      rm -rf "$DATA_DIR/.dolt"
      cp -a "$LIVE_CLI_DIR/.dolt" "$DATA_DIR/.dolt"
      echo "Reseeded local file remote from the repaired embedded checkout."
      (
        cd "$LIVE_CLI_DIR"
        dolt push -u origin "$SYNC_BRANCH" >/dev/null
      )
    else
      echo "$push_output" >&2
      exit "$push_status"
    fi
  fi
fi

echo "Verifying: bd dolt remote list"
bd dolt remote list
report_repo_local_backup_reservation

echo ""
echo "Next: bd dolt push   (run after issue changes; hooks may call this automatically)"
