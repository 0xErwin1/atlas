import { ref } from 'vue';
import type { WikilinkRef } from '@/lib/wikilink';

export interface WikilinkCaret {
  left: number;
  top: number;
}

interface EditorTarget {
  insertWikilink: (ref: WikilinkRef) => void;
}

interface SuggestTarget {
  open: boolean;
  moveDown: () => void;
  moveUp: () => void;
  confirmActive: () => void;
}

/**
 * Shared wiring for the `[[wikilink]]` autocomplete used by every markdown editor
 * that supports it (the note editor and the task description). It holds the active
 * `[[` query and the caret anchor for positioning the suggestion list, inserts the
 * chosen reference into the editor, and routes the arrow/enter/escape keys to the
 * list while it is open.
 *
 * The host owns the editor and suggestion component refs and passes them as
 * getters, so this composable only threads them together — the glue lives here
 * once instead of being duplicated per editor host.
 */
export function useWikilinkSuggest(editor: () => EditorTarget | null, suggest: () => SuggestTarget | null) {
  const query = ref<string | null>(null);
  const caret = ref<WikilinkCaret | null>(null);

  function reset(): void {
    query.value = null;
    caret.value = null;
  }

  function onQuery(nextQuery: string | null, nextCaret: WikilinkCaret | null): void {
    query.value = nextQuery;
    caret.value = nextCaret;
  }

  function onSelect(ref: WikilinkRef): void {
    editor()?.insertWikilink(ref);
    reset();
  }

  // Only intercept navigation keys while the suggestion list is open; otherwise
  // the editor handles them normally.
  function onKeydown(event: KeyboardEvent): void {
    const list = suggest();
    if (list?.open !== true) return;

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      list.moveDown();
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      list.moveUp();
    } else if (event.key === 'Enter') {
      event.preventDefault();
      list.confirmActive();
    } else if (event.key === 'Escape') {
      reset();
    }
  }

  return { query, caret, onQuery, onSelect, onKeydown };
}
