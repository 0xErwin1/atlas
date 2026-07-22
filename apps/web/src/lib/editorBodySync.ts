/**
 * Debounced sync of the editor document into Vue reactive body state.
 *
 * While typing, CodeMirror remains the source of truth. Reactive dependents
 * (wikilink title resolution, `:body` prop mirrors) observe a delayed copy so
 * every keystroke does not thrash Vue's reactivity graph.
 */

/** Delay before mirroring editor markdown into the reactive body ref. */
export const EDITOR_BODY_SYNC_MS = 300;

export interface BodySyncScheduler {
  /** Queue markdown to apply after the debounce window (latest wins). */
  schedule: (markdown: string) => void;
  /** Apply any pending markdown immediately. */
  flush: () => void;
  /** Drop any pending sync without applying. */
  cancel: () => void;
  /** True when a debounced apply is scheduled. */
  readonly isPending: boolean;
}

/**
 * Creates a debounce scheduler that eventually calls `apply` with the latest
 * markdown string. Used by Notes so `body` is not rewritten on every keystroke.
 */
export function createBodySyncScheduler(
  apply: (markdown: string) => void,
  delayMs: number = EDITOR_BODY_SYNC_MS,
): BodySyncScheduler {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: string | null = null;

  const clearTimer = (): void => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };

  const applyPending = (): void => {
    if (pending === null) return;

    const value = pending;
    pending = null;
    apply(value);
  };

  return {
    schedule(markdown: string): void {
      pending = markdown;
      clearTimer();
      timer = setTimeout(() => {
        timer = null;
        applyPending();
      }, delayMs);
    },

    flush(): void {
      clearTimer();
      applyPending();
    },

    cancel(): void {
      clearTimer();
      pending = null;
    },

    get isPending(): boolean {
      return timer !== null;
    },
  };
}
