#!/usr/bin/env bash
#
# Validate that the LLM Wiki remains wired to the real workspace shape.
#
# This is intentionally narrow: source files and Cargo metadata are
# authoritative, and the wiki is an orientation layer for agents.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

errors=0

err() {
    echo "ERROR: $*" >&2
    errors=1
}

require_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        err "required command not found: $1"
    fi
}

require_cmd cargo
require_cmd jq

if [ "$errors" -ne 0 ]; then
    exit "$errors"
fi

metadata_file="$(mktemp "${TMPDIR:-/tmp}/rust-daq-cargo-metadata.XXXXXX")"
trap 'rm -f "$metadata_file"' EXIT

cargo metadata --no-deps --format-version 1 > "$metadata_file"

mapfile -t cargo_crates < <(jq -r '.packages[].name' "$metadata_file" | sort)
mapfile -t wiki_crates < <(
    find llm-wiki/crates -maxdepth 1 -name '*.md' ! -name index.md -type f \
        | sed 's#.*/##; s#\.md$##' \
        | sort
)

cargo_count="${#cargo_crates[@]}"
wiki_count="${#wiki_crates[@]}"

if [ "$cargo_count" -ne "$wiki_count" ]; then
    err "Cargo workspace has $cargo_count crates, but llm-wiki/crates has $wiki_count crate pages"
fi

missing_pages="$(comm -23 <(printf '%s\n' "${cargo_crates[@]}") <(printf '%s\n' "${wiki_crates[@]}"))"
stale_pages="$(comm -13 <(printf '%s\n' "${cargo_crates[@]}") <(printf '%s\n' "${wiki_crates[@]}"))"

if [ -n "$missing_pages" ]; then
    err "missing LLM Wiki crate pages:"
    printf '%s\n' "$missing_pages" | sed 's/^/  - /' >&2
fi

if [ -n "$stale_pages" ]; then
    err "stale LLM Wiki crate pages:"
    printf '%s\n' "$stale_pages" | sed 's/^/  - /' >&2
fi

for crate in "${cargo_crates[@]}"; do
    if ! grep -Fq "${crate}.md" llm-wiki/crates/index.md; then
        err "llm-wiki/crates/index.md does not link ${crate}.md"
    fi
done

for entrypoint in AGENTS.md CLAUDE.md GEMINI.md llms.txt .github/copilot-instructions.md; do
    if [ ! -f "$entrypoint" ]; then
        err "missing agent entrypoint: $entrypoint"
        continue
    fi

    if ! grep -Fq "llm-wiki/index.md" "$entrypoint"; then
        err "$entrypoint does not point agents at llm-wiki/index.md"
    fi
done

if git ls-files --error-unmatch llm-wiki/.DS_Store >/dev/null 2>&1; then
    err "llm-wiki/.DS_Store is tracked"
fi

if grep -Eq '25 workspace crates|Rust 1\.75\+' .github/copilot-instructions.md; then
    err ".github/copilot-instructions.md contains stale workspace/toolchain facts"
fi

if ! grep -Fq "30 workspace" llms.txt; then
    err "llms.txt does not mention the current 30 workspace members"
fi

if ! grep -Fq '1.92.0' llms.txt; then
    err "llms.txt does not mention the pinned Rust 1.92.0 toolchain"
fi

if [ "$errors" -ne 0 ]; then
    exit "$errors"
fi

echo "LLM Wiki drift checks passed: $cargo_count crate pages match Cargo metadata."
