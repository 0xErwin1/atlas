import { nextTick, type Ref, ref, shallowRef } from 'vue';

/**
 * Shared inline create/rename input state for sidebar trees.
 *
 * `T` is an opaque descriptor of what is being edited (e.g. `{ kind: 'new-folder',
 * parentId }`). `start` opens the input and focuses it; `commit` calls `onCommit`
 * with the trimmed value and descriptor when non-empty, then resets. The active
 * descriptor is cleared BEFORE `onCommit` runs so committing on blur right after
 * Enter is a harmless no-op (no double submit).
 */
export function useInlineEdit<T>(onCommit: (value: string, ctx: T) => void) {
  const active = shallowRef<T | null>(null);
  const value = ref('');
  const inputRef = ref<HTMLInputElement | null>(null) as Ref<HTMLInputElement | null>;

  function start(ctx: T, initial = '', select = false): void {
    active.value = ctx;
    value.value = initial;
    void nextTick(() => {
      // A `ref` placed on an element inside `v-for` is populated by Vue as an
      // array; the inline inputs live in the sidebar's project/board loops, so
      // unwrap before focusing — otherwise the focus call silently no-ops.
      const target = inputRef.value as HTMLInputElement | HTMLInputElement[] | null;
      const el = Array.isArray(target) ? target[0] : target;
      el?.focus();
      if (select) el?.select();
    });
  }

  function commit(): void {
    const name = value.value.trim();
    const ctx = active.value;

    if (name === '' || ctx === null) {
      cancel();
      return;
    }

    active.value = null;
    value.value = '';
    onCommit(name, ctx);
  }

  function cancel(): void {
    active.value = null;
    value.value = '';
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      commit();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      cancel();
    }
  }

  return { active, value, inputRef, start, commit, cancel, onKeydown };
}
