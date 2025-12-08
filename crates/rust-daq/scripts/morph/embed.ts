#!/usr/bin/env npx tsx
/**
 * Morph Embedding API Utility
 *
 * Generate code embeddings using morph-embedding-v3.
 * Useful for custom similarity search, clustering, or analysis.
 *
 * Usage:
 *   export MORPH_API_KEY=sk-your-key
 *   npm run embed -- --file src/hardware/camera.rs
 *   npm run embed -- --text "async fn capture_frame"
 *   npm run embed -- --file src/hardware/camera.rs --output embeddings.json
 */

import OpenAI from 'openai';
import * as fs from 'fs';
import * as path from 'path';

const MORPH_API_URL = 'https://api.morphllm.com/v1';
const EMBEDDING_MODEL = 'morph-embedding-v3';
const EMBEDDING_DIMENSIONS = 1024;

interface EmbedOptions {
  file?: string;
  text?: string;
  output?: string;
  verbose?: boolean;
}

function parseArgs(): EmbedOptions {
  const args = process.argv.slice(2);
  const options: EmbedOptions = {};

  let i = 0;
  while (i < args.length) {
    if (args[i] === '--file' || args[i] === '-f') {
      options.file = args[++i];
    } else if (args[i] === '--text' || args[i] === '-t') {
      options.text = args[++i];
    } else if (args[i] === '--output' || args[i] === '-o') {
      options.output = args[++i];
    } else if (args[i] === '--verbose' || args[i] === '-v') {
      options.verbose = true;
    }
    i++;
  }

  return options;
}

function chunkText(text: string, maxChunkSize: number = 8000): string[] {
  const lines = text.split('\n');
  const chunks: string[] = [];
  let currentChunk = '';

  for (const line of lines) {
    if ((currentChunk + line + '\n').length > maxChunkSize) {
      if (currentChunk) {
        chunks.push(currentChunk.trim());
      }
      currentChunk = line + '\n';
    } else {
      currentChunk += line + '\n';
    }
  }

  if (currentChunk.trim()) {
    chunks.push(currentChunk.trim());
  }

  return chunks;
}

async function main() {
  const apiKey = process.env.MORPH_API_KEY;
  if (!apiKey) {
    console.error('Error: MORPH_API_KEY environment variable is not set');
    process.exit(1);
  }

  const options = parseArgs();

  if (!options.file && !options.text) {
    console.error('Usage: npm run embed -- --file <path> | --text "<code>"');
    console.error('\nOptions:');
    console.error('  --file, -f <path>    Embed a file');
    console.error('  --text, -t "<code>"  Embed inline text');
    console.error('  --output, -o <path>  Save embeddings to JSON file');
    console.error('  --verbose, -v        Show detailed output');
    process.exit(1);
  }

  let content: string;
  let sourceName: string;

  if (options.file) {
    const filePath = path.resolve(options.file);
    if (!fs.existsSync(filePath)) {
      console.error(`File not found: ${filePath}`);
      process.exit(1);
    }
    content = fs.readFileSync(filePath, 'utf-8');
    sourceName = options.file;
    console.log(`📄 Embedding file: ${sourceName}`);
  } else {
    content = options.text!;
    sourceName = 'inline text';
    console.log(`📝 Embedding text (${content.length} chars)`);
  }

  const client = new OpenAI({
    apiKey,
    baseURL: MORPH_API_URL,
  });

  try {
    const chunks = chunkText(content);
    console.log(`   Chunks: ${chunks.length}`);
    console.log(`   Model: ${EMBEDDING_MODEL}`);
    console.log(`   Dimensions: ${EMBEDDING_DIMENSIONS}\n`);

    const startTime = Date.now();

    const response = await client.embeddings.create({
      model: EMBEDDING_MODEL,
      input: chunks,
    });

    const elapsed = Date.now() - startTime;
    console.log(`✅ Generated ${response.data.length} embedding(s) in ${elapsed}ms`);

    const result = {
      source: sourceName,
      model: EMBEDDING_MODEL,
      dimensions: EMBEDDING_DIMENSIONS,
      chunks: chunks.map((chunk, i) => ({
        index: i,
        text: options.verbose ? chunk : chunk.slice(0, 100) + (chunk.length > 100 ? '...' : ''),
        embedding: response.data[i].embedding,
      })),
      metadata: {
        totalChunks: chunks.length,
        generatedAt: new Date().toISOString(),
        elapsedMs: elapsed,
      }
    };

    if (options.output) {
      const outputPath = path.resolve(options.output);
      fs.writeFileSync(outputPath, JSON.stringify(result, null, 2));
      console.log(`   Saved to: ${outputPath}`);
    }

    if (options.verbose) {
      console.log('\nEmbedding preview (first 10 values of first chunk):');
      console.log('  ', result.chunks[0].embedding.slice(0, 10));
    }

    // Show usage stats
    console.log(`\nUsage:`);
    console.log(`   Total tokens: ${response.usage?.total_tokens || 'N/A'}`);

  } catch (error) {
    console.error('Embedding error:', error);
    process.exit(1);
  }
}

main();
