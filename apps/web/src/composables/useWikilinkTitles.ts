import { type Ref, ref, watch } from 'vue';
import { wrappedClient } from '@/api/wrapper';
import { collectWikilinkIds } from '@/lib/wikilink';

/**
 * Resolves the CURRENT title of every id-bound wikilink (`[[uuid|Title]]`) in the
 * given markdown, so a rendered link can show the target's live title instead of
 * the snapshot baked into the text (E04: rename auto-updates the display).
 *
 * Returns a reactive id → title map. Titles are fetched once per id (cached) and
 * re-resolved, debounced, as the body gains new id-bound links. The document
 * route resolves a uuid directly. Unresolved ids (deleted/forbidden) are left
 * out so the widget falls back to the snapshot title.
 */
export function useWikilinkTitles(ws: Ref<string>, body: Ref<string>): Ref<Record<string, string>> {
  const titles = ref<Record<string, string>>({});
  let timer: ReturnType<typeof setTimeout> | null = null;

  async function resolveMissing(): Promise<void> {
    if (ws.value === '') return;

    const missing = collectWikilinkIds(body.value).filter((id) => !(id in titles.value));
    if (missing.length === 0) return;

    const resolved: Record<string, string> = {};
    for (const id of missing) {
      try {
        const { data } = await wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}', {
          params: { path: { ws: ws.value, slug: id } },
        });
        if (data?.title) resolved[id] = data.title;
      } catch {
        // leave unresolved; the widget keeps the snapshot title
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
