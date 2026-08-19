import { mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import Popover from '@/components/ui/Popover.vue';

/**
 * The teleported surface tracks the trigger on scroll/resize. Those reads are
 * forced reflows, and a task list mounts three popovers per row, so they must be
 * coalesced to one per frame and cost nothing at all while the popover is closed.
 */

let frames: FrameRequestCallback[] = [];

function runFrames(): void {
  const pending = frames;
  frames = [];
  for (const frame of pending) frame(0);
}

function rect(top: number, bottom: number): DOMRect {
  return { top, bottom, left: 10, right: 60, width: 50, height: bottom - top } as DOMRect;
}

const mounted: { unmount: () => void }[] = [];

function mountPopover(open: boolean) {
  const wrapper = mount(Popover, {
    props: { teleport: true, open },
    slots: {
      trigger: '<button class="trigger">open</button>',
      default: '<div class="panel">body</div>',
    },
  });

  mounted.push(wrapper);
  return wrapper;
}

describe('Popover teleported surface', () => {
  beforeEach(() => {
    frames = [];
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal('cancelAnimationFrame', () => {
      frames = [];
    });
  });

  afterEach(() => {
    for (const wrapper of mounted.splice(0)) wrapper.unmount();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.innerHTML = '';
  });

  it('puts nothing in the document body while closed', () => {
    mountPopover(false);

    expect(document.body.childNodes).toHaveLength(0);
  });

  it('mounts the surface into the body once opened', async () => {
    const wrapper = mountPopover(false);

    await wrapper.setProps({ open: true });

    expect(document.body.querySelector('.panel')).not.toBeNull();
  });

  it('coalesces the scroll-driven layout reads into a single frame', async () => {
    const wrapper = mountPopover(false);
    await wrapper.setProps({ open: true });
    await nextTick();

    const reads = vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(rect(100, 120));
    frames = [];

    for (let i = 0; i < 5; i += 1) window.dispatchEvent(new Event('scroll'));

    expect(reads).not.toHaveBeenCalled();
    expect(frames).toHaveLength(1);

    runFrames();

    expect(reads).toHaveBeenCalledTimes(1);
  });

  it('keeps the panel anchored to the trigger while scrolling', async () => {
    const reads = vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(rect(100, 120));

    const wrapper = mountPopover(false);
    await wrapper.setProps({ open: true });
    await nextTick();
    runFrames();

    await nextTick();

    const panel = document.body.querySelector<HTMLElement>('.atl-popover-panel');
    expect(panel?.style.top).toBe('124px');

    reads.mockReturnValue(rect(40, 60));
    window.dispatchEvent(new Event('scroll'));
    runFrames();
    await nextTick();

    expect(panel?.style.top).toBe('64px');
  });

  it('drops a pending frame when it closes', async () => {
    const wrapper = mountPopover(false);
    await wrapper.setProps({ open: true });
    await nextTick();

    const reads = vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue(rect(100, 120));
    frames = [];

    window.dispatchEvent(new Event('scroll'));
    await wrapper.setProps({ open: false });
    runFrames();

    expect(reads).not.toHaveBeenCalled();
  });

  it('registers the viewport listeners passively', async () => {
    const listeners = vi.spyOn(window, 'addEventListener');

    const wrapper = mountPopover(false);
    await wrapper.setProps({ open: true });

    const scroll = listeners.mock.calls.find(([type]) => type === 'scroll');
    expect(scroll?.[2]).toMatchObject({ passive: true, capture: true });
  });
});
