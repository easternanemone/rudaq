# CocoIndex-PG Architecture

## Table of Contents

1. [System Overview](#system-overview)
2. [Data Flow](#data-flow)
3. [Components](#components)
4. [Network Topology](#network-topology)
5. [Schema Details](#schema-details)
6. [Configuration](#configuration)

## System Overview

The rust-daq code search system is a distributed pipeline:

```
┌──────────────┐   30s poll    ┌──────────────────┐   HTTP/REST    ┌─────────────────┐
│  rust-daq/   │──────────────▶│  CocoIndex v0.3  │──────────────▶│  nomic-embed-    │
│  source tree │  file watcher │  (Mac local)     │  embed request │  code.Q8_0       │
└──────────────┘               │                  │               │  (cluster GPU)   │
                               │  - chunk files   │◀──────────────│  vasp-01         │
                               │  - detect lang   │  3584-d vector │  10.0.0.21:8081  │
                               │  - call embed    │               └─────────────────┘
                               │  - write to PG   │
                               └────────┬─────────┘
                                        │ SQL INSERT/UPDATE
                                        ▼
                               ┌──────────────────┐
                               │  PostgreSQL 17    │
                               │  + pgvector 0.8   │
                               │  halfvec HNSW idx  │
                               │  (Mac local)      │
                               └──────────────────┘
```

**Key insight**: The Mac does orchestration (file I/O, chunking, SQL writes). The GPU cluster does the expensive neural network inference. This means the Mac CPU will be active during indexing, but the heavy compute is remote.

## Data Flow

1. **File Discovery**: CocoIndex `LocalFile` source watches `/Users/briansquires/code/rust-daq` every 30 seconds
2. **Change Detection**: Compares file hashes against internal state DB (`cocoindex` internal tables)
3. **Language Detection**: `DetectProgrammingLanguage()` classifies by file extension
4. **Chunking**: `SplitRecursively(chunk_size=1000, chunk_overlap=100)` — language-aware splitting
5. **Embedding**: Each chunk sent to `nomic-embed-code.Q8_0` via OpenAI-compatible `/v1/embeddings` endpoint
6. **Storage**: Results written to `code_chunks` table (halfvec HNSW index accelerates cosine similarity queries)

## Components

### CocoIndex Python Framework (v0.3.28)
- Installed in: `~/beefcake2/.venv/`
- Flow definition: `~/beefcake2/index_flow_v2.py`
- Modes: `--live` (continuous), `--server` (query API + optional live), or one-shot
- Internal state: stored in PostgreSQL internal tables (not user-facing)

### PostgreSQL + pgvector
- Database: `cocoindex` (local, no password, unix socket)
- User: `briansquires`
- pgvector version: 0.8.0
- Vector index: HNSW on `halfvec(3584)` with `halfvec_cosine_ops` (ef_construction=128, ~116 MB). Queries use the index (~60ms) instead of sequential scan.
- Table: `code_chunks` (see Schema Details below)

### Embedding Server (nomic-embed-code.Q8_0)
- Model: `nomic-embed-code` 7B parameter, Q8_0 quantization, GGUF format
- Dimensions: 3584
- Runtime: llama-server (llama.cpp)
- Host: vasp-01 (10.0.0.21), port 8081
- Protocol: OpenAI-compatible `/v1/embeddings`
- API key: `sk-dummy` (required by client, ignored by server)
- Managed by: SLURM job or ai-inference-daemon systemd service

### launchd Service
- Label: `com.briansquires.cocoindex-rustdaq`
- Plist: `~/Library/LaunchAgents/com.briansquires.cocoindex-rustdaq.plist`
- Runs: `index_flow_v2.py --live` (continuous 30s refresh)
- Auto-start: on login (`RunAtLoad`)
- Auto-restart: on non-zero exit (`KeepAlive.SuccessfulExit = false`)
- Throttle: 30 seconds between restarts
- Logs: `~/Library/Logs/cocoindex-rustdaq.log` (stdout), `.error.log` (stderr)
- Env: `COCOINDEX_DATABASE_URL`, `COCOINDEX_SOURCE_MAX_INFLIGHT_ROWS=50`

## Network Topology

| Component | Host | IP | Port |
|-----------|------|-----|------|
| PostgreSQL | Mac (localhost) | 127.0.0.1 | 5432 |
| Embedding server | vasp-01 | 10.0.0.21 | 8081 |
| SLURM controller | controller | 10.0.0.5 | — |
| CocoInsight UI | cocoindex.io | — | 49344 (local) |

## Schema Details

```sql
-- Main table
CREATE TABLE code_chunks (
    filename       TEXT        NOT NULL,
    chunk_location INT8RANGE   NOT NULL,
    language       TEXT,
    chunk_content  TEXT,
    embedding      VECTOR(3584),
    PRIMARY KEY (filename, chunk_location)
);

-- HNSW index on halfvec cast (pgvector 0.8 limits full vector HNSW to 2000 dims,
-- but halfvec supports up to ~4000 dims with 16-bit scalar quantization):
-- CREATE INDEX code_chunks_embedding_hnsw ON code_chunks
--   USING hnsw ((embedding::halfvec(3584)) halfvec_cosine_ops)
--   WITH (ef_construction = 128);
-- NOTE: Queries must cast to halfvec to use the index (see search patterns).
```

- `filename`: Relative path from rust-daq root (e.g., `crates/common/src/capabilities.rs`)
- `chunk_location`: PostgreSQL int8range (byte offset within file), e.g., `[0,1000)`
- `language`: One of: rust, markdown, toml, yaml, bash, python, json
- `chunk_content`: Raw text of the chunk (typically ~1000 chars)
- `embedding`: 3584-dimensional float vector (cosine similarity for search)

## Configuration

### Indexed File Patterns

**Included**: `*.rs`, `*.toml`, `*.md`, `*.py`, `*.sh`, `*.yaml`, `*.yml`, `*.json`, `*.cfg`

**Excluded**:
- Build artifacts: `target/`, `*.lock`
- VCS/tooling: `.git/`, `.worktrees/`, `.planning/`, `.claude/`, `.beads/`, `.brv/`, `.jules/`
- Python envs: `.venv/`, `venv/`, `node_modules/`, `site-packages/`, `__pycache__/`
- Generated: `**/generated/**`, `**/*.pb.rs`, `**/CHANGELOG.md`

### Chunking Parameters
- `chunk_size`: 1000 characters (most Rust functions fit in one chunk)
- `chunk_overlap`: 100 characters (prevents splitting mid-statement)
- Language-aware: respects syntax boundaries where possible
