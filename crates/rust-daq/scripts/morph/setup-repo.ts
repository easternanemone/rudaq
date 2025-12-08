#!/usr/bin/env npx tsx
/**
 * Setup Morph Repo Storage for rust-daq
 *
 * This script initializes the repository with Morph's Git integration,
 * enabling automatic code indexing for semantic search.
 *
 * Usage:
 *   export MORPH_API_KEY=sk-your-key
 *   npm run setup
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
    console.error('Get your key at: https://morphllm.com/dashboard/api-keys');
    process.exit(1);
  }

  // Get GitHub token from gh CLI for HTTPS authentication
  let ghToken: string;
  try {
    ghToken = execSync('gh auth token', { encoding: 'utf-8' }).trim();
    console.log('✓ Retrieved GitHub token from gh CLI\n');
  } catch (error) {
    console.error('Error: Could not get GitHub token from gh CLI');
    console.error('Please run: gh auth login');
    process.exit(1);
  }

  console.log('🚀 Setting up Morph Repo Storage for rust-daq...\n');

  const morph = new MorphClient({ apiKey });

  try {
    // Initialize the repository with Morph
    console.log('1. Initializing repository with Morph...');
    try {
      await morph.git.init({
        repoId: REPO_ID,
        dir: REPO_DIR,
        force: true, // Overwrite existing remote if present
      });
      console.log('   ✓ Repository initialized\n');
    } catch (initError: any) {
      if (initError.code === 'AlreadyExistsError') {
        console.log('   ✓ Repository already initialized, continuing...\n');
      } else {
        throw initError;
      }
    }

    // Get current branch
    const branch = execSync('git rev-parse --abbrev-ref HEAD', {
      cwd: REPO_DIR,
      encoding: 'utf-8'
    }).trim();
    console.log(`2. Current branch: ${branch}\n`);

    // Stage and push to trigger indexing
    console.log('3. Pushing to Morph for indexing...');
    console.log('   (Pushing to GitHub origin with HTTPS authentication)\n');
    await morph.git.push({
      dir: REPO_DIR,
      branch,
      onAuth: () => ({ 
        username: 'x-access-token',
        password: ghToken 
      }),
    });
    console.log('   ✓ Push complete, indexing started\n');

    // Wait for embeddings to complete
    console.log('4. Waiting for embeddings to complete...');
    await morph.git.waitForEmbeddings({
      repoId: REPO_ID,
      timeout: 300000, // 5 minutes
      onProgress: (progress) => {
        console.log(`   Processing: ${progress.filesProcessed}/${progress.totalFiles} files`);
      },
    });
    console.log('   ✓ Embeddings complete!\n');

    console.log('✅ Setup complete! You can now use semantic search.');
    console.log('\nExample search:');
    console.log('  npm run search -- "Where is camera exposure controlled?"');

  } catch (error) {
    console.error('Error during setup:', error);
    process.exit(1);
  }
}

main();
