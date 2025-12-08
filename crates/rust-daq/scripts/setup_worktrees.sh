#!/bin/bash
set -e

BASE_DIR=~/code/rust-daq-worktrees

# Fetch all branches
git fetch origin

echo "Creating worktrees for PR review..."

# New PRs (already based on current main)
git worktree add "$BASE_DIR/pr-27" origin/fix-clippy-warnings
git worktree add "$BASE_DIR/pr-28" origin/feature/error-context  
git worktree add "$BASE_DIR/pr-29" origin/feature/validation-module

# Old PRs (need rebasing)
git worktree add "$BASE_DIR/pr-22" origin/fix/fft-architecture
git worktree add "$BASE_DIR/pr-20" origin/daq-31-fft-config
git worktree add "$BASE_DIR/pr-24" origin/docs/add-module-level-docs
git worktree add "$BASE_DIR/pr-21" origin/add-architecture-documentation
git worktree add "$BASE_DIR/pr-19" origin/update-readme-with-examples

echo "✅ Worktrees created!"
git worktree list
