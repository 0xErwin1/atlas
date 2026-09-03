import { type Ref, ref, watch } from 'vue';
import { wrappedClient } from '@/api/wrapper';
import { collectWikilinkTitleKeys } from '@/lib/wikilink';

/**
 * Resolves the CURRENT title of every wikilink target in the given markdown, so
 * a rendered link shows the target's live title instead of the snapshot baked
 * into the text (E04: rename auto-updates the display).
 *
 * Returns a reactive key → title map, keyed the way `wikilinkTitleKey` keys
 * them. Titles are fetched once per key (cached) and re-resolved, debounced, as
 * the body gains new links. Unresolved keys (deleted/forbidden) are left out so
 * the widget falls back to the text written in the markdown.
 */
export function useWikilinkTitles(ws: Ref<string>, body: Ref<string>): Ref<Record<string, string>> {
  const titles = ref<Record<string, string>>({});
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function resolveKey(workspace: string, key: string): Promise<string | null> {
    const readableId = key.startsWith('task:') ? key.slice('task:'.length) : null;

    if (readableId !== null) {
      const { data } = await wrappedClient.GET('/api/v2/acta/workspaces/{ws}/tasks/{readable_id}', {
        params: { path: { ws: workspace, readable_id: readableId } },
      });
      return data?.title ?? null;
    }

    // A `note:` key carries the slug; a bare key is a document uuid, and the
    // document route resolves either one.
    const slug = key.startsWith('note:') ? key.slice('note:'.length) : key;
    const { data } = await wrappedClient.GET('/api/v2/acta/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws: workspace, slug } },
    });
    return data?.title ?? null;
  }

  async function resolveMissing(): Promise<void> {
    if (ws.value === '') return;

    const missing = collectWikilinkTitleKeys(body.value).filter((key) => !(key in titles.value));
    if (missing.length === 0) return;

    const resolved: Record<string, string> = {};
    for (const key of missing) {
      try {
        const title = await resolveKey(ws.value, key);
        if (title !== null) resolved[key] = title;
      } catch {
        // leave unresolved; the widget keeps the text from the markdown
      }
    }

    if (Object.keys(resolved).length > 0) {
      titles.value = { ...titles.value, ...resolved };
    }
  }

  watch(
    [ws, body],
    () => {
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(() => void resolveMissing(), 300);
    },
    { immediate: true },
  );

  return titles;
}
