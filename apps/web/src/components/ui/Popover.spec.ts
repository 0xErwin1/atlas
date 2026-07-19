import { mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { h, nextTick } from 'vue';
import Popover from '@/components/ui/Popover.vue';

interface TriggerScope {
  open: boolean;
  toggle: () => void;
  close: () => void;
}

let activeWrapper: VueWrapper | null = null;

function mountPopover(
  props: Record<string, unknown> = {},
  panelContent: () => ReturnType<typeof h> = () => h('button', { 'data-test': 'inner' }, 'inner'),
): VueWrapper {
  const wrapper = mount(Popover, {
    props: { teleport: true, ...props },
    slots: {
      trigger: (scope: TriggerScope) =>
        h('button', { 'data-test': 'trigger', onClick: scope.toggle }, 'open'),
      default: panelContent,
    },
    attachTo: document.body,
  });
  activeWrapper = wrapper;
  return wrapper;
}

function domRect(over: Partial<DOMRect> = {}): DOMRect {
  const base = { x: 0, y: 0, top: 0, bottom: 0, left: 0, right: 0, width: 0, height: 0 };
  return { ...base, ...over, toJSON: () => ({}) } as DOMRect;
}

function teleportedPanel(): HTMLElement | null {
  return document.body.querySelector<HTMLElement>('.atl-popover-panel.fixed');
}

async function openViaTrigger(wrapper: VueWrapper): Promise<void> {
  await wrapper.find('[data-test="trigger"]').trigger('click');
  await nextTick();
  await nextTick();
}

async function closeViaBackdrop(): Promise<void> {
  const backdrop = document.body.querySelector<HTMLElement>('.atl-popover-backdrop.fixed');
  if (backdrop === null) throw new Error('expected the popover backdrop');
  backdrop.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await nextTick();
}

beforeEach(() => {
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  document.body.innerHTML = '';
});

describe('Popover — panel role', () => {
  it('defaults the panel to role="menu"', async () => {
    const wrapper = mountPopover();

    await openViaTrigger(wrapper);

    expect(teleportedPanel()?.getAttribute('role')).toBe('menu');
  });

  it('applies the configured role and aria-label to the panel', async () => {
    const wrapper = mountPopover({ role: 'dialog', ariaLabel: 'Choose a date' });

    await openViaTrigger(wrapper);

    const panel = teleportedPanel();
    expect(panel?.getAttribute('role')).toBe('dialog');
    expect(panel?.getAttribute('aria-label')).toBe('Choose a date');
  });

  it('applies the role to the non-teleported panel too', async () => {
    const wrapper = mountPopover({ teleport: false, role: 'listbox' });

    await openViaTrigger(wrapper);

    expect(wrapper.find('.atl-popover-panel').attributes('role')).toBe('listbox');
  });
});

describe('Popover — teleported focus management', () => {
  it('moves focus to the first focusable element in the panel on open', async () => {
    const wrapper = mountPopover();

    (wrapper.find('[data-test="trigger"]').element as HTMLElement).focus();
    await openViaTrigger(wrapper);

    expect(document.activeElement).toBe(document.body.querySelector('[data-test="inner"]'));
  });

  it('falls back to the panel itself when it has no focusable content', async () => {
    const wrapper = mountPopover({}, () => h('span', 'plain text'));

    (wrapper.find('[data-test="trigger"]').element as HTMLElement).focus();
    await openViaTrigger(wrapper);

    expect(document.activeElement).toBe(teleportedPanel());
    expect(teleportedPanel()?.getAttribute('tabindex')).toBe('-1');
  });

  it('restores focus to the previously focused trigger on close', async () => {
    const wrapper = mountPopover();

    const trigger = wrapper.find('[data-test="trigger"]').element as HTMLElement;
    trigger.focus();
    await openViaTrigger(wrapper);
    expect(document.activeElement).not.toBe(trigger);

    await closeViaBackdrop();

    expect(document.activeElement).toBe(trigger);
  });

  it('leaves focus untouched in non-teleported mode', async () => {
    const wrapper = mountPopover({ teleport: false });

    const trigger = wrapper.find('[data-test="trigger"]').element as HTMLElement;
    trigger.focus();
    await openViaTrigger(wrapper);

    expect(document.activeElement).toBe(trigger);
  });
});

describe('Popover — teleported repositioning', () => {
  it('attaches capture-phase scroll and resize listeners while open and detaches them on close', async () => {
    const addSpy = vi.spyOn(window, 'addEventListener');
    const removeSpy = vi.spyOn(window, 'removeEventListener');

    const wrapper = mountPopover();
    await openViaTrigger(wrapper);

    expect(addSpy).toHaveBeenCalledWith('scroll', expect.any(Function), {
      capture: true,
      passive: true,
    });
    expect(addSpy).toHaveBeenCalledWith('resize', expect.any(Function), { passive: true });

    await closeViaBackdrop();

    expect(removeSpy).toHaveBeenCalledWith('scroll', expect.any(Function), { capture: true });
    expect(removeSpy).toHaveBeenCalledWith('resize', expect.any(Function));
  });

  it('recomputes the panel position when the window scrolls', async () => {
    const wrapper = mountPopover();
    const anchorEl = wrapper.element as HTMLElement;
    const rectSpy = vi
      .spyOn(anchorEl, 'getBoundingClientRect')
      .mockReturnValue(domRect({ top: 100, bottom: 120, left: 40, right: 140 }));

    await openViaTrigger(wrapper);
    expect(teleportedPanel()?.style.top).toBe('124px');

    rectSpy.mockReturnValue(domRect({ top: 50, bottom: 70, left: 40, right: 140 }));
    window.dispatchEvent(new Event('scroll'));
    await nextTick();

    expect(teleportedPanel()?.style.top).toBe('74px');
  });

  it('stops repositioning after close', async () => {
    const wrapper = mountPopover();
    const anchorEl = wrapper.element as HTMLElement;
    const rectSpy = vi
      .spyOn(anchorEl, 'getBoundingClientRect')
      .mockReturnValue(domRect({ top: 100, bottom: 120, left: 40, right: 140 }));

    await openViaTrigger(wrapper);
    await closeViaBackdrop();

    rectSpy.mockClear();
    window.dispatchEvent(new Event('scroll'));
    await nextTick();

    expect(rectSpy).not.toHaveBeenCalled();
  });

  it('flips a bottom placement to the top when the panel would overflow below', async () => {
    const wrapper = mountPopover();
    const anchorEl = wrapper.element as HTMLElement;
    const bottom = window.innerHeight - 30;
    vi.spyOn(anchorEl, 'getBoundingClientRect').mockReturnValue(
      domRect({ top: bottom - 20, bottom, left: 40, right: 140 }),
    );

    await openViaTrigger(wrapper);

    const panel = teleportedPanel();
    if (panel === null) throw new Error('expected the teleported panel');
    Object.defineProperty(panel, 'offsetHeight', { value: 200, configurable: true });

    window.dispatchEvent(new Event('scroll'));
    await nextTick();

    expect(panel.style.bottom).toBe(`${window.innerHeight - (bottom - 20) + 4}px`);
    expect(panel.style.top).toBe('');
  });

  it('clamps the panel to the right viewport edge', async () => {
    const wrapper = mountPopover();
    const anchorEl = wrapper.element as HTMLElement;
    const left = window.innerWidth - 50;
    vi.spyOn(anchorEl, 'getBoundingClientRect').mockReturnValue(
      domRect({ top: 100, bottom: 120, left, right: left + 40 }),
    );

    await openViaTrigger(wrapper);

    const panel = teleportedPanel();
    if (panel === null) throw new Error('expected the teleported panel');
    Object.defineProperty(panel, 'offsetWidth', { value: 300, configurable: true });

    window.dispatchEvent(new Event('scroll'));
    await nextTick();

    expect(panel.style.left).toBe(`${window.innerWidth - 300}px`);
  });
});
