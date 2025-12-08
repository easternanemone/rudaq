# AGENTS.md

Instructions for AI coding agents (Claude Code, Cursor, Codex CLI, etc.) working with this repository.

## Morph Integration

This repo has Morph SDK integration for AI-powered code editing and semantic search.

### MCP Server (Agent Tools)

**Setup** (one-time per tool):
```bash
# Claude Code
claude mcp add filesystem-with-morph \
  -e MORPH_API_KEY=$MORPH_API_KEY \
  -e ENABLED_TOOLS=all \
  -- npx @morphllm/morphmcp

# Codex CLI
codex mcp add filesystem-with-morph \
  -e MORPH_API_KEY=$MORPH_API_KEY \
  -e ENABLED_TOOLS=all \
  -- npx @morphllm/morphmcp
```

**Available MCP Tools**:

| Tool | Description | Use When |
|------|-------------|----------|
| `mcp__filesystem-with-morph__edit_file` | Fast code editing (10,500 tokens/s, 98% accuracy) | Any file modification |
| `mcp__filesystem-with-morph__warp_grep` | AI-powered code search with grep + file analysis | Finding code by description |
| `mcp__filesystem-with-morph__codebase_search` | Semantic search via embeddings | Only works with HTTPS git remotes |

**Prefer Morph edit_file** over default Edit tool for all code changes.

### SDK Power-User Utilities

Located in `scripts/morph/`. Install once:
```bash
cd scripts/morph && npm install
```

#### Embedding API (`morph-embedding-v3`)

Generate 1024-dimensional code embeddings for similarity search:

```bash
# Embed a file (auto-chunks large files)
npm run embed -- --file ../../src/hardware/capabilities.rs
# Output: Generated 3 embedding(s) in 679ms

# Embed inline code
npm run embed -- --text "async fn set_exposure(&self, seconds: f64) -> Result<()>"

# Save embeddings to JSON
npm run embed -- --file <path> --output /tmp/embeddings.json
```

**Use case**: Find semantically similar functions by computing cosine similarity between embeddings.

#### Rerank API (`morph-rerank-v3`)

Reorder search results by semantic relevance:

```bash
npm run rerank -- --query "set camera exposure time in milliseconds" \
  --docs "async fn set_exposure(&self, seconds: f64)" \
  --docs "async fn get_position(&self) -> Result<f64>" \
  --docs "pub async fn set_exposure_ms(&self, exposure_ms: f64)" \
  --docs "fn frame_count(&self) -> u64"

# Output (ranked by relevance):
# [2] Score: 57.8%  set_exposure_ms  <- Most relevant
# [0] Score: 52.5%  set_exposure
# [3] Score: 25.6%  frame_count
# [1] Score: 19.0%  get_position     <- Least relevant
```

**Use case**: After grep/search returns many results, rerank by semantic relevance to find the best match.

#### Code Similarity Workflow

Find similar functions using embeddings:

```bash
# 1. Embed functions to compare
npm run embed -- --text "pub async fn set_exposure(&self, seconds: f64)" --output /tmp/e1.json
npm run embed -- --text "pub async fn set_exposure_ms(&self, ms: f64)" --output /tmp/e2.json
npm run embed -- --text "pub async fn set_binning(&self, x: u16, y: u16)" --output /tmp/e3.json

# 2. Compute cosine similarity
node -e '
const fs = require("fs");
const e1 = JSON.parse(fs.readFileSync("/tmp/e1.json")).chunks[0].embedding;
const e2 = JSON.parse(fs.readFileSync("/tmp/e2.json")).chunks[0].embedding;
const e3 = JSON.parse(fs.readFileSync("/tmp/e3.json")).chunks[0].embedding;

function cosineSim(a, b) {
  const dot = a.reduce((sum, x, i) => sum + x * b[i], 0);
  const mag1 = Math.sqrt(a.reduce((sum, x) => sum + x * x, 0));
  const mag2 = Math.sqrt(b.reduce((sum, x) => sum + x * x, 0));
  return dot / (mag1 * mag2);
}

console.log("set_exposure vs set_exposure_ms:", (cosineSim(e1, e2) * 100).toFixed(1) + "%");  // ~83%
console.log("set_exposure vs set_binning:", (cosineSim(e1, e3) * 100).toFixed(1) + "%");      // ~27%
'
```

**Result**: Semantically similar functions score ~83%, different functions score ~27%.

### Known Limitations

1. **SSH Remotes**: `codebase_search` and cloud repo features require HTTPS git remotes. This repo uses SSH (`git@github.com:...`), so use `warp_grep` or Embed+Rerank instead.

2. **Environment Variables**: GUI apps (Claude Code) don't inherit shell env vars. Use `launchctl setenv MORPH_API_KEY <key>` on macOS.

## Project-Specific Instructions

### Architecture

This project is transitioning to V5 "headless-first" architecture. Key patterns:

- **Capability Traits**: `Movable`, `Readable`, `FrameProducer`, `ExposureControl`, `Triggerable`
- **Async Drivers**: All hardware drivers use `tokio::sync::Mutex` for async-safe state
- **Scripting**: Rhai-based experiment control (`examples/*.rhai`)
- **Data Plane**: Ring buffer + HDF5 writer for high-throughput acquisition

### Feature Flags

Enable features as needed:
```bash
# Hardware drivers
cargo build --features "instrument_thorlabs,instrument_newport"

# All hardware
cargo build --features "all_hardware"

# Storage backends
cargo build --features "storage_hdf5,storage_arrow"

# gRPC server
cargo build --features "networking"
```

### Issue Tracking

Use `bd` (beads) for persistent issue tracking:
```bash
bd ready          # Check available work
bd list           # List all issues
bd create "Task"  # Create new issue
bd close <id>     # Close completed issue
```

### File Organization

- `/src` - Source code
- `/tests` - Test files
- `/docs` - Documentation
- `/config` - Configuration files
- `/scripts` - Utility scripts (including `scripts/morph/`)
- `/examples` - Example Rhai scripts

## Links

- [Full Morph Documentation](docs/MORPH_INTEGRATION.md)
- [Architecture Overview](docs/architecture/ARCHITECTURAL_FLAW_ANALYSIS.md)
- [Morph Dashboard](https://morphllm.com/dashboard)
