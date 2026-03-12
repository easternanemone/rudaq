#!/usr/bin/env bash

set -euo pipefail

# Keep cargo-audit ignores aligned with deny.toml until cargo-audit can consume
# the same advisory policy directly.
CARGO_AUDIT_IGNORES=(
  RUSTSEC-2025-0134
  RUSTSEC-2025-0141
  RUSTSEC-2024-0436
  RUSTSEC-2024-0370
)

audit_args=()
for advisory in "${CARGO_AUDIT_IGNORES[@]}"; do
  audit_args+=(--ignore "$advisory")
done

# ui-slint is an evaluation-only crate with an unresolved upstream Slint license
# decision; exclude it from the release hygiene gate until that work lands.
cargo_deny_args=(
  --exclude ui-slint
)

metadata_file="$(mktemp "${TMPDIR:-/tmp}/rust-daq-cargo-metadata.XXXXXX")"
trap 'rm -f "$metadata_file"' EXIT

echo "== cargo audit =="
cargo audit "${audit_args[@]}"

echo
echo "== cargo deny (advisories, bans, licenses, sources) =="
cargo deny "${cargo_deny_args[@]}" check advisories bans licenses sources

echo
echo "== cargo machete =="
cargo metadata --format-version 1 --locked > "$metadata_file"
cargo machete --with-metadata "$metadata_file"
