# CocoIndex-PG Troubleshooting

## Table of Contents

1. [Embedding Server Unreachable](#embedding-server-unreachable)
2. [Launchd Service Issues](#launchd-service-issues)
3. [Stale or Missing Data](#stale-or-missing-data)
4. [Multiple Processes](#multiple-processes)
5. [Schema Problems](#schema-problems)
6. [Performance Issues](#performance-issues)

## Embedding Server Unreachable

**Symptom**: Semantic search fails, indexer stalls at N/M files, `Connection refused` errors.

**Diagnose**:
```bash
curl -s --max-time 5 http://10.0.0.21:8081/health
# Expected: {"status":"ok"}
```

**Fixes**:
1. Check SLURM job status:
   ```bash
   ssh root@10.0.0.5 'squeue --format="%.8i %.20j %.2t %.8M %.5D %R"'
   ```
2. If no embedding job, check the ai-inference-daemon on vasp-01:
   ```bash
   ssh root@10.0.0.21 'systemctl status ai-inference-daemon'
   ```
3. Verify model loaded:
   ```bash
   curl -s http://10.0.0.21:8081/v1/models | python3 -m json.tool
   # Should show nomic-embed-code.Q8_0.gguf
   ```

**Workaround**: Fall back to text search (ILIKE) which requires no embedding server.

## Launchd Service Issues

### Service Not Running

```bash
# Check status
launchctl list com.briansquires.cocoindex-rustdaq

# If "Could not find service":
launchctl load ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist

# Force restart:
launchctl unload ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
launchctl load ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
```

### Service Crash-Looping

Check error log:
```bash
tail -50 ~/Library/Logs/cocoindex-rustdaq.error.log
```

Common causes:
- PostgreSQL not running → start Postgres.app or `brew services start postgresql`
- Embedding server down → indexer retries but eventually errors out
- Python venv broken → recreate `~/beefcake2/.venv`

### Plist Disabled

If the plist was renamed to `.plist.disabled`:
```bash
mv ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist.disabled \
   ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
launchctl load ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
```

## Stale or Missing Data

**Symptom**: Search returns old results, new files not appearing.

**Diagnose**:
```sql
-- Check when a specific file was last indexed
psql -d cocoindex -c "
  SELECT filename, count(*) as chunks
  FROM code_chunks
  WHERE filename LIKE '%capabilities%'
  GROUP BY filename;
"
```

**Fixes**:
1. Verify live updater is running:
   ```bash
   launchctl list com.briansquires.cocoindex-rustdaq | grep PID
   ```
2. Check recent log for refresh activity:
   ```bash
   tail -5 ~/Library/Logs/cocoindex-rustdaq.log
   # Should show: "N/M source rows: X updated, Y no change"
   ```
3. Force full reprocess:
   ```bash
   COCOINDEX_DATABASE_URL="postgresql://briansquires@localhost/cocoindex" \
     ~/beefcake2/.venv/bin/python ~/beefcake2/index_flow_v2.py
   ```

## Multiple Processes

**Symptom**: High CPU, database locks, inconsistent results.

**Diagnose**:
```bash
ps aux | grep index_flow_v2 | grep -v grep
```

**Fix**: Kill all stale processes, keep only the launchd-managed one:
```bash
# Kill all
pkill -f index_flow_v2
# Restart clean via launchd
launchctl unload ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
launchctl load ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
```

## Schema Problems

**Symptom**: `column "embedding" does not exist` or dimension mismatch errors.

**Diagnose**:
```bash
psql -d cocoindex -c "\d code_chunks"
# Should show: embedding vector(3584)
```

**Fix** (nuclear — full reindex):
```bash
# Stop the service first
launchctl unload ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
# Drop and recreate
psql -d cocoindex -c "DROP TABLE IF EXISTS code_chunks CASCADE;"
# Restart — CocoIndex will recreate the table
launchctl load ~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist
```

Note: A full reindex takes ~15-30 minutes depending on embedding server speed. After reindexing, recreate the halfvec HNSW index (CocoIndex cannot create it automatically because pgvector rejects HNSW on raw 3584-dim vectors):
```sql
CREATE INDEX CONCURRENTLY code_chunks_embedding_hnsw ON code_chunks
  USING hnsw ((embedding::halfvec(3584)) halfvec_cosine_ops)
  WITH (ef_construction = 128);
```

## Performance Issues

### Slow Queries

With the halfvec HNSW index, vector searches should complete in ~60ms. If queries regress to ~1s:

**Diagnose**:
```bash
psql -d cocoindex -c "SELECT indexname, pg_size_pretty(pg_relation_size(indexname::regclass)) FROM pg_indexes WHERE tablename = 'code_chunks' AND indexdef LIKE '%hnsw%';"
# Expected: code_chunks_embedding_hnsw, ~116 MB
```

**Fixes**:
- **Index missing or invalid**: Recreate it:
  ```sql
  DROP INDEX IF EXISTS code_chunks_embedding_hnsw;
  CREATE INDEX CONCURRENTLY code_chunks_embedding_hnsw ON code_chunks
    USING hnsw ((embedding::halfvec(3584)) halfvec_cosine_ops)
    WITH (ef_construction = 128);
  ```
- **Query not using halfvec cast**: Ensure all queries cast to `halfvec(3584)` — without the cast, pgvector falls back to sequential scan on the raw `vector(3584)` column.
- **Tune search accuracy**: `SET hnsw.ef_search = 100;` (default 40) for higher recall at slight latency cost.

### Large Database

```bash
# Check database size
psql -d cocoindex -c "SELECT pg_size_pretty(pg_database_size('cocoindex'));"
# Vacuum if fragmented
psql -d cocoindex -c "VACUUM ANALYZE code_chunks;"
```
