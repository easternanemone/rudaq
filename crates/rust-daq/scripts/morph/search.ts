#!/usr/bin/env npx tsx
/**
 * Semantic Code Search using Morph
 *
 * Search the rust-daq codebase using natural language queries.
 * Uses two-stage retrieval: vector search (~240ms) + reranking (~630ms).
 *
 * Usage:
 *   export MORPH_API_KEY=sk-your-key
 *   npm run search -- "Where is camera exposure controlled?"
 *   npm run search -- --dir src/hardware "rotation mount driver"
 *   npm run search -- --branch feature-branch "gRPC service implementation"
 *   npm run search -- --commit abc123 "old implementation of FFT"
 */

import { MorphClient } from '@morphllm/morphsdk';

const REPO_ID = 'rust-daq';

interface SearchOptions {
  query: string;
  directories?: string[];
  branch?: string;
  commitHash?: string;
  limit?: number;
}

function parseArgs(): SearchOptions {
  const args = process.argv.slice(2);
  const options: SearchOptions = {
    query: '',
    directories: [],
    limit: 10,
  };

  let i = 0;
  while (i < args.length) {
    if (args[i] === '--dir' || args[i] === '-d') {
      options.directories!.push(args[++i]);
    } else if (args[i] === '--branch' || args[i] === '-b') {
      options.branch = args[++i];
    } else if (args[i] === '--commit' || args[i] === '-c') {
      options.commitHash = args[++i];
    } else if (args[i] === '--limit' || args[i] === '-n') {
      options.limit = parseInt(args[++i], 10);
    } else if (!args[i].startsWith('-')) {
      options.query = args[i];
    }
    i++;
  }

  return options;
}

async function main() {
  const apiKey = process.env.MORPH_API_KEY;
  if (!apiKey) {
    console.error('Error: MORPH_API_KEY environment variable is not set');
    process.exit(1);
  }

  const options = parseArgs();

  if (!options.query) {
    console.error('Usage: npm run search -- "your natural language query"');
    console.error('\nOptions:');
    console.error('  --dir, -d <path>     Search only in specific directory');
    console.error('  --branch, -b <name>  Search specific branch');
    console.error('  --commit, -c <hash>  Search specific commit');
    console.error('  --limit, -n <num>    Number of results (default: 10)');
    console.error('\nExamples:');
    console.error('  npm run search -- "Where is camera exposure controlled?"');
    console.error('  npm run search -- --dir src/hardware "rotation mount"');
    process.exit(1);
  }

  console.log(`🔍 Searching: "${options.query}"\n`);

  const morph = new MorphClient({ apiKey });

  try {
    const startTime = Date.now();

    const results = await morph.codebaseSearch.search({
      repoId: REPO_ID,
      query: options.query,
      directories: options.directories,
      branch: options.branch,
      commitHash: options.commitHash,
      limit: options.limit,
    });

    const elapsed = Date.now() - startTime;

    if (!results || !Array.isArray(results)) {
      console.log(`No results returned in ${elapsed}ms`);
      console.log('\n⚠️  The repository may not be indexed yet.');
      console.log('   Run: npm run setup');
      console.log('   Or:  npm run push');
      process.exit(0);
    }

    console.log(`Found ${results.length} results in ${elapsed}ms\n`);

    if (results.length === 0) {
      console.log('No matches found. Try a different query.');
      process.exit(0);
    }

    console.log('─'.repeat(80));

    for (const result of results) {
      const score = (result.relevanceScore * 100).toFixed(1);
      console.log(`\n📄 ${result.filePath}:${result.lineStart}-${result.lineEnd}`);
      console.log(`   Score: ${score}% | Language: ${result.language}`);
      console.log('');

      // Show code snippet with line numbers
      const lines = result.content.split('\n');
      let lineNum = result.lineStart;
      for (const line of lines.slice(0, 15)) { // Limit to 15 lines
        console.log(`   ${lineNum.toString().padStart(4)} │ ${line}`);
        lineNum++;
      }
      if (lines.length > 15) {
        console.log(`   ... (${lines.length - 15} more lines)`);
      }
      console.log('\n' + '─'.repeat(80));
    }

  } catch (error) {
    console.error('Search error:', error);
    process.exit(1);
  }
}

main();
