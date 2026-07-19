import { useUiStore } from '@/stores/ui';

/**
 * Writes text to the clipboard, surfacing the same "Clipboard is not available"
 * error banner every call site used to inline. Returns whether the write
 * succeeded so callers can drive their own transient "Copied" affordance.
 */
export function useCopyToClipboard() {
  const ui = useUiStore();

  async function copy(text: string): Promise<boolean> {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      ui.showBanner('Clipboard is not available', 'error');
      return false;
    }
  }

  return { copy };
}
