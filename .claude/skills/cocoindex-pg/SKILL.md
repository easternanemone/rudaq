---
name: cocoindex-pg
description: "Search and manage the rust-daq PostgreSQL+pgvector semantic code index powered by CocoIndex and nomic-embed-code. Use when: (1) searching code by meaning rather than exact text, (2) finding similar implementations across the codebase, (3) exploring unfamiliar code areas by concept, (4) checking index health or stats, (5) managing the launchd indexer service, (6) troubleshooting embedding or database issues. Triggers on: semantic search, code search, find similar code, index health, cocoindex status, embedding server."
---

# CocoIndex PostgreSQL Semantic Code Search

## Decision: Search Method

| Situation | Method |
|-----------|--------|
| Know the concept, not the keywords | **Semantic search** (Pattern 1) |
| Know exact symbol/string | Use `Grep` or `Glob` tools instead |
| Embedding server might be down | **Text search** (Pattern 3) or check health first |
| Need stats or health check | **Utility queries** (Pattern 4–5) |

## Quick Reference

```
Database:    postgresql://briansquires@localhost/cocoindex
Table:       code_chunks (filename, chunk_location, language, chunk_content, embedding)
Embedding:   nomic-embed-code.Q8_0, 3584 dims, cosine similarity (halfvec HNSW index)
Endpoint:    http://10.0.0.21:8081/v1 (vasp-01 GPU)
Service:     com.briansquires.cocoindex-rustdaq (launchd, 30s refresh)
Flow:        ~/beefcake2/index_flow_v2.py --live
Logs:        ~/Library/Logs/cocoindex-rustdaq.log (.error.log for stderr)
```

## Search Patterns

### Pattern 1: Semantic Search (via bundled script)

Run `scripts/query_cocoindex.py` from this skill directory:

```bash
~/beefcake2/.venv/bin/python <skill-dir>/scripts/query_cocoindex.py "error recovery retry logic" -k 10
```

Crate-scoped:
```bash
~/beefcake2/.venv/bin/python <skill-dir>/scripts/query_cocoindex.py "frame allocation" --crate common -k 5
```

JSON output (for programmatic use):
```bash
~/beefcake2/.venv/bin/python <skill-dir>/scripts/query_cocoindex.py "capability traits" --json
```

The script auto-falls back to text search if the embedding server is unreachable.

### Pattern 2: Semantic Search (raw SQL)

When you need more control or the script isn't available:

```bash
# Step 1: Get embedding vector
EMBEDDING=$(curl -s http://10.0.0.21:8081/v1/embeddings \
  -H "Content-Type: application/json" \
  -d '{"model":"nomic-embed-code.Q8_0","input":"your query here"}' \
  | python3 -c "import sys,json; print('['+','.join(str(x) for x in json.load(sys.stdin)['data'][0]['embedding'])+']')")

# Step 2: Query pgvector (cast to halfvec to use HNSW index)
psql -d cocoindex -c "
  SELECT filename,
         1 - (embedding::halfvec(3584) <=> '${EMBEDDING}'::halfvec) as score,
         left(chunk_content, 120) as preview
  FROM code_chunks
  ORDER BY embedding::halfvec(3584) <=> '${EMBEDDING}'::halfvec
  LIMIT 10;
"
```

### Pattern 3: Text Search (no embedding server needed)

```sql
psql -d cocoindex -c "
  SELECT filename, left(chunk_content, 120)
  FROM code_chunks
  WHERE chunk_content ILIKE '%DriverFactory%'
  LIMIT 10;
"
```

Crate-scoped text search:
```sql
psql -d cocoindex -c "
  SELECT filename, chunk_content
  FROM code_chunks
  WHERE filename LIKE 'crates/common/%'
    AND chunk_content ILIKE '%Parameter%'
  ORDER BY filename, chunk_location
  LIMIT 20;
"
```

### Pattern 4: File Inventory

```sql
-- Files by language
psql -d cocoindex -c "
  SELECT language, count(DISTINCT filename) as files, count(*) as chunks
  FROM code_chunks GROUP BY language ORDER BY chunks DESC;
"

-- All files in a crate
psql -d cocoindex -c "
  SELECT filename, count(*) as chunks
  FROM code_chunks WHERE filename LIKE 'crates/experiment/%'
  GROUP BY filename ORDER BY filename;
"
```

### Pattern 5: Context Expansion

After finding a relevant chunk, fetch surrounding chunks from the same file:

```sql
psql -d cocoindex -c "
  SELECT chunk_location, left(chunk_content, 200)
  FROM code_chunks
  WHERE filename = 'crates/common/src/capabilities.rs'
  ORDER BY chunk_location;
"
```

**Always follow up** with the `Read` tool on the actual file for full context and line numbers before editing.

## Service Management

```bash
# Check status (look for PID)
launchctl list com.briansquires.cocoindex-rustdaq

# Recent activity
tail -5 ~/Library/Logs/cocoindex-rustdaq.log

# Restart
launchctl unload ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
launchctl load ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist

# Force full reindex (one-shot, run manually)
COCOINDEX_DATABASE_URL="postgresql://briansquires@localhost/cocoindex" \
  ~/beefcake2/.venv/bin/python ~/beefcake2/index_flow_v2.py
```

## Health Check

```bash
~/beefcake2/.venv/bin/python <skill-dir>/scripts/query_cocoindex.py --health
```

Or manually:
```bash
# PostgreSQL
psql -d cocoindex -c "SELECT count(*) FROM code_chunks;"

# Embedding server
curl -s --max-time 5 http://10.0.0.21:8081/health

# Launchd
launchctl list com.briansquires.cocoindex-rustdaq | grep PID
```

## Embedding Server

The `nomic-embed-code.Q8_0` model runs on the GPU cluster (vasp-01, 10.0.0.21:8081):

```bash
# Check if running
curl -s --max-time 5 http://10.0.0.21:8081/health

# Check SLURM job status
ssh root@10.0.0.5 'squeue --format="%.8i %.20j %.2t %.8M %.5D %R"'

# Check model info
curl -s http://10.0.0.21:8081/v1/models | python3 -m json.tool
```

If the embedding server is down, text search (Pattern 3) still works — it doesn't need embeddings.

## Bundled Resources

- **scripts/query_cocoindex.py**: Standalone semantic/text search CLI with health check and stats
- **references/architecture.md**: System architecture, data flow, schema details, network topology — read when debugging or understanding the pipeline
- **references/troubleshooting.md**: Common failure modes and fixes — read when something breaks

## Tips

- Chunk size is 1000 chars with 100 overlap — most Rust functions fit in one chunk
- `chunk_location` is a byte range (`int8range`) — use it to reconstruct reading order
- **HNSW index via halfvec**: 3584 dims exceeds pgvector's 2000-dim `vector` limit, but casting to `halfvec(3584)` (16-bit) enables HNSW indexing with `halfvec_cosine_ops`. Queries run in ~60ms.
- Combine with grep: use cocoindex to discover relevant files, then grep for precise locations
- The `<skill-dir>` placeholder refers to this skill's directory on disk
