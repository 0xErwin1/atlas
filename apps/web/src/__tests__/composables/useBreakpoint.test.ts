import { describe, expect, it } from 'vitest';
import { useBreakpoint } from '@/composables/useBreakpoint';

function setViewportWidth(width: number): void {
  Object.defineProperty(window, 'innerWidth', { value: width, configurable: true, writable: true });
  window.dispatchEvent(new Event('resize'));
}

describe('useBreakpoint', () => {
  it('reports mobile when the viewport is at or below the mobile breakpoint', () => {
    setViewportWidth(390);

    const { isMobile } = useBreakpoint();

    expect(isMobile.value).toBe(true);
  });

  it('reports desktop above the mobile breakpoint', () => {
    setViewportWidth(1280);

    const { isMobile } = useBreakpoint();

    expect(isMobile.value).toBe(false);
  });

  it('reacts to viewport resize', () => {
    setViewportWidth(1280);
    const { isMobile } = useBreakpoint();
    expect(isMobile.value).toBe(false);

    setViewportWidth(500);
    expect(isMobile.value).toBe(true);

    setViewportWidth(900);
    expect(isMobile.value).toBe(false);
  });

  it('treats exactly 767px as mobile and 768px as desktop', () => {
    setViewportWidth(767);
    const { isMobile } = useBreakpoint();
    expect(isMobile.value).toBe(true);

    setViewportWidth(768);
    expect(isMobile.value).toBe(false);
  });
});
