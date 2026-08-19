import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it } from 'vitest';
import ContextSidebar from '@/components/shell/ContextSidebar.vue';

function mountSidebar() {
  return mount(ContextSidebar, {
    global: {
      stubs: {
        WorkspaceSwitcher: true,
      },
    },
  });
}

function pointerEvent(type: string, pointerId: number, clientX: number, button = 0): PointerEvent {
  const event = new MouseEvent(type, { bubbles: true, clientX, button });
  Object.defineProperty(event, 'pointerId', { value: pointerId });
  return event as PointerEvent;
}

function separator(wrapper: ReturnType<typeof mountSidebar>) {
  return wrapper.get('[role="separator"]');
}

async function startResize(wrapper: ReturnType<typeof mountSidebar>, pointerId: number): Promise<void> {
  separator(wrapper).element.dispatchEvent(pointerEvent('pointerdown', pointerId, 100));
  await wrapper.vm.$nextTick();
}

afterEach(() => {
  document.body.style.cursor = '';
  document.body.style.userSelect = '';
});

describe('ContextSidebar', () => {
  it('starts at the transient default width with the accessible separator contract', () => {
    const wrapper = mountSidebar();
    const handle = separator(wrapper);
    const aside = wrapper.get('aside');

    expect(handle.attributes('aria-label')).toBe('Resize sidebar');
    expect(handle.attributes('aria-orientation')).toBe('vertical');
    expect(handle.attributes('aria-valuemin')).toBe('218');
    expect(handle.attributes('aria-valuemax')).toBe('480');
    expect(handle.attributes('aria-valuenow')).toBe('218');
    expect(aside.attributes('style')).toContain('width: 218px');
    expect(aside.attributes('style')).toContain('flex: 0 0 218px');
  });

  it('clamps pointer resizing and ignores unrelated pointers', async () => {
    const wrapper = mountSidebar();

    await startResize(wrapper, 1);
    window.dispatchEvent(pointerEvent('pointermove', 2, 500));
    await wrapper.vm.$nextTick();
    expect(separator(wrapper).attributes('aria-valuenow')).toBe('218');

    window.dispatchEvent(pointerEvent('pointermove', 1, 500));
    await wrapper.vm.$nextTick();
    expect(separator(wrapper).attributes('aria-valuenow')).toBe('480');

    window.dispatchEvent(pointerEvent('pointermove', 1, -500));
    await wrapper.vm.$nextTick();
    expect(separator(wrapper).attributes('aria-valuenow')).toBe('218');
  });

  it('cleans up pointer listeners and restores body styles after pointer end, cancellation, blur, and unmount', async () => {
    document.body.style.cursor = 'crosshair';
    document.body.style.userSelect = 'text';

    const wrapper = mountSidebar();
    await startResize(wrapper, 1);

    expect(document.body.style.cursor).toBe('col-resize');
    expect(document.body.style.userSelect).toBe('none');

    window.dispatchEvent(pointerEvent('pointerup', 1, 100));
    expect(document.body.style.cursor).toBe('crosshair');
    expect(document.body.style.userSelect).toBe('text');

    await startResize(wrapper, 2);
    window.dispatchEvent(pointerEvent('pointercancel', 2, 100));
    expect(document.body.style.cursor).toBe('crosshair');
    expect(document.body.style.userSelect).toBe('text');

    window.dispatchEvent(pointerEvent('pointermove', 1, 400));
    expect(separator(wrapper).attributes('aria-valuenow')).toBe('218');

    await startResize(wrapper, 3);
    window.dispatchEvent(new Event('blur'));
    expect(document.body.style.cursor).toBe('crosshair');

    await startResize(wrapper, 4);
    wrapper.unmount();
    expect(document.body.style.cursor).toBe('crosshair');
    expect(document.body.style.userSelect).toBe('text');
  });

  it('resizes by keyboard steps and bounds', async () => {
    const wrapper = mountSidebar();
    const handle = separator(wrapper);

    await handle.trigger('keydown', { key: 'ArrowRight' });
    expect(handle.attributes('aria-valuenow')).toBe('234');

    await handle.trigger('keydown', { key: 'ArrowDown' });
    expect(handle.attributes('aria-valuenow')).toBe('218');

    await handle.trigger('keydown', { key: 'Home' });
    expect(handle.attributes('aria-valuenow')).toBe('218');

    await handle.trigger('keydown', { key: 'End' });
    expect(handle.attributes('aria-valuenow')).toBe('480');

    await handle.trigger('keydown', { key: 'ArrowRight' });
    expect(handle.attributes('aria-valuenow')).toBe('480');
  });

  it('resets the width when a new sidebar instance mounts', async () => {
    const first = mountSidebar();
    await separator(first).trigger('keydown', { key: 'End' });
    expect(separator(first).attributes('aria-valuenow')).toBe('480');
    first.unmount();

    const second = mountSidebar();
    expect(separator(second).attributes('aria-valuenow')).toBe('218');
  });
});
