import { type InjectionKey, inject, provide } from 'vue';

/**
 * Bridges the persistent Docs shell (which owns the sidebar) to the routed view
 * content nested under it. The shell provides sidebar actions the content pane
 * still needs to trigger — e.g. the Notes tab strip "+" opening a new page in
 * the hoisted sidebar tree.
 */
export interface DocsShellApi {
  openNewPage: () => void;
}

const DOCS_SHELL_KEY: InjectionKey<DocsShellApi> = Symbol('docs-shell');

export function provideDocsShell(api: DocsShellApi): void {
  provide(DOCS_SHELL_KEY, api);
}

export function useDocsShell(): DocsShellApi | null {
  return inject(DOCS_SHELL_KEY, null);
}
