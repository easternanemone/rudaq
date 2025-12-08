# Morph SDK Integration

This document provides comprehensive guidance for using Morph's advanced features in the rust-daq project, including MCP tools, SDK components, and AI-powered code search.

## Table of Contents

1. [Overview](#overview)
2. [Setup](#setup)
3. [MCP Tools (Claude Code)](#mcp-tools-claude-code)
4. [SDK Advanced Features](#sdk-advanced-features)
5. [Performance Characteristics](#performance-characteristics)
6. [Best Practices](#best-practices)

## Overview

Morph provides three integration levels:

1. **MCP Tools** - Claude Code integration for fast editing and intelligent search
2. **SDK Components** - Programmatic access to embeddings, reranking, and git operations
3. **Agent Tools** - Warp Grep and semantic search for AI agents

### Key Capabilities

- **Fast Code Editing**: 98% accuracy at 10,500 tokens/s
- **Semantic Search**: Two-stage retrieval (vector search + GPU reranking) in ~1.2s
- **Warp Grep**: AI-powered code search, 20x faster than stock grepping
- **Git Integration**: Automatic code embedding on push
- **Embeddings**: 1024-dimensional code embeddings via `morph-v4-embedding`
- **Reranking**: Precision scoring with `morph-v4-rerank`

## Setup

### 1. MCP Server Setup (One-Time)

For Claude Code integration:

```bash
claude mcp add filesystem-with-morph \
  -e MORPH_API_KEY=$MORPH_API_KEY \
  -e ENABLED_TOOLS=all \
  -- npx @morphllm/morphmcp
```

**Required Environment Variable:**
- `MORPH_API_KEY` - Get your API key from [Morph Dashboard](https://morph.so/dashboard)

### 2. SDK Setup (Optional, for Advanced Features)

For programmatic access to Morph features:

```bash
cd scripts/morph
npm install
```

This installs the `@morphllm/morphsdk` package and utilities.

## MCP Tools (Claude Code)

The MCP server provides three primary tools for Claude Code:

### 1. `edit_file`

Fast, context-aware code editing without reading entire files.

**Features:**
- 98% accuracy rate
- 10,500 tokens/second throughput
- Intelligent placeholder support (`// ... existing code ...`)
- Minimal context pollution

**Usage in Claude Code:**
```
Use mcp__filesystem-with-morph__edit_file to modify files efficiently
```

**Example:**
```rust
// Before
fn calculate(x: f64) -> f64 {
    x * 2.0
}

// Edit instruction: "Add input validation"
// After (automatically generated)
fn calculate(x: f64) -> Result<f64> {
    if x.is_nan() || x.is_infinite() {
        return Err(anyhow!("Invalid input"));
    }
    Ok(x * 2.0)
}
```

### 2. `warp_grep`

AI-powered intelligent code search with automatic file reading.

**Features:**
- 20x faster than stock grep
- Understands semantic context
- Automatically reads relevant files
- Natural language queries

**Usage:**
```
Use mcp__filesystem-with-morph__warpgrep_codebase_search for intelligent code search
```

**Example Queries:**
- "Find where camera exposure is set"
- "Show async hardware callback implementations"
- "Locate Parameter trait usage in drivers"

**Response Format:**
```json
{
  "success": true,
  "contexts": [
    {
      "filepath": "src/hardware/pvcam.rs",
      "content": "async fn set_exposure(&self, seconds: f64) -> Result<()>",
      "lineStart": 145,
      "lineEnd": 158
    }
  ],
  "summary": "Found 3 implementations of exposure setting in camera drivers"
}
```

### 3. `codebase_search`

Semantic search across the entire codebase using embeddings and reranking.

**Features:**
- Two-stage retrieval (vector search + GPU reranking)
- ~1.2 second response time
- Relevance scoring (0-1 scale)
- Branch/commit-specific search

**Requirements:**
- Git remote must be HTTPS (not SSH)
- Code must be pushed to Morph via `git push`

**Usage:**
```
Use mcp__filesystem-with-morph__codebase_search for semantic code search
```

**Parameters:**
- `query` (string, required): Natural language question
- `repoId` (string, required): Repository identifier
- `branch` (optional): Branch name
- `commitHash` (optional): Specific commit (overrides branch)
- `target_directories` (array, optional): Filter to specific paths
- `limit` (number, default 10): Maximum results

**Example:**
```javascript
{
  query: "How is JWT validation implemented?",
  repoId: "rust-daq",
  branch: "main",
  target_directories: ["src/hardware"],
  limit: 10
}
```

**Response Structure:**
```json
{
  "results": [
    {
      "filepath": "src/hardware/capabilities.rs",
      "content": "pub trait ExposureControl: Send + Sync { ... }",
      "rerankScore": 0.89,
      "language": "rust",
      "startLine": 67,
      "endLine": 85
    }
  ]
}
```

## SDK Advanced Features

The Morph SDK (`@morphllm/morphsdk`) provides programmatic access to advanced features.

### Installation

```bash
npm install @morphllm/morphsdk
```

### 1. Semantic Search API

Programmatically search code using natural language queries.

**Example (TypeScript/JavaScript):**
```typescript
import { MorphClient } from '@morphllm/morphsdk';

const morph = new MorphClient({
  apiKey: process.env.MORPH_API_KEY
});

// Search for specific code patterns
const results = await morph.repos.search({
  query: "async hardware callback implementations",
  repoId: "rust-daq",
  branch: "main",
  target_directories: ["src/hardware"],
  limit: 5
});

// Results include relevance scores and exact locations
results.forEach(result => {
  console.log(`${result.filepath} (score: ${result.rerankScore})`);
  console.log(result.content);
});
```

**Performance:**
- Stage 1 (Vector Search): ~240ms → Top 50 candidates via HNSW index
- Stage 2 (GPU Reranking): ~630ms → Precision scoring with `morph-v4-rerank`
- Total: ~870ms + network latency

**Best Practices:**
- Use specific queries: "Where is JWT validation implemented?"
- Avoid single-word queries like "auth" or "database"
- Leverage `target_directories` to narrow scope
- Use `rerankScore` > 0.7 for high-confidence matches

### 2. Git Operations with Auto-Embedding

Morph extends standard git operations with automatic code embedding on push.

**Available Operations:**
- `init()` - Create repository
- `clone()` - Clone repository
- `add()` - Stage files
- `commit()` - Commit changes
- `push()` - Push and trigger embedding (3-8 seconds)
- `pull()` - Fetch changes
- `status()` - File status
- `log()` - Commit history
- `checkout()` - Switch branches
- `branch()` - Create branch
- `listBranches()` - List branches
- `currentBranch()` - Get current branch
- `resolveRef()` - Resolve commit hash

**Example Workflow:**
```typescript
const morph = new MorphClient({ apiKey: process.env.MORPH_API_KEY });

// Initialize repository
await morph.git.init({
  repoId: 'rust-daq',
  dir: '/Users/briansquires/code/rust-daq'
});

// Standard git workflow
await morph.git.add({
  dir: '/Users/briansquires/code/rust-daq',
  filepath: '.'
});

await morph.git.commit({
  dir: '/Users/briansquires/code/rust-daq',
  message: 'feat: Add async hardware callbacks'
});

// Push triggers automatic embedding for semantic search
await morph.git.push({
  dir: '/Users/briansquires/code/rust-daq',
  branch: 'main'
});

// Wait for embeddings (useful in CI/CD)
await morph.git.waitForEmbeddings({
  repoId: 'rust-daq',
  timeout: 120000,
  onProgress: (progress) => {
    console.log(
      `Embedding: ${progress.filesProcessed}/${progress.totalFiles} files`
    );
  }
});
```

**Key Benefits:**
- Zero infrastructure (no vector DB setup)
- Content-addressable caching (shared embeddings across commits)
- Git-native workflow (no new tools)
- Progress monitoring for CI/CD integration

### 3. Code Embeddings

Generate 1024-dimensional embeddings for code snippets or entire files.

**Use Cases:**
- Similarity search
- Code clustering
- Duplicate detection
- Documentation generation

**Example (using scripts/morph utilities):**
```bash
cd scripts/morph

# Generate embedding for a file
npm run embed -- --file ../../src/hardware/capabilities.rs

# Generate embedding for code snippet
npm run embed -- --text "async fn set_exposure(&self, seconds: f64)"

# Save embeddings to file
npm run embed -- --file ../../src/hardware/pvcam.rs --output embeddings.json
```

**Programmatic Usage:**
```typescript
const morph = new MorphClient({ apiKey: process.env.MORPH_API_KEY });

// Embed code snippet
const embedding = await morph.embed.code({
  text: `
    async fn move_abs(&self, position: f64) -> Result<()> {
        self.position.set(position).await
    }
  `
});

console.log(embedding.vector); // [0.123, -0.456, 0.789, ...]
console.log(embedding.vector.length); // 1024
```

**Embedding Model:**
- **Model**: `morph-v4-embedding`
- **Dimensions**: 1024
- **Type**: Optimized for code understanding
- **Latency**: ~50-100ms per snippet

### 4. Code Reranking

Score and rerank code snippets by semantic relevance to a query.

**Use Cases:**
- Refine search results
- Rank documentation sections
- Prioritize code review candidates

**Example (using scripts/morph utilities):**
```bash
cd scripts/morph

# Rerank multiple code snippets
npm run rerank -- \
  --query "set camera exposure" \
  --docs "fn set_exposure(&self, s: f64) -> Result<()>" \
  --docs "fn get_position(&self) -> f64" \
  --docs "async fn capture_frame(&self) -> Result<Frame>"
```

**Programmatic Usage:**
```typescript
const morph = new MorphClient({ apiKey: process.env.MORPH_API_KEY });

// Rerank code snippets
const scores = await morph.rerank({
  query: "async hardware callback implementation",
  documents: [
    { id: "1", text: "fn callback() -> Result<()> { ... }" },
    { id: "2", text: "async fn callback() -> BoxFuture<'static, Result<()>> { ... }" },
    { id: "3", text: "fn sync_operation() -> i32 { ... }" }
  ]
});

// Results sorted by relevance score
scores.forEach(score => {
  console.log(`ID: ${score.id}, Score: ${score.score}`);
});
```

**Output:**
```
ID: 2, Score: 0.92  // Exact match: async + BoxFuture
ID: 1, Score: 0.45  // Partial match: callback but not async
ID: 3, Score: 0.12  // Low relevance: unrelated
```

**Reranking Model:**
- **Model**: `morph-v4-rerank` (GPU-accelerated cross-attention)
- **Latency**: ~630ms for 50 candidates
- **Score Range**: 0.0 (irrelevant) to 1.0 (perfect match)

### 5. Warp Grep as Agent Tool

Integrate Warp Grep into custom AI agents (Anthropic, OpenAI, Vercel AI SDK).

**Features:**
- Custom tool naming and descriptions
- Flexible backend providers (local, E2B, Modal, Daytona)
- Standardized response format

**Example (Anthropic SDK):**
```typescript
import Anthropic from '@anthropic-ai/sdk';
import { MorphClient } from '@morphllm/morphsdk';

const morph = new MorphClient({ apiKey: process.env.MORPH_API_KEY });
const anthropic = new Anthropic({ apiKey: process.env.ANTHROPIC_API_KEY });

// Create custom tool
const warpGrepTool = {
  name: "search_code",
  description: "Search rust-daq codebase for code patterns",
  input_schema: {
    type: "object",
    properties: {
      query: { type: "string", description: "Search query" }
    },
    required: ["query"]
  }
};

// Agent loop
const messages = [
  { role: "user", content: "Find async hardware callback implementations" }
];

const response = await anthropic.messages.create({
  model: "claude-sonnet-4",
  max_tokens: 4096,
  tools: [warpGrepTool],
  messages
});

// Handle tool use
if (response.stop_reason === "tool_use") {
  const toolUse = response.content.find(c => c.type === "tool_use");

  // Execute Warp Grep
  const result = await morph.warpGrep.execute({
    query: toolUse.input.query,
    repoRoot: '/Users/briansquires/code/rust-daq'
  });

  console.log(result.summary);
  console.log(result.contexts);
}
```

**Custom Provider:**
```typescript
import { WarpGrepProvider } from '@morphllm/morphsdk';

class RemoteProvider implements WarpGrepProvider {
  async grep(pattern: string): Promise<string[]> {
    // Execute grep on remote machine
    return executeRemoteCommand(`grep -r "${pattern}" .`);
  }

  async read(filepath: string): Promise<string> {
    // Read file from remote machine
    return fetchRemoteFile(filepath);
  }

  async analyse(query: string, contexts: string[]): Promise<string> {
    // Use LLM to analyze results
    return analyzeWithLLM(query, contexts);
  }
}

// Use custom provider
const result = await morph.warpGrep.execute({
  query: "Find Parameter trait usage",
  provider: new RemoteProvider()
});
```

## Performance Characteristics

### Semantic Search Performance

**Two-Stage Retrieval Pipeline:**
```
User Query
    ↓
[Stage 1: Vector Search]  ~240ms
    ↓ HNSW index lookup
Top 50 Candidates
    ↓
[Stage 2: GPU Reranking]  ~630ms
    ↓ Cross-attention scoring
Top 10 Results (sorted by relevance)
    ↓
~870ms total + network latency
```

### Warp Grep Performance

- **Speed**: 20x faster than stock grep
- **Latency**: ~200-500ms for typical queries
- **Context**: Automatically reads and includes relevant files

### Embedding Performance

- **Single snippet**: ~50-100ms
- **Batch processing**: ~5-10 snippets/second
- **Dimensions**: 1024 (high-quality semantic representation)

### Git Operations

- **Push**: 3-8 seconds to trigger embedding
- **Embedding completion**: 30-120 seconds (depends on repo size)
- **Caching**: Content-addressable (shared across commits)

## Best Practices

### 1. Semantic Search Queries

**Good Queries:**
- "Where is JWT validation implemented?"
- "Show database error handling patterns"
- "Find async hardware callback implementations"

**Bad Queries:**
- "auth" (too vague)
- "database" (single word, no context)
- "code" (overly broad)

### 2. Warp Grep Usage

Use Warp Grep for:
- Exploring unfamiliar codebases
- Finding specific patterns quickly
- Understanding code structure

Use Semantic Search for:
- Finding semantically similar code
- Discovering best practices
- Locating edge cases

### 3. Git Integration

**Workflow:**
1. Make code changes locally
2. Commit with descriptive messages
3. Push to trigger embedding
4. Wait for embedding completion (optional)
5. Use semantic search to find related code

**CI/CD Integration:**
```typescript
// In CI pipeline
await morph.git.push({ dir: '.', branch: 'main' });
await morph.git.waitForEmbeddings({
  repoId: 'rust-daq',
  timeout: 300000
});
// Now semantic search is up-to-date
```

### 4. Embedding and Reranking

**When to Use:**
- **Embeddings**: Building custom search tools, clustering, similarity
- **Reranking**: Improving search quality, prioritizing results

**Optimization:**
- Batch embed operations when possible
- Cache embeddings for frequently-queried code
- Use `target_directories` to reduce search space

### 5. Error Handling

```typescript
try {
  const results = await morph.repos.search({
    query: "hardware callbacks",
    repoId: "rust-daq"
  });
} catch (error) {
  if (error.message.includes("Repository not found")) {
    console.error("Run 'morph.git.push()' to initialize repository");
  } else if (error.message.includes("Embeddings not ready")) {
    console.error("Wait for embedding completion");
    await morph.git.waitForEmbeddings({ repoId: "rust-daq" });
  } else {
    throw error;
  }
}
```

## Troubleshooting

### Issue: Semantic Search Returns No Results

**Causes:**
1. Repository not pushed to Morph
2. Embeddings not yet completed
3. Query too vague

**Solutions:**
```bash
# 1. Ensure code is pushed
cd scripts/morph
npm run push

# 2. Wait for embeddings
npm run wait-embeddings

# 3. Refine query
# Bad: "auth"
# Good: "JWT token validation in authentication middleware"
```

### Issue: Warp Grep Too Slow

**Causes:**
1. Large repository size
2. Overly broad query
3. Network latency

**Solutions:**
- Use `target_directories` parameter
- Make queries more specific
- Use local provider for large repos

### Issue: Git Push Not Triggering Embedding

**Causes:**
1. SSH remote instead of HTTPS
2. API key not configured
3. Repository not initialized with Morph

**Solutions:**
```bash
# 1. Check remote URL
git remote -v
# Should be: https://github.com/user/repo.git
# Not: git@github.com:user/repo.git

# 2. Update remote if needed
git remote set-url origin https://github.com/user/repo.git

# 3. Reinitialize with Morph
cd scripts/morph
npm run init
```

## Additional Resources

- [Morph SDK Documentation](https://docs.morphllm.com/sdk)
- [Semantic Search Guide](https://docs.morphllm.com/sdk/components/repos/semantic-search)
- [Git Operations Reference](https://docs.morphllm.com/sdk/components/repos/git-operations)
- [Warp Grep Tool Guide](https://docs.morphllm.com/sdk/components/warp-grep/tool)
- [Morph Dashboard](https://morph.so/dashboard)

## Examples for rust-daq

### Find All Async Hardware Callbacks

```typescript
// Search for async callback patterns
const results = await morph.repos.search({
  query: "async hardware callback BoxFuture implementation",
  repoId: "rust-daq",
  target_directories: ["src/hardware"],
  limit: 20
});

// Filter high-confidence matches
const relevant = results.filter(r => r.rerankScore > 0.7);
```

### Analyze Parameter Pattern Usage

```bash
# Use Warp Grep to find Parameter usage
cd scripts/morph
npm run warp-grep -- "Parameter trait implementation in drivers"
```

### Compare Driver Implementations

```typescript
// Embed multiple driver files
const drivers = [
  'src/hardware/pvcam.rs',
  'src/hardware/ell14.rs',
  'src/hardware/maitai.rs'
];

const embeddings = await Promise.all(
  drivers.map(file => morph.embed.file({ filepath: file }))
);

// Calculate similarity between drivers
const similarity = cosineSimilarity(embeddings[0].vector, embeddings[1].vector);
console.log(`PVCAM vs ELL14 similarity: ${similarity}`);
```

### Track Architectural Changes

```typescript
// Compare main vs feature branch
const mainResults = await morph.repos.search({
  query: "Parameter hardware callback implementation",
  repoId: "rust-daq",
  branch: "main"
});

const featureResults = await morph.repos.search({
  query: "Parameter hardware callback implementation",
  repoId: "rust-daq",
  branch: "feature/v5-migration"
});

// Analyze differences
console.log(`Main branch: ${mainResults.length} matches`);
console.log(`Feature branch: ${featureResults.length} matches`);
```

---

**Last Updated**: 2025-12-03
**Morph SDK Version**: Latest
**Related Issues**: See `.beads/daq.db` for Morph integration tracking
