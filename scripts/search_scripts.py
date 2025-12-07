#!/usr/bin/env python3
"""
Command-line tool for searching Rhai scripts using CocoIndex.

Usage:
    python scripts/search_scripts.py "camera capture"
    python scripts/search_scripts.py "polarization measurement" --limit 5
"""

import sys
import argparse
from pathlib import Path

# Add cocoindex_flows to path
sys.path.insert(0, str(Path(__file__).parent.parent))

from cocoindex_flows.rhai_script_index import search_scripts


def main():
    parser = argparse.ArgumentParser(
        description="Search Rhai scripts using semantic search"
    )
    parser.add_argument("query", help="Natural language search query")
    parser.add_argument(
        "--limit", "-n",
        type=int,
        default=5,
        help="Maximum number of results (default: 5)"
    )
    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Show full summaries and hardware details"
    )

    args = parser.parse_args()

    print(f"Searching for: \"{args.query}\"")
    print("=" * 60)
    print()

    try:
        results = search_scripts(args.query, limit=args.limit)

        if not results:
            print("No results found.")
            return

        for i, result in enumerate(results, 1):
            print(f"{i}. {result['filename']}")
            print(f"   Similarity: {result['similarity']:.3f}")

            if args.verbose:
                print(f"\n   Summary:")
                print(f"   {result['summary']}")

                if result['hardware_stages']:
                    print(f"\n   Stages: {', '.join(result['hardware_stages'])}")
                if result['hardware_cameras']:
                    print(f"   Cameras: {', '.join(result['hardware_cameras'])}")
                if result['hardware_other']:
                    print(f"   Other Devices: {', '.join(result['hardware_other'])}")
            else:
                # Truncate summary for non-verbose mode
                summary = result['summary']
                if len(summary) > 100:
                    summary = summary[:97] + "..."
                print(f"   {summary}")

            print()

    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
