import type { Extension } from '@codemirror/state';
import { EditorView } from '@codemirror/view';

/**
 * Reads the CSP nonce that the Tauri asset pipeline stamps on the inline
 * `<style>` in `index.html`. Per the CSP spec, once a nonce appears in
 * `style-src` the `'unsafe-inline'` keyword is ignored, so in the desktop app
 * every nonce-less runtime `<style>` — including the one CodeMirror injects via
 * style-mod — is silently blocked. Reusing that stamped nonce is the only way
 * for runtime-injected styles to pass the served CSP.
 *
 * The `nonce` IDL property must be used here: browsers hide the content
 * attribute for security, so `getAttribute('nonce')` returns `''`.
 *
 * Returns `''` when no nonced style exists (web deployment, dev server).
 */
export function documentStyleNonce(): string {
  return document.querySelector<HTMLStyleElement>('style[nonce]')?.nonce ?? '';
}

/**
 * CodeMirror extension that forwards the document's CSP nonce to style-mod, so
 * the editor stylesheet it injects carries the nonce and survives the desktop
 * CSP. Resolves to no extension when the document has no nonce, leaving web and
 * dev behavior unchanged.
 */
export function cspNonceExtension(): Extension {
  const nonce = documentStyleNonce();

  if (nonce === '') return [];

  return EditorView.cspNonce.of(nonce);
}
