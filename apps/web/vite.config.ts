import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath, URL } from 'node:url';
import tailwindcss from '@tailwindcss/vite';
import vue from '@vitejs/plugin-vue';
import { defineConfig } from 'vite';

/**
 * Resolves the app version stamped into the build from package.json, the single
 * JS-readable source of truth for the frontend version.
 */
function resolveBuildVersion(): string {
  try {
    const manifest = readFileSync(fileURLToPath(new URL('./package.json', import.meta.url)), 'utf8');
    const version = (JSON.parse(manifest) as { version?: unknown }).version;
    return typeof version === 'string' && version.length > 0 ? version : '0.0.0';
  } catch {
    return '0.0.0';
  }
}

/**
 * Resolves the short commit stamped into the build. A CI- or caller-provided
 * value wins (GITHUB_SHA in CI, VITE_ATLAS_BUILD_COMMIT for other pipelines);
 * otherwise the local git checkout is queried. When neither is available — for
 * example a source tree built without a .git directory — it falls back to
 * "unknown" so the build never fails on the missing commit.
 */
function resolveBuildCommit(): string {
  const provided = process.env.VITE_ATLAS_BUILD_COMMIT ?? process.env.GITHUB_SHA;
  if (provided !== undefined && provided.trim().length > 0) {
    return provided.trim().slice(0, 7);
  }

  try {
    return execSync('git rev-parse --short HEAD', {
      stdio: ['ignore', 'pipe', 'ignore'],
    })
      .toString()
      .trim();
  } catch {
    return 'unknown';
  }
}

export default defineConfig({
  define: {
    __ATLAS_BUILD_VERSION__: JSON.stringify(resolveBuildVersion()),
    __ATLAS_BUILD_COMMIT__: JSON.stringify(resolveBuildCommit()),
  },
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    host: '0.0.0.0',
    port: 5173,
    allowedHosts: true,
    proxy: {
      '/api': 'http://localhost:8080',
      '/openapi.json': 'http://localhost:8080',
    },
  },
});
