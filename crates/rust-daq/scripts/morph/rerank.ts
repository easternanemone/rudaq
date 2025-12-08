#!/usr/bin/env npx tsx
/**
 * Morph Rerank API Utility
 *
 * Reorder search results by relevance using morph-rerank-v3.
 * Can be used with any search results, not just Morph searches.
 *
 * Usage:
 *   export MORPH_API_KEY=sk-your-key
 *
 *   # Rerank from JSON file
 *   npm run rerank -- --query "camera capture" --input results.json
 *
 *   # Rerank inline documents
 *   npm run rerank -- --query "async camera" --docs "fn capture()" --docs "struct Camera"
 *
 *   # Pipe from grep/ripgrep
 *   rg -l "camera" src/ | npm run rerank -- --query "exposure control" --stdin
 */

import * as fs from 'fs';
import * as path from 'path';
import * as readline from 'readline';

const MORPH_API_URL = 'https://api.morphllm.com/v1/rerank';
const RERANK_MODEL = 'morph-rerank-v3';

interface RerankOptions {
  query: string;
  documents?: string[];
  inputFile?: string;
  stdin?: boolean;
  topN?: number;
}

function parseArgs(): RerankOptions {
  const args = process.argv.slice(2);
  const options: RerankOptions = {
    query: '',
    documents: [],
    topN: 10,
  };

  let i = 0;
  while (i < args.length) {
    if (args[i] === '--query' || args[i] === '-q') {
      options.query = args[++i];
    } else if (args[i] === '--docs' || args[i] === '-d') {
      options.documents!.push(args[++i]);
    } else if (args[i] === '--input' || args[i] === '-i') {
      options.inputFile = args[++i];
    } else if (args[i] === '--stdin') {
      options.stdin = true;
    } else if (args[i] === '--top' || args[i] === '-n') {
      options.topN = parseInt(args[++i], 10);
    }
    i++;
  }

  return options;
}

async function readStdin(): Promise<string[]> {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    terminal: false,
  });

  const lines: string[] = [];
  for await (const line of rl) {
    if (line.trim()) {
      lines.push(line.trim());
    }
  }
  return lines;
}

async function main() {
  const apiKey = process.env.MORPH_API_KEY;
  if (!apiKey) {
    console.error('Error: MORPH_API_KEY environment variable is not set');
    process.exit(1);
  }

  const options = parseArgs();

  if (!options.query) {
    console.error('Usage: npm run rerank -- --query "search query" [options]');
    console.error('\nOptions:');
    console.error('  --query, -q <text>   Query to rank against (required)');
    console.error('  --docs, -d <text>    Document to rank (can repeat)');
    console.error('  --input, -i <file>   JSON file with documents array');
    console.error('  --stdin              Read documents from stdin');
    console.error('  --top, -n <num>      Number of results (default: 10)');
    console.error('\nExamples:');
    console.error('  npm run rerank -- -q "camera exposure" -d "fn set_exposure()" -d "struct Camera"');
    console.error('  rg -l "camera" | npm run rerank -- -q "exposure" --stdin');
    process.exit(1);
  }

  let documents: string[] = options.documents || [];

  // Load from file
  if (options.inputFile) {
    const filePath = path.resolve(options.inputFile);
    const data = JSON.parse(fs.readFileSync(filePath, 'utf-8'));
    if (Array.isArray(data)) {
      documents = data;
    } else if (data.documents) {
      documents = data.documents;
    }
  }

  // Read from stdin
  if (options.stdin) {
    const stdinDocs = await readStdin();
    // If these are file paths, read them
    for (const doc of stdinDocs) {
      if (fs.existsSync(doc)) {
        const content = fs.readFileSync(doc, 'utf-8');
        documents.push(`${doc}:\n${content.slice(0, 2000)}`);
      } else {
        documents.push(doc);
      }
    }
  }

  if (documents.length === 0) {
    console.error('Error: No documents to rerank');
    console.error('Provide documents via --docs, --input, or --stdin');
    process.exit(1);
  }

  console.log(`🔄 Reranking ${documents.length} documents`);
  console.log(`   Query: "${options.query}"`);
  console.log(`   Model: ${RERANK_MODEL}\n`);

  try {
    const startTime = Date.now();

    const response = await fetch(MORPH_API_URL, {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${apiKey}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        model: RERANK_MODEL,
        query: options.query,
        documents,
        top_n: options.topN,
      }),
    });

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`API error: ${response.status} - ${error}`);
    }

    const result = await response.json();
    const elapsed = Date.now() - startTime;

    console.log(`✅ Reranked in ${elapsed}ms\n`);
    console.log('─'.repeat(80));

    for (const item of result.results) {
      const score = (item.relevance_score * 100).toFixed(1);
      const docText = typeof item.document === 'string' ? item.document : item.document?.text || '';
      const preview = docText.slice(0, 100).replace(/\n/g, ' ');
      console.log(`\n[${item.index}] Score: ${score}%`);
      console.log(`    ${preview}${docText.length > 100 ? '...' : ''}`);
    }

    console.log('\n' + '─'.repeat(80));

  } catch (error) {
    console.error('Rerank error:', error);
    process.exit(1);
  }
}

main();
