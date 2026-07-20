import type { RouteLocationRaw } from 'vue-router';
import type { TabRef } from '@/stores/notesTabs';

/**
 * The route a tab activates: a board tab lands on its board, a document tab on
 * its note. Shared by the tab strip, the sidebar delete flows, and cold-start
 * restore so every entry point routes a given tab identically.
 */
export function routeForTab(ref: TabRef): RouteLocationRaw {
  return ref.kind === 'board'
    ? { name: 'tasks', params: { boardId: ref.id } }
    : { name: 'notes', params: { slug: ref.id } };
}

/** Where to land after the active tab is closed and `next` is its replacement. */
export function routeAfterClose(next: TabRef | null): RouteLocationRaw {
  return next !== null ? routeForTab(next) : { name: 'notes' };
}
