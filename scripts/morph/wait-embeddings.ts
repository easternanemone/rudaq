#!/usr/bin/env npx tsx
/**
 * Wait for Morph Embeddings to Complete
 *
 * Useful in CI/CD pipelines to ensure search index is up-to-date
 * before running tests or deployment.
 *
 * Usage:
 *   export MORPH_API_KEY=sk-your-key
 *   npm run git:wait-embeddings
 */

import { MorphClient } from '@morphllm/morphsdk';

const REPO_ID = 'rust-daq';

async function main() {
  const apiKey = process.env.MORPH_API_KEY;
  if (!apiKey) {
    console.error('Error: MORPH_API_KEY environment variable is not set');
    process.exit(1);
  }

  console.log('⏳ Waiting for embeddings to complete...\n');

  const morph = new MorphClient({ apiKey });

  try {
    const startTime = Date.now();

    await morph.git.waitForEmbeddings({
      repoId: REPO_ID,
      timeout: 300000, // 5 minutes
      onProgress: (progress) => {
        const percent = Math.round((progress.filesProcessed / progress.totalFiles) * 100);
        process.stdout.write(`\r   ${progress.filesProcessed}/${progress.totalFiles} files (${percent}%)`);
      },
    });

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
    console.log(`\n\n✅ Embeddings complete in ${elapsed}s`);
    console.log('   Search index is now up-to-date.');

  } catch (error) {
    console.error('Error:', error);
    process.exit(1);
  }
}

main();
