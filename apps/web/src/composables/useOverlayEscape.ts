import { computed, type MaybeRefOrGetter, onBeforeUnmount, toValue } from 'vue';
import { installKeymapListener, registerShortcut } from '@/composables/useKeymap';
import { KEYMAP_PRIORITIES } from '@/lib/keymap';

export function useOverlayEscape(
  enabled: MaybeRefOrGetter<boolean>,
  close: (event: KeyboardEvent) => void,
): () => void {
  const uninstallListener = installKeymapListener();
  const unregister = registerShortcut({
    id: 'escape',
    enabled: computed(() => toValue(enabled)),
    priority: KEYMAP_PRIORITIES.overlay,
    allowInText: true,
    handler: (event) => {
      close(event);
    },
  });

  function cleanup(): void {
    unregister();
    uninstallListener();
  }

  onBeforeUnmount(cleanup);
  return cleanup;
}
