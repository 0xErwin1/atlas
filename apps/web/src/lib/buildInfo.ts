/**
 * Build identity for the running frontend bundle. The values are injected at
 * build time by Vite `define` (see vite.config.ts). The desktop app embeds this
 * same bundle via Tauri's `generate_context!`, so the commit here is the desktop
 * app's actual compiled commit as well.
 *
 * The `typeof` guards keep this safe under Vitest, whose standalone config does
 * not apply the production `define`, so the constants are absent there and the
 * accessor resolves to the development fallback instead of throwing.
 */

const DEV_FALLBACK = 'dev';

export const APP_VERSION: string =
  typeof __ATLAS_BUILD_VERSION__ === 'string' ? __ATLAS_BUILD_VERSION__ : DEV_FALLBACK;

export const APP_COMMIT: string =
  typeof __ATLAS_BUILD_COMMIT__ === 'string' ? __ATLAS_BUILD_COMMIT__ : DEV_FALLBACK;

/** A compact, copyable build label, e.g. `Version 0.0.0 · a1b2c3d`. */
export const BUILD_LABEL = `Version ${APP_VERSION} · ${APP_COMMIT}`;
