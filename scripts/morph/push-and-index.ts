#!/usr/bin/env npx tsx
/**
 * Push changes and wait for Morph indexing
 *
 * Pushes the current branch to Morph and waits for embedding completion.
 * Use this after making changes to keep the search index up-to-date.
 *
 * Usage:
 *   export MORPH_API_KEY=sk-your-key
 *   npm run push
 *   npm run push -- --no-wait    # Don't wait for embeddings
 */

import { MorphClient } from '@morphllm/morphsdk';
import { execSync } from 'child_process';
import * as path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const REPO_ID = 'rust-daq';
const REPO_DIR = path.resolve(__dirname, '../..');

async function main() {
  const apiKey = process.env.MORPH_API_KEY;
  if (!apiKey) {
    console.error('Error: MORPH_API_KEY environment variable is not set');
    process.exit(1);
  }

  const noWait = process.argv.includes('--no-wait');

  // Get current branch
  const branch = execSync('git rev-parse --abbrev-ref HEAD', {
    cwd: REPO_DIR,
    encoding: 'utf-8'
  }).trim();

  // Get commit info
  const commitHash = execSync('git rev-parse --short HEAD', {
    cwd: REPO_DIR,
    encoding: 'utf-8'
  }).trim();

  const commitMsg = execSync('git log -1 --format=%s', {
    cwd: REPO_DIR,
    encoding: 'utf-8'
  }).trim();

  console.log(`📤 Pushing to Morph...`);
  console.log(`   Branch: ${branch}`);
  console.log(`   Commit: ${commitHash} - ${commitMsg}`);
  console.log(`   (Morph handles Azure DevOps authentication automatically)\n`);

  const morph = new MorphClient({ apiKey });

  try {
    const startTime = Date.now();

    await morph.git.push({
      dir: REPO_DIR,
      branch,
    });

    console.log('   ✓ Push complete\n');

    if (noWait) {
      console.log('⏭️  Skipping embedding wait (--no-wait)');
      console.log('   Indexing will complete in background (3-100s)');
      return;
    }

    console.log('⏳ Waiting for embeddings...');
    await morph.git.waitForEmbeddings({
      repoId: REPO_ID,
      timeout: 300000, // 5 minutes
      onProgress: (progress) => {
        const percent = Math.round((progress.filesProcessed / progress.totalFiles) * 100);
        process.stdout.write(`\r   ${progress.filesProcessed}/${progress.totalFiles} files (${percent}%)`);
      },
    });

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
    console.log(`\n\n✅ Indexing complete in ${elapsed}s`);
    console.log('   Search index is now up-to-date.');

  } catch (error) {
    console.error('Error:', error);
    process.exit(1);
  }
}

main();
