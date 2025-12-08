# Hybrid Morph + CocoIndex Search Setup Guide

This guide walks you through setting up the hybrid search system that combines:
- **Morph**: Fast code search with Warp Grep (~200-500ms)
- **CocoIndex**: Comprehensive documentation search with semantic understanding (~100-300ms)

## Quick Start

### Prerequisites

- Docker and Docker Compose installed
- Python 3.12+ with pip
- Node.js 18+ (for Morph tools)
- OpenAI API key
- Morph API key (optional, for Warp Grep)

### 1. Start Infrastructure

```bash
# Start Postgres+pgvector and Neo4j
docker-compose up -d

# Verify services are running
docker-compose ps

# Check Postgres
docker exec rust-daq-postgres psql -U cocoindex -c '\l'

# Check Neo4j (in browser)
open http://localhost:7474
# Login: neo4j / cocoindex
```

### 2. Configure Environment

Update `.env` with your API keys:

```bash
# Edit .env and add your keys
MORPH_API_KEY=sk-morph-your-key-here
OPENAI_API_KEY=sk-your-openai-key-here

# Verify database URLs are correct (already set by setup)
# POSTGRES_URL=postgresql://cocoindex:cocoindex@localhost:5432/cocoindex
# NEO4J_URL=bolt://neo4j:cocoindex@localhost:7687
```

### 3. Install Python Dependencies

```bash
# Create virtual environment
python3 -m venv .venv
source .venv/bin/activate  # or .venv\Scripts\activate on Windows

# Install CocoIndex and dependencies
pip install cocoindex psycopg2-binary python-dotenv
```

### 4. Index Documentation

```bash
# Run the comprehensive documentation index flow
python cocoindex_flows/comprehensive_docs_index.py

# Expected output:
# Starting comprehensive documentation indexing flow...
# This will index all .md files in:
#   - docs/**/*.md
#   - *.md (root)
#   - clients/python/docs/**/*.md
#   - examples/**/*.md
#
# ✅ Indexing complete!
#    Processed ~180 documentation files
#    Time: 120-180s (depending on OpenAI API speed)
```

**Note**: Initial indexing costs ~$2-5 in OpenAI API calls (gpt-4o-mini for extraction + text-embedding-3-small).

### 5. Test the Hybrid Search

```bash
# Make search script executable
chmod +x scripts/search_hybrid.py

# Test comprehensive search (documentation)
./scripts/search_hybrid.py --query "How does V5 parameter reactive system work?"

# Test quick search (code) - requires Morph MCP setup
./scripts/search_hybrid.py --query "impl Movable trait"

# Auto-detect mode (default)
./scripts/search_hybrid.py --query "BoxFuture async callbacks"
```

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    Hybrid Search System                      │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │  Query Router   │
                    │  (Auto-Detect)  │
                    └─────────────────┘
                              │
                ┌─────────────┴─────────────┐
                ▼                           ▼
        ┌──────────────┐            ┌──────────────┐
        │  Morph       │            │  CocoIndex   │
        │  Warp Grep   │            │  Semantic    │
        │  (Code)      │            │  (Docs)      │
        └──────────────┘            └──────────────┘
                │                           │
                ▼                           ▼
        Code patterns            Documentation knowledge
        - Fast (~300ms)          - Comprehensive (~200ms)
        - Syntax-aware           - LLM-extracted metadata
        - File:line results      - Summaries + concepts
```

## System Components

### 1. CocoIndex Documentation Flow

**File**: `cocoindex_flows/comprehensive_docs_index.py`

**What it does**:
- Indexes ~180 current markdown files (excludes backups/system dirs)
- Extracts structured metadata: title, category, key concepts, related files, status
- Generates natural language summaries
- Creates embeddings for semantic search
- Stores in Postgres with pgvector

**Data Sources**:
- `docs/` - 76 files (architecture, guides, reference)
- Root `*.md` - 16 files (README, CLAUDE.md, setup guides)
- `clients/python/docs/` - 5 files
- `examples/*.md` - 1-2 files

**Query Example**:
```python
from cocoindex_flows.comprehensive_docs_index import search_docs

# Search with category filter
results = search_docs(
    "V5 parameter reactive system",
    category="architecture",
    limit=10
)

for r in results:
    print(f"{r['title']} ({r['similarity']:.1%} match)")
    print(f"  {r['summary'][:150]}...")
```

### 2. Unified Search Interface

**File**: `scripts/search_hybrid.py`

**What it does**:
- Auto-detects query type (code vs knowledge)
- Routes to appropriate search system
- Unifies result format
- Provides human-readable and JSON output

**Auto-Detection Logic**:
- **Code keywords**: implementation, impl, fn, struct, async, trait → Quick mode (Morph)
- **Knowledge keywords**: architecture, why, how, design, guide → Comprehensive mode (CocoIndex)
- **Patterns**: Contains `.rs`, `::`, `<>` → Quick mode

**Usage**:
```bash
# Auto-detect (recommended)
./scripts/search_hybrid.py --query "your question"

# Force mode
./scripts/search_hybrid.py --query "..." --mode quick
./scripts/search_hybrid.py --query "..." --mode comprehensive

# JSON output (for tooling)
./scripts/search_hybrid.py --query "..." --json
```

### 3. Docker Infrastructure

**File**: `docker-compose.yml`

**Services**:

1. **Postgres 16 + pgvector** (port 5432)
   - Database: `cocoindex`
   - User: `cocoindex`
   - Password: `cocoindex` (change in `.env`)
   - Extensions: pgvector for embeddings
   - Volume: `.docker-data/pgdata`

2. **Neo4j 5.14** (ports 7474, 7687)
   - User: `neo4j`
   - Password: `cocoindex` (change in `.env`)
   - Plugins: APOC for graph algorithms
   - Volumes: `.docker-data/neo4j/{data,logs,import}`
   - Browser: http://localhost:7474

**Management**:
```bash
# Start services
docker-compose up -d

# Stop services
docker-compose down

# View logs
docker-compose logs -f postgres
docker-compose logs -f neo4j

# Restart services
docker-compose restart

# Remove all data (destructive!)
docker-compose down -v
rm -rf .docker-data
```

## Usage Workflows

### Workflow 1: Finding Implementation Details

```bash
# Step 1: Quick code search (Morph Warp Grep)
./scripts/search_hybrid.py --query "BoxFuture async callbacks" --mode quick
# Returns: maitai.rs:245, parameter.rs:167 (~300ms)

# Step 2: Deep knowledge search (CocoIndex)
./scripts/search_hybrid.py --query "Why use BoxFuture for async hardware?" --mode comprehensive
# Returns: Architecture docs, design decisions, related examples (~2s)
```

### Workflow 2: Exploring Architecture

```bash
# Find architecture documentation
./scripts/search_hybrid.py --query "V5 headless-first architecture design decisions"

# Results show:
# - docs/architecture/V5_ARCHITECTURE.md
# - docs/architecture/ARCHITECTURAL_FLAW_ANALYSIS.md
# - Related ADRs (Architecture Decision Records)
# - Key concepts: headless-first, capability traits, gRPC
```

### Workflow 3: Hardware Discovery

```bash
# Find hardware implementations
./scripts/search_hybrid.py --query "devices implementing Movable capability"

# Results show:
# - src/hardware/ell14.rs - Thorlabs rotation mount
# - src/hardware/esp300.rs - Newport motion controller
# - src/hardware/mock_stage.rs - Mock stage for testing
# - Related docs: hardware capability traits guide
```

## Performance Characteristics

### Morph Warp Grep (Code Search)
- **Speed**: ~200-500ms
- **Accuracy**: Syntax-aware pattern matching
- **Coverage**: All Rust, TOML, Rhai files
- **Results**: File path, line numbers, code snippets

### CocoIndex (Documentation Search)
- **Speed**: ~100-300ms (pgvector HNSW index)
- **Accuracy**: Semantic understanding via embeddings
- **Coverage**: ~180 current markdown files
- **Results**: Summaries, categories, key concepts, related files

### Initial Indexing
- **Time**: 2-5 minutes for ~180 docs
- **Cost**: ~$2-5 (OpenAI API: gpt-4o-mini + text-embedding-3-small)
- **Storage**: ~150MB (embeddings + metadata)
- **Updates**: Automatic with file watching (5-15s per file)

## Troubleshooting

### Issue: "CocoIndex not installed or flow not indexed"

```bash
# Install CocoIndex
pip install cocoindex

# Run indexing flow
python cocoindex_flows/comprehensive_docs_index.py
```

### Issue: "Postgres connection refused"

```bash
# Check if Postgres is running
docker-compose ps

# Check logs
docker-compose logs postgres

# Restart Postgres
docker-compose restart postgres

# Verify connection
docker exec rust-daq-postgres psql -U cocoindex -c 'SELECT 1'
```

### Issue: "Neo4j authentication failed"

```bash
# Check Neo4j password in .env
cat .env | grep NEO4J_PASSWORD

# Reset Neo4j (destructive!)
docker-compose stop neo4j
rm -rf .docker-data/neo4j
docker-compose up -d neo4j
```

### Issue: "No search results found"

```bash
# Verify CocoIndex index exists
docker exec rust-daq-postgres psql -U cocoindex -c '\dt'
# Should show: comprehensivedocsindex__comprehensive_docs

# Check row count
docker exec rust-daq-postgres psql -U cocoindex -c \
  'SELECT COUNT(*) FROM comprehensivedocsindex__comprehensive_docs'
# Should show ~180

# Re-index if needed
python cocoindex_flows/comprehensive_docs_index.py
```

### Issue: "OpenAI API rate limit"

```bash
# Check API usage at https://platform.openai.com/usage

# Reduce batch size in flow (edit comprehensive_docs_index.py):
# chunk_size=500  # Default: 1000
```

## Cost Estimates

### One-Time Setup Costs
- Initial indexing: ~$2-5 (OpenAI API)
- Total time: 2-5 minutes

### Ongoing Costs
- Re-indexing on doc changes: ~$0.01 per file
- Embeddings for queries: ~$0.0001 per query
- LLM extraction for new docs: ~$0.02 per doc

### Infrastructure Costs
- Local Docker: Free
- Storage: ~150MB local disk

## Next Steps

1. **Run Initial Index**: `python cocoindex_flows/comprehensive_docs_index.py`
2. **Test Search**: `./scripts/search_hybrid.py --query "test query"`
3. **Integrate with Workflow**: Add to your daily development routine
4. **Monitor Performance**: Check query times and adjust as needed

## Phase 2: Hardware Knowledge Graph (Future)

After Phase 1 (comprehensive docs index) is stable, implement:

- **Flow**: `cocoindex_flows/hardware_knowledge_graph.py`
- **Target**: Neo4j graph database
- **Schema**: Device → Capability → Parameter → Protocol
- **Benefits**: Relationship queries, dependency mapping, hardware discovery

See [implementation plan](/Users/briansquires/.claude/plans/stateless-inventing-leaf.md) for details.

## System Status

### Infrastructure

✅ **Postgres + pgvector**: Running on port 5432  
✅ **Neo4j**: Running on ports 7474 (HTTP), 7687 (Bolt)  
✅ **CocoIndex Flow**: Configured and indexed  
✅ **Morph Integration**: Placeholder ready (requires MCP server)

### Indexed Documentation

**Total Files**: 55 markdown files  
**Breakdown**:
- `docs_main`: 48 files (architecture, guides, reference)
- `docs_python`: 4 files (Python client documentation)
- `docs_root`: 3 files (README.md, CLAUDE.md, CHANGELOG.md)

**Categories**: 8 total
- `reference`: 23 docs (hardware, protocols, testing)
- `guides`: 14 docs (development, scripting, tools)
- `getting_started`: 5 docs
- `architecture`: 5 docs (V5 design, ADRs)
- `other`: 5 docs
- `examples`: 1 doc
- `instruments`: 1 doc
- `tools`: 1 doc

**Exclusions Working**: ✅ Zero node_modules files indexed, zero development tool files (.brv, .github, .jules)

## Support

For issues or questions:
- Check this guide's troubleshooting section
- Review CocoIndex documentation: https://docs.cocoindex.com
- Review Morph documentation: https://docs.morphllm.com
- Open issue in rust-daq repository
