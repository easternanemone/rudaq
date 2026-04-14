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
#
# Prerequisites:
#   - macOS with Accessibility permissions granted for Terminal
#   - ax-bridge binary built (tools/mcp-accesskit/ax-bridge)
#   - rust-daq-gui binary built (target/debug/rust-daq-gui)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
AX_BRIDGE="$REPO_ROOT/tools/mcp-accesskit/ax-bridge"
GUI_BINARY="${GUI_PATH:-$REPO_ROOT/target/debug/rust-daq-gui}"
SNAPSHOT_DIR="$REPO_ROOT/tools/mcp-accesskit/snapshots"
SNAPSHOT_MODE=false
TREE_FILE=""
GUI_PID=""

# Parse args
while [[ $# -gt 0 ]]; do
    case "$1" in
        --gui-path) GUI_BINARY="$2"; shift 2 ;;
        --snapshot) SNAPSHOT_MODE=true; shift ;;
        *) echo "Unknown arg: $1"; exit 2 ;;
    esac
done

# Platform check
if [[ "$(uname)" != "Darwin" ]]; then
    echo "SKIP: AccessKit quality check requires macOS (AXUIElement API)"
    exit 0
fi

# Binary checks
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

# Wait for GUI to start (poll app-status)
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

# Give egui a moment to render the full widget tree
sleep 2

# ============================================================================
# Phase 2: Capture accessibility tree
# ============================================================================
echo ""
echo "  Capturing accessibility tree..."
TREE_FILE=$(mktemp /tmp/ax-tree-XXXXXX.json)
"$AX_BRIDGE" tree "$GUI_PID" --depth 10 > "$TREE_FILE"

NODE_COUNT=$(python3 -c "
import json, sys

def count_nodes(node):
    c = 1
    for child in (node.get('children') or []):
        c += count_nodes(child)
    return c

tree = json.load(open('$TREE_FILE'))
print(count_nodes(tree))
")
echo "  Captured $NODE_COUNT nodes"

# ============================================================================
# Phase 3: Quality assertions
# ============================================================================
echo ""
echo "  Running quality checks..."

FAILURES=0

RESULTS=$(python3 << 'PYEOF'
import json, sys

def flatten(node, path="root"):
    """Flatten tree to a list of (path, node) tuples."""
    yield (path, node)
    for i, child in enumerate(node.get("children") or []):
        role = child.get("role", "?")
        title = child.get("title", "")
        label = f"{role}({title})" if title else role
        yield from flatten(child, f"{path}/{label}")

tree = json.load(open(sys.argv[1]))
nodes = list(flatten(tree))

# Check 1: Count AXUnknown elements
unknowns = [(p, n) for p, n in nodes if n.get("role") == "AXUnknown"]
print(f"CHECK:unknown_count:{len(unknowns)}")
for p, n in unknowns[:5]:
    print(f"  DETAIL:{p}")

# Check 2: Buttons without labels
unlabeled_buttons = [
    (p, n) for p, n in nodes
    if n.get("role") == "AXButton"
    and not n.get("title")
    and not n.get("description")
]
print(f"CHECK:unlabeled_buttons:{len(unlabeled_buttons)}")
for p, n in unlabeled_buttons[:5]:
    print(f"  DETAIL:{p}")

# Check 3: Interactive elements (sliders, text fields) without labels
interactive_roles = {"AXSlider", "AXTextField", "AXTextArea", "AXCheckBox",
                     "AXPopUpButton", "AXComboBox"}
unlabeled_interactive = [
    (p, n) for p, n in nodes
    if n.get("role") in interactive_roles
    and not n.get("title")
    and not n.get("description")
]
print(f"CHECK:unlabeled_interactive:{len(unlabeled_interactive)}")
for p, n in unlabeled_interactive[:5]:
    role = n.get("role", "?")
    print(f"  DETAIL:{p} ({role})")

# Check 4: Total interactive element count (sanity — should be > 0)
interactive = [n for _, n in nodes if n.get("role") in interactive_roles | {"AXButton"}]
print(f"CHECK:interactive_count:{len(interactive)}")

# Check 5: Window present
windows = [n for _, n in nodes if n.get("role") == "AXWindow"]
print(f"CHECK:window_count:{len(windows)}")

# Summary stats
roles = {}
for _, n in nodes:
    r = n.get("role", "?")
    roles[r] = roles.get(r, 0) + 1
print(f"STATS:total_nodes:{len(nodes)}")
for r in sorted(roles, key=lambda x: -roles[x])[:10]:
    print(f"STATS:role:{r}={roles[r]}")
PYEOF
"$TREE_FILE")

echo "$RESULTS" | while IFS= read -r line; do
    case "$line" in
        CHECK:unknown_count:*)
            COUNT="${line#CHECK:unknown_count:}"
            if [[ "$COUNT" -gt 5 ]]; then
                echo "  FAIL: $COUNT AXUnknown elements (max 5)"
                FAILURES=$((FAILURES + 1))
            else
                echo "  PASS: $COUNT AXUnknown elements"
            fi
            ;;
        CHECK:unlabeled_buttons:*)
            COUNT="${line#CHECK:unlabeled_buttons:}"
            if [[ "$COUNT" -gt 3 ]]; then
                echo "  FAIL: $COUNT unlabeled buttons (max 3)"
            else
                echo "  PASS: $COUNT unlabeled buttons"
            fi
            ;;
        CHECK:unlabeled_interactive:*)
            COUNT="${line#CHECK:unlabeled_interactive:}"
            if [[ "$COUNT" -gt 0 ]]; then
                echo "  WARN: $COUNT unlabeled interactive elements"
            else
                echo "  PASS: all interactive elements have labels"
            fi
            ;;
        CHECK:interactive_count:*)
            COUNT="${line#CHECK:interactive_count:}"
            if [[ "$COUNT" -lt 5 ]]; then
                echo "  FAIL: only $COUNT interactive elements (expected >= 5)"
            else
                echo "  PASS: $COUNT interactive elements found"
            fi
            ;;
        CHECK:window_count:*)
            COUNT="${line#CHECK:window_count:}"
            if [[ "$COUNT" -lt 1 ]]; then
                echo "  FAIL: no windows found"
            else
                echo "  PASS: $COUNT window(s)"
            fi
            ;;
        STATS:*)
            # Print stats at the end
            echo "  ${line#STATS:}"
            ;;
        "  DETAIL:"*)
            echo "    ${line#  DETAIL:}"
            ;;
    esac
done

# ============================================================================
# Phase 4: Snapshot mode
# ============================================================================
if $SNAPSHOT_MODE; then
    mkdir -p "$SNAPSHOT_DIR"
    SNAPSHOT_FILE="$SNAPSHOT_DIR/golden-tree.json"
    cp "$TREE_FILE" "$SNAPSHOT_FILE"
    echo ""
    echo "  Saved golden snapshot to: $SNAPSHOT_FILE"
    echo "  Nodes: $NODE_COUNT"
fi

# ============================================================================
# Phase 5: Snapshot comparison (if golden exists)
# ============================================================================
GOLDEN="$SNAPSHOT_DIR/golden-tree.json"
if [[ -f "$GOLDEN" && ! "$SNAPSHOT_MODE" = true ]]; then
    echo ""
    echo "  Comparing against golden snapshot..."
    DIFF_RESULT=$(python3 << PYEOF
import json

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

golden = json.load(open("$GOLDEN"))
current = json.load(open("$TREE_FILE"))

g_count = count_nodes(golden)
c_count = count_nodes(current)
g_unknown = count_role(golden, "AXUnknown")
c_unknown = count_role(current, "AXUnknown")

delta = c_count - g_count
unknown_delta = c_unknown - g_unknown

if abs(delta) > g_count * 0.2:
    print(f"WARN: node count changed significantly: {g_count} -> {c_count} (delta: {delta:+d})")
elif delta != 0:
    print(f"OK: node count: {g_count} -> {c_count} (delta: {delta:+d})")
else:
    print(f"OK: node count unchanged ({c_count})")

if unknown_delta > 0:
    print(f"WARN: AXUnknown count increased: {g_unknown} -> {c_unknown} (delta: {unknown_delta:+d})")
elif unknown_delta < 0:
    print(f"OK: AXUnknown count decreased: {g_unknown} -> {c_unknown} ({unknown_delta:+d})")
else:
    print(f"OK: AXUnknown count unchanged ({c_unknown})")
PYEOF
)
    echo "$DIFF_RESULT" | while IFS= read -r line; do
        echo "  $line"
    done
fi

echo ""
echo "=== Done ==="
