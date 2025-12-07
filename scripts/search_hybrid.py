#!/usr/bin/env python3
"""
Hybrid Morph + CocoIndex Search Orchestrator

This script intelligently routes search queries to either Morph (fast code search)
or CocoIndex (comprehensive documentation search) based on query characteristics.

Architecture:
- Morph Warp Grep: Fast code pattern search (~200-500ms)
- CocoIndex: Deep semantic search over indexed documentation (~100-300ms)

Usage:
    # Quick code search (auto-detected)
    ./scripts/search_hybrid.py --query "BoxFuture async callbacks"

    # Comprehensive knowledge search (auto-detected)
    ./scripts/search_hybrid.py --query "Why use V5 parameter reactive system?"

    # Force specific mode
    ./scripts/search_hybrid.py --query "impl Movable" --mode quick
    ./scripts/search_hybrid.py --query "architecture decisions" --mode comprehensive

    # JSON output for tooling
    ./scripts/search_hybrid.py --query "camera exposure" --json
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Literal, TypedDict


class MorphResult(TypedDict):
    """Result from Morph Warp Grep search."""
    file_path: str
    line_start: int
    line_end: int
    content: str
    match_type: str  # 'warp_grep'


class CocoIndexResult(TypedDict):
    """Result from CocoIndex semantic search."""
    filename: str
    source: str
    title: str
    category: str
    summary: str
    similarity: float
    key_concepts: list[str]
    related_files: list[str]
    match_type: str  # 'cocoindex'


class HybridSearchResults(TypedDict):
    """Unified search results from both systems."""
    query: str
    mode: str  # 'quick' | 'comprehensive' | 'hybrid'
    morph_results: list[MorphResult]
    cocoindex_results: list[CocoIndexResult]
    total_results: int
    search_time_ms: int


def detect_query_mode(query: str) -> Literal["quick", "comprehensive"]:
    """
    Auto-detect search mode based on query characteristics.

    Quick mode (code-focused) keywords:
    - implementation, code, function, method, impl, struct, trait, fn
    - async, await, trait implementation
    - specific language constructs

    Comprehensive mode (knowledge-focused) keywords:
    - architecture, design, concept, why, how, what, when
    - explain, understand, guide, documentation
    - decision, pattern, strategy

    Args:
        query: Natural language search query

    Returns:
        'quick' for code search, 'comprehensive' for knowledge search
    """
    query_lower = query.lower()

    # Code-focused keywords
    code_keywords = [
        "implementation", "impl", "code", "function", "method", "fn",
        "struct", "trait", "async", "await", "boxfuture", "parameter",
        "driver", "hardware", "callback", "mutex", "lock", "spawn"
    ]

    # Knowledge-focused keywords
    knowledge_keywords = [
        "architecture", "design", "concept", "why", "how", "what", "when",
        "explain", "understand", "guide", "documentation", "docs",
        "decision", "pattern", "strategy", "philosophy", "approach",
        "overview", "introduction", "getting started", "tutorial"
    ]

    code_score = sum(1 for kw in code_keywords if kw in query_lower)
    knowledge_score = sum(1 for kw in knowledge_keywords if kw in query_lower)

    # Code patterns: contains file extensions, line references, code symbols
    if any(pattern in query_lower for pattern in [".rs", ".toml", "::", "<", ">"]):
        code_score += 2

    return "comprehensive" if knowledge_score > code_score else "quick"


def search_with_morph_warpgrep(query: str, repo_path: str = "/Users/briansquires/code/rust-daq") -> list[MorphResult]:
    """
    Search code using Morph Warp Grep (would use MCP tool in actual implementation).

    NOTE: This is a placeholder. In actual deployment, this would invoke the
    mcp__filesystem-with-morph__warpgrep_codebase_search MCP tool.

    Args:
        query: Code search query
        repo_path: Path to repository root

    Returns:
        List of code search results
    """
    # TODO: Integrate with MCP tool mcp__filesystem-with-morph__warpgrep_codebase_search
    # For now, return empty list with placeholder message
    print(f"   [Morph Warp Grep] Would search for: '{query}'")
    print(f"   [Morph Warp Grep] MCP integration required - add to Claude Code MCP servers")
    return []


def search_with_cocoindex(
    query: str,
    category: str | None = None,
    limit: int = 10
) -> list[CocoIndexResult]:
    """
    Search documentation using CocoIndex semantic search.

    Args:
        query: Natural language query
        category: Optional category filter (architecture/guides/etc)
        limit: Maximum results

    Returns:
        List of document search results
    """
    try:
        # Import CocoIndex flow
        sys.path.insert(0, str(Path(__file__).parent.parent / "cocoindex_flows"))
        from comprehensive_docs_index import search_docs

        results = search_docs(query, limit=limit, category=category)

        # Convert to our result type
        return [
            CocoIndexResult(
                filename=r["filename"],
                source=r["source"],
                title=r["title"],
                category=r["category"],
                summary=r["summary"],
                similarity=float(r["similarity"]),
                key_concepts=r.get("key_concepts", []),
                related_files=r.get("related_files", []),
                match_type="cocoindex"
            )
            for r in results
        ]

    except ImportError as e:
        print(f"   [CocoIndex] Error: CocoIndex not installed or flow not indexed")
        print(f"   [CocoIndex] Run: cd cocoindex_flows && python comprehensive_docs_index.py")
        return []
    except Exception as e:
        print(f"   [CocoIndex] Search error: {e}")
        return []


def hybrid_search(
    query: str,
    mode: Literal["quick", "comprehensive", "auto"] = "auto",
    limit: int = 10
) -> HybridSearchResults:
    """
    Execute hybrid search using Morph + CocoIndex.

    Args:
        query: Natural language search query
        mode: Search mode (auto detects based on query)
        limit: Maximum results per system

    Returns:
        Unified search results
    """
    import time

    start_time = time.time()

    # Auto-detect mode if needed
    if mode == "auto":
        mode = detect_query_mode(query)
        print(f"🔍 Auto-detected mode: {mode}\n")

    results = HybridSearchResults(
        query=query,
        mode=mode,
        morph_results=[],
        cocoindex_results=[],
        total_results=0,
        search_time_ms=0
    )

    # Execute searches based on mode
    if mode == "quick":
        print("⚡ Quick Search (Morph Warp Grep)\n")
        results["morph_results"] = search_with_morph_warpgrep(query)

    elif mode == "comprehensive":
        print("📚 Comprehensive Search (CocoIndex)\n")
        results["cocoindex_results"] = search_with_cocoindex(query, limit=limit)

    # Calculate totals
    results["total_results"] = len(results["morph_results"]) + len(results["cocoindex_results"])
    results["search_time_ms"] = int((time.time() - start_time) * 1000)

    return results


def print_human_readable(results: HybridSearchResults) -> None:
    """
    Print search results in human-readable format.

    Args:
        results: Hybrid search results
    """
    print("\n" + "=" * 80)
    print(f"Query: {results['query']}")
    print(f"Mode: {results['mode']}")
    print(f"Results: {results['total_results']} found in {results['search_time_ms']}ms")
    print("=" * 80)

    # Print Morph Warp Grep results
    if results["morph_results"]:
        print("\n🔧 Code Results (Morph Warp Grep):\n")
        for i, result in enumerate(results["morph_results"], 1):
            print(f"{i}. {result['file_path']}:{result['line_start']}-{result['line_end']}")
            print(f"   {result['content'][:100]}...")
            print()

    # Print CocoIndex results
    if results["cocoindex_results"]:
        print("\n📚 Documentation Results (CocoIndex):\n")
        for i, result in enumerate(results["cocoindex_results"], 1):
            similarity_pct = result['similarity'] * 100
            print(f"{i}. {result['title']}")
            print(f"   📄 {result['filename']}")
            print(f"   📂 Category: {result['category']} | Similarity: {similarity_pct:.1f}%")
            print(f"   💡 {result['summary'][:150]}...")

            if result.get('related_files'):
                print(f"   🔗 Related: {', '.join(result['related_files'][:3])}")

            print()

    if results["total_results"] == 0:
        print("\n❌ No results found. Try:")
        print("   - Different keywords")
        print("   - Broader search terms")
        print("   - Check if CocoIndex is indexed (run: python cocoindex_flows/comprehensive_docs_index.py)")


def main():
    parser = argparse.ArgumentParser(
        description="Hybrid Morph + CocoIndex search for rust-daq",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Auto-detect mode
  ./scripts/search_hybrid.py --query "BoxFuture async callbacks"
  ./scripts/search_hybrid.py --query "V5 architecture decisions"

  # Force specific mode
  ./scripts/search_hybrid.py --query "impl Movable" --mode quick
  ./scripts/search_hybrid.py --query "why use parameters" --mode comprehensive

  # JSON output
  ./scripts/search_hybrid.py --query "camera exposure" --json
        """
    )

    parser.add_argument(
        "--query", "-q",
        required=True,
        help="Search query (natural language)"
    )

    parser.add_argument(
        "--mode", "-m",
        choices=["quick", "comprehensive", "auto"],
        default="auto",
        help="Search mode (default: auto-detect)"
    )

    parser.add_argument(
        "--limit", "-n",
        type=int,
        default=10,
        help="Maximum results per system (default: 10)"
    )

    parser.add_argument(
        "--json",
        action="store_true",
        help="Output JSON format (for tooling)"
    )

    args = parser.parse_args()

    # Execute hybrid search
    results = hybrid_search(args.query, args.mode, args.limit)

    # Output results
    if args.json:
        print(json.dumps(results, indent=2))
    else:
        print_human_readable(results)

    # Exit code based on results
    sys.exit(0 if results["total_results"] > 0 else 1)


if __name__ == "__main__":
    main()
