#!/usr/bin/env python3
"""Semantic search over the rust-daq CocoIndex PostgreSQL index.

Usage:
    # Semantic search (requires embedding server)
    python query_cocoindex.py "error recovery retry logic" --top-k 10

    # Text search (no embedding server needed)
    python query_cocoindex.py "DriverFactory" --mode text --top-k 10

    # Crate-scoped semantic search
    python query_cocoindex.py "frame allocation" --crate common --top-k 5

    # Stats
    python query_cocoindex.py --stats

    # Health check
    python query_cocoindex.py --health
"""

import argparse
import json
import sys
import urllib.request
import urllib.error

DB_URL = "postgresql://briansquires@localhost/cocoindex"
EMBED_URL = "http://10.0.0.21:8081/v1"
EMBED_MODEL = "nomic-embed-code.Q8_0"


def get_connection():
    import psycopg
    from pgvector.psycopg import register_vector
    conn = psycopg.connect(DB_URL)
    register_vector(conn)
    return conn


def embed_query(text: str) -> list[float]:
    """Get embedding vector from the cluster GPU."""
    payload = json.dumps({
        "model": EMBED_MODEL,
        "input": text,
    }).encode()
    req = urllib.request.Request(
        f"{EMBED_URL}/embeddings",
        data=payload,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        data = json.loads(resp.read())
    return data["data"][0]["embedding"]


def semantic_search(query: str, top_k: int = 10, crate: str | None = None):
    """Semantic similarity search using pgvector."""
    vec = embed_query(query)
    conn = get_connection()
    crate_filter = ""
    params: list = [vec, vec, top_k]
    if crate:
        crate_filter = "AND filename LIKE %s"
        params = [vec, f"crates/{crate}/%", vec, top_k]

    with conn.cursor() as cur:
        cur.execute(
            f"""
            SELECT filename, language, chunk_content,
                   1.0 - (embedding::halfvec(3584) <=> %s::halfvec) AS score
            FROM code_chunks
            WHERE embedding IS NOT NULL {crate_filter}
            ORDER BY embedding::halfvec(3584) <=> %s::halfvec
            LIMIT %s
            """,
            params,
        )
        return cur.fetchall()


def text_search(query: str, top_k: int = 10, crate: str | None = None):
    """ILIKE text search (no embedding server needed)."""
    conn = get_connection()
    crate_filter = ""
    params: list = [f"%{query}%", top_k]
    if crate:
        crate_filter = "AND filename LIKE %s"
        params = [f"%{query}%", f"crates/{crate}/%", top_k]

    with conn.cursor() as cur:
        cur.execute(
            f"""
            SELECT filename, language, chunk_content, 0.0 AS score
            FROM code_chunks
            WHERE chunk_content ILIKE %s {crate_filter}
            LIMIT %s
            """,
            params,
        )
        return cur.fetchall()


def show_stats():
    """Print index statistics."""
    conn = get_connection()
    with conn.cursor() as cur:
        cur.execute("SELECT count(*) FROM code_chunks")
        total = cur.fetchone()[0]
        cur.execute("SELECT count(DISTINCT filename) FROM code_chunks")
        files = cur.fetchone()[0]
        cur.execute(
            "SELECT language, count(*) FROM code_chunks "
            "GROUP BY language ORDER BY count(*) DESC"
        )
        langs = cur.fetchall()
    print(f"Total chunks: {total}")
    print(f"Total files:  {files}")
    print(f"\nBy language:")
    for lang, count in langs:
        print(f"  {lang:<12} {count:>6}")


def health_check():
    """Check all system components."""
    checks = {}

    # PostgreSQL
    try:
        conn = get_connection()
        with conn.cursor() as cur:
            cur.execute("SELECT count(*) FROM code_chunks")
            n = cur.fetchone()[0]
        checks["postgres"] = f"OK ({n} chunks)"
        conn.close()
    except Exception as e:
        checks["postgres"] = f"FAIL: {e}"

    # Embedding server
    try:
        req = urllib.request.Request(
            f"{EMBED_URL.replace('/v1', '')}/health", method="GET"
        )
        with urllib.request.urlopen(req, timeout=5) as resp:
            data = json.loads(resp.read())
        checks["embedding"] = f"OK ({data.get('status', 'unknown')})"
    except Exception as e:
        checks["embedding"] = f"FAIL: {e}"

    # Launchd service
    import subprocess
    try:
        result = subprocess.run(
            ["launchctl", "list", "com.briansquires.cocoindex-rustdaq"],
            capture_output=True, text=True, timeout=5,
        )
        if "PID" in result.stdout:
            import re
            pid_match = re.search(r'"PID"\s*=\s*(\d+)', result.stdout)
            pid = pid_match.group(1) if pid_match else "?"
            checks["launchd"] = f"OK (PID {pid})"
        else:
            checks["launchd"] = "NOT RUNNING"
    except Exception as e:
        checks["launchd"] = f"FAIL: {e}"

    for component, status in checks.items():
        icon = "\u2705" if status.startswith("OK") else "\u274c"
        print(f"{icon} {component:<12} {status}")

    return all(s.startswith("OK") for s in checks.values())


def main():
    parser = argparse.ArgumentParser(description="Query CocoIndex semantic code search")
    parser.add_argument("query", nargs="?", help="Search query")
    parser.add_argument("--top-k", "-k", type=int, default=10, help="Number of results")
    parser.add_argument("--mode", choices=["semantic", "text"], default="semantic")
    parser.add_argument("--crate", "-c", help="Limit to a specific crate")
    parser.add_argument("--stats", action="store_true", help="Show index stats")
    parser.add_argument("--health", action="store_true", help="Check system health")
    parser.add_argument("--json", action="store_true", help="Output as JSON")
    args = parser.parse_args()

    if args.health:
        sys.exit(0 if health_check() else 1)

    if args.stats:
        show_stats()
        return

    if not args.query:
        parser.error("query is required (or use --stats/--health)")

    search_fn = semantic_search if args.mode == "semantic" else text_search
    try:
        results = search_fn(args.query, args.top_k, args.crate)
    except (urllib.error.URLError, TimeoutError, OSError, ConnectionError) as e:
        print(f"Embedding server unreachable ({e}). Falling back to text search.", file=sys.stderr)
        results = text_search(args.query, args.top_k, args.crate)

    if args.json:
        out = [
            {"filename": r[0], "language": r[1], "content": r[2], "score": float(r[3])}
            for r in results
        ]
        print(json.dumps(out, indent=2))
    else:
        for filename, language, content, score in results:
            print(f"[{score:.3f}] {filename} ({language})")
            preview = content.replace("\n", " ")[:120]
            print(f"  {preview}")
            print()


if __name__ == "__main__":
    main()
