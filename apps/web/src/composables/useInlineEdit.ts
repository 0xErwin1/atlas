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
      inputRef.value?.focus();
      if (select) inputRef.value?.select();
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
