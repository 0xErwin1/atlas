import { type ComputedRef, computed, ref } from 'vue';

/**
 * Viewport width at or below which the app switches to its mobile layout
 * (bottom tab bar, full-screen sidebars, bottom sheets). Mirrors the 390px
 * phone artboards of the hi-fi responsive design.
 */
const MOBILE_MAX_WIDTH = 767;

const DEFAULT_WIDTH = 1024;

const viewportWidth = ref(typeof window !== 'undefined' ? window.innerWidth : DEFAULT_WIDTH);

if (typeof window !== 'undefined') {
  window.addEventListener('resize', () => {
    viewportWidth.value = window.innerWidth;
  });
}

const isMobile = computed(() => viewportWidth.value <= MOBILE_MAX_WIDTH);

/**
 * Shared, reactive breakpoint state. A single module-level resize listener feeds
 * one reactive width, so every consumer observes the same `isMobile` flag without
 * registering its own listener.
 */
export function useBreakpoint(): {
  isMobile: ComputedRef<boolean>;
  viewportWidth: typeof viewportWidth;
} {
  return { isMobile, viewportWidth };
}
