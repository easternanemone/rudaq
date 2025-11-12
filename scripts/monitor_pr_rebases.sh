#!/bin/bash
# PR Rebase Monitoring Script
# Tracks progress of Jules agent rebases on rust-daq PRs
# Usage: ./scripts/monitor_pr_rebases.sh

set -e

echo "==================================="
echo "PR Rebase Progress Monitor"
echo "==================================="
echo ""

# Get all open PRs
open_prs=$(gh pr list --json number,mergeable)
open_count=$(echo "$open_prs" | jq 'length')

# Get conflicting PRs (mergeable == "CONFLICTING")
conflicting_prs=$(echo "$open_prs" | jq '[.[] | select(.mergeable == "CONFLICTING")]')
conflicting_count=$(echo "$conflicting_prs" | jq 'length')

# Get mergeable PRs (mergeable == "MERGEABLE")
mergeable_prs=$(echo "$open_prs" | jq '[.[] | select(.mergeable == "MERGEABLE")]')
mergeable_count=$(echo "$mergeable_prs" | jq 'length')

# Get unknown state PRs (still being checked by GitHub)
unknown_prs=$(echo "$open_prs" | jq '[.[] | select(.mergeable == "UNKNOWN")]')
unknown_count=$(echo "$unknown_prs" | jq 'length')

echo "Summary:"
echo "--------"
echo "Total Open PRs:        $open_count"
echo "Ready for Review:      $mergeable_count"
echo "Conflicting (rebase):  $conflicting_count"
echo "Unknown (checking):    $unknown_count"
echo ""

if [[ $mergeable_count -gt 0 ]]; then
    echo "✅ PRs Ready for Review:"
    echo "------------------------"
    gh pr list --json number,title,author,labels,mergeable --jq '.[] | select(.mergeable == "MERGEABLE") | "#\(.number) by @\(.author.login): \(.title) [Labels: \(.labels | map(.name) | join(", "))]"'
    echo ""
fi

if [[ $conflicting_count -gt 0 ]]; then
    echo "⚠️  PRs Still Conflicting (awaiting rebase):"
    echo "--------------------------------------------"
    gh pr list --json number,title,author,mergeable --jq '.[] | select(.mergeable == "CONFLICTING") | "#\(.number) by @\(.author.login): \(.title)"'
    echo ""
fi

if [[ $unknown_count -gt 0 ]]; then
    echo "🔍 PRs with Unknown Status:"
    echo "---------------------------"
    gh pr list --json number,title,mergeable --jq '.[] | select(.mergeable == "UNKNOWN") | "#\(.number): \(.title)"'
    echo ""
fi

# Calculate progress percentage
if [[ $open_count -gt 0 ]]; then
    progress=$((mergeable_count * 100 / open_count))
    echo "Progress: $progress% of PRs ready for review ($mergeable_count/$open_count)"
fi
