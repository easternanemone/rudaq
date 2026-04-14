#!/usr/bin/env bash
# check-accesskit-quality.sh — AccessKit tree quality gate (bd-ibjvs)
#
# Launches the GUI in mock mode, captures the full accessibility tree via
# ax-bridge, and asserts quality criteria. macOS only (requires AXUIElement).
#
# Usage:
#   bash scripts/ops/check-accesskit-quality.sh [--gui-path <path>]
#   bash scripts/ops/check-accesskit-quality.sh --snapshot  # save golden snapshot
#
# Exit codes:
#   0 — all checks pass
#   1 — quality check failed
#   2 — setup error (missing binary, not macOS, etc.)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
AX_BRIDGE="$REPO_ROOT/tools/mcp-accesskit/ax-bridge"
GUI_BINARY="${GUI_PATH:-$REPO_ROOT/target/debug/rust-daq-gui}"
SNAPSHOT_DIR="$REPO_ROOT/tools/mcp-accesskit/snapshots"
SNAPSHOT_MODE=false
TREE_FILE=""
GUI_PID=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --gui-path) GUI_BINARY="$2"; shift 2 ;;
        --snapshot) SNAPSHOT_MODE=true; shift ;;
        *) echo "Unknown arg: $1"; exit 2 ;;
    esac
done

if [[ "$(uname)" != "Darwin" ]]; then
    echo "SKIP: AccessKit quality check requires macOS (AXUIElement API)"
    exit 0
fi

if [[ ! -x "$AX_BRIDGE" ]]; then
    echo "ERROR: ax-bridge not found at $AX_BRIDGE"
    echo "Build it: cd tools/mcp-accesskit && bash build.sh"
    exit 2
fi

if [[ ! -x "$GUI_BINARY" ]]; then
    echo "ERROR: GUI binary not found at $GUI_BINARY"
    echo "Build it: cargo build -p ui --features standalone"
    exit 2
fi

cleanup() {
    if [[ -n "$GUI_PID" ]]; then
        kill "$GUI_PID" 2>/dev/null || true
        wait "$GUI_PID" 2>/dev/null || true
    fi
    if [[ -n "$TREE_FILE" && -f "$TREE_FILE" ]]; then
        rm -f "$TREE_FILE"
    fi
}
trap cleanup EXIT

# ============================================================================
# Phase 1: Launch GUI in mock mode
# ============================================================================
echo "=== AccessKit Quality Check ==="
echo ""
echo "  GUI:       $GUI_BINARY"
echo "  ax-bridge: $AX_BRIDGE"
echo ""

echo "  Launching GUI in mock mode..."
"$GUI_BINARY" --runtime-mode mock &>/dev/null &
GUI_PID=$!

MAX_WAIT=15
for i in $(seq 1 $MAX_WAIT); do
    sleep 1
    STATUS=$("$AX_BRIDGE" app-status "$GUI_PID" 2>/dev/null || echo '{"running":false}')
    RUNNING=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('running',False))" 2>/dev/null || echo "False")
    if [[ "$RUNNING" == "True" ]]; then
        WINDOW_COUNT=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('window_count',0))" 2>/dev/null || echo "0")
        if [[ "$WINDOW_COUNT" -gt 0 ]]; then
            echo "  GUI ready (pid=$GUI_PID, ${WINDOW_COUNT} window(s))"
            break
        fi
    fi
    if [[ $i -eq $MAX_WAIT ]]; then
        echo "ERROR: GUI did not start within ${MAX_WAIT}s"
        exit 2
    fi
done

sleep 2

# ============================================================================
# Phase 2: Capture accessibility tree
# ============================================================================
echo ""
echo "  Capturing accessibility tree..."
TREE_FILE=$(mktemp /tmp/ax-tree-XXXXXX.json)
"$AX_BRIDGE" tree "$GUI_PID" --depth 10 > "$TREE_FILE"
echo "  Tree captured to $TREE_FILE"

# ============================================================================
# Phase 3: Quality assertions + snapshot comparison (single Python pass)
# ============================================================================

# Determine golden snapshot path
GOLDEN="$SNAPSHOT_DIR/golden-tree.json"
GOLDEN_ARG=""
if [[ -f "$GOLDEN" && "$SNAPSHOT_MODE" != true ]]; then
    GOLDEN_ARG="$GOLDEN"
fi

# All analysis in one Python invocation — avoids subshell variable loss
EXIT_CODE=$(python3 - "$TREE_FILE" "$GOLDEN_ARG" << 'PYEOF'
import json, sys

# ── Tree utilities ──

def count_nodes(node):
    c = 1
    for child in (node.get("children") or []):
        c += count_nodes(child)
    return c

def count_role(node, role):
    c = 1 if node.get("role") == role else 0
    for child in (node.get("children") or []):
        c += count_role(child, role)
    return c

def flatten(node, path="root"):
    yield (path, node)
    for child in (node.get("children") or []):
        role = child.get("role", "?")
        title = child.get("title", "")
        label = f"{role}({title})" if title else role
        yield from flatten(child, f"{path}/{label}")

# ── Load tree ──

tree_path = sys.argv[1]
golden_path = sys.argv[2] if len(sys.argv) > 2 and sys.argv[2] else None

tree = json.load(open(tree_path))
nodes = list(flatten(tree))
total = len(nodes)
failures = 0

print(f"  Nodes: {total}")
print()
print("  Running quality checks...")

# ── Check 1: AXUnknown count ──

unknowns = [(p, n) for p, n in nodes if n.get("role") == "AXUnknown"]
if len(unknowns) > 5:
    print(f"  FAIL: {len(unknowns)} AXUnknown elements (max 5)")
    failures += 1
else:
    print(f"  PASS: {len(unknowns)} AXUnknown elements")
for p, _ in unknowns[:5]:
    print(f"    {p}")

# ── Check 2: Unlabeled buttons ──

unlabeled_buttons = [
    (p, n) for p, n in nodes
    if n.get("role") == "AXButton"
    and not n.get("title")
    and not n.get("description")
]
if len(unlabeled_buttons) > 3:
    print(f"  FAIL: {len(unlabeled_buttons)} unlabeled buttons (max 3)")
    failures += 1
else:
    print(f"  PASS: {len(unlabeled_buttons)} unlabeled buttons")
for p, _ in unlabeled_buttons[:5]:
    print(f"    {p}")

# ── Check 3: Unlabeled interactive elements ──

interactive_roles = {"AXSlider", "AXTextField", "AXTextArea", "AXCheckBox",
                     "AXPopUpButton", "AXComboBox"}
unlabeled = [
    (p, n) for p, n in nodes
    if n.get("role") in interactive_roles
    and not n.get("title")
    and not n.get("description")
]
if unlabeled:
    print(f"  WARN: {len(unlabeled)} unlabeled interactive elements")
    for p, n in unlabeled[:5]:
        print(f"    {p} ({n.get('role')})")
else:
    print("  PASS: all interactive elements have labels")

# ── Check 4: Minimum interactive element count ──

all_interactive = [n for _, n in nodes if n.get("role") in interactive_roles | {"AXButton"}]
if len(all_interactive) < 5:
    print(f"  FAIL: only {len(all_interactive)} interactive elements (expected >= 5)")
    failures += 1
else:
    print(f"  PASS: {len(all_interactive)} interactive elements found")

# ── Check 5: Window present ──

windows = [n for _, n in nodes if n.get("role") == "AXWindow"]
if not windows:
    print("  FAIL: no windows found")
    failures += 1
else:
    print(f"  PASS: {len(windows)} window(s)")

# ── Role distribution ──

print()
roles = {}
for _, n in nodes:
    r = n.get("role", "?")
    roles[r] = roles.get(r, 0) + 1
for r in sorted(roles, key=lambda x: -roles[x])[:10]:
    print(f"  {r}: {roles[r]}")

# ── Snapshot comparison ──

if golden_path:
    print()
    print("  Comparing against golden snapshot...")
    golden = json.load(open(golden_path))
    g_count = count_nodes(golden)
    c_count = total
    g_unknown = count_role(golden, "AXUnknown")
    c_unknown = len(unknowns)

    delta = c_count - g_count
    if abs(delta) > g_count * 0.2:
        print(f"  WARN: node count changed significantly: {g_count} -> {c_count} ({delta:+d})")
    elif delta:
        print(f"  OK: node count: {g_count} -> {c_count} ({delta:+d})")
    else:
        print(f"  OK: node count unchanged ({c_count})")

    unknown_delta = c_unknown - g_unknown
    if unknown_delta > 0:
        print(f"  WARN: AXUnknown increased: {g_unknown} -> {c_unknown} ({unknown_delta:+d})")
    elif unknown_delta < 0:
        print(f"  OK: AXUnknown decreased: {g_unknown} -> {c_unknown} ({unknown_delta:+d})")
    else:
        print(f"  OK: AXUnknown unchanged ({c_unknown})")

# ── Exit code ──

print()
if failures:
    print(f"  FAILED: {failures} check(s) failed")
else:
    print("  All checks passed")

# Print exit code for the shell to capture
print(f"EXIT:{1 if failures else 0}")
PYEOF
)

# ============================================================================
# Phase 4: Snapshot mode
# ============================================================================
if $SNAPSHOT_MODE; then
    mkdir -p "$SNAPSHOT_DIR"
    cp "$TREE_FILE" "$SNAPSHOT_DIR/golden-tree.json"
    echo "  Saved golden snapshot to: $SNAPSHOT_DIR/golden-tree.json"
fi

echo ""
echo "=== Done ==="

# Extract exit code from Python output (last line: "EXIT:0" or "EXIT:1")
RESULT=$(echo "$EXIT_CODE" | grep "^EXIT:" | tail -1)
# Print all lines except the EXIT marker
echo "$EXIT_CODE" | grep -v "^EXIT:"

exit "${RESULT#EXIT:}"
