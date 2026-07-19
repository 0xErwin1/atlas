import { mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { nextTick } from 'vue';
import DatePicker from '@/components/ui/DatePicker.vue';

let activeWrapper: VueWrapper | null = null;

function mountPicker(modelValue = ''): VueWrapper {
  const wrapper = mount(DatePicker, { props: { modelValue }, attachTo: document.body });
  activeWrapper = wrapper;
  return wrapper;
}

// The panel teleports to <body>, so its nodes live outside the wrapper.
function bodyEl<T extends Element = HTMLElement>(selector: string): T | null {
  return document.body.querySelector<T>(selector);
}

async function clickBody(selector: string): Promise<void> {
  const el = bodyEl(selector);
  if (el === null) throw new Error(`element not found: ${selector}`);
  el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await nextTick();
}

async function openPanel(wrapper: VueWrapper): Promise<void> {
  await wrapper.find('[data-dp-trigger]').trigger('click');
  await nextTick();
  await nextTick();
}

function monthLabel(): string {
  return bodyEl('[data-dp-month]')?.textContent?.trim() ?? '';
}

beforeEach(() => {
  document.body.innerHTML = '';
});

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  document.body.innerHTML = '';
});

describe('DatePicker — month navigation', () => {
  it('rolls the view over year boundaries via prev/next', async () => {
    const wrapper = mountPicker('2024-01-10');

    await openPanel(wrapper);
    expect(monthLabel()).toBe('January 2024');

    await clickBody('[data-dp-prev]');
    expect(monthLabel()).toBe('December 2023');

    await clickBody('[data-dp-next]');
    await clickBody('[data-dp-next]');
    expect(monthLabel()).toBe('February 2024');
  });

  it('syncs the view when the model changes externally', async () => {
    const wrapper = mountPicker('2024-01-10');

    await openPanel(wrapper);
    expect(monthLabel()).toBe('January 2024');

    await wrapper.setProps({ modelValue: '2025-06-05' });
    await nextTick();

    expect(monthLabel()).toBe('June 2025');
    expect(bodyEl('[data-dp-day="5"]')?.classList.contains('atl-dp-day--selected')).toBe(true);
  });
});

describe('DatePicker — invalid model handling', () => {
  it('treats a non-date string as unset and opens on the current month', async () => {
    const wrapper = mountPicker('not-a-date');

    expect(wrapper.find('[data-dp-trigger]').text()).toContain('No date');

    await openPanel(wrapper);

    const now = new Date();
    expect(monthLabel()).toBe(`${now.toLocaleString('en-US', { month: 'long' })} ${now.getFullYear()}`);
    expect(bodyEl('.atl-dp-day--selected')).toBeNull();
  });
});

describe('DatePicker — selection and clear', () => {
  it('picking a day emits the date string and closes the panel', async () => {
    const wrapper = mountPicker('2024-01-10');

    await openPanel(wrapper);
    await clickBody('[data-dp-day="15"]');

    const emitted = wrapper.emitted('update:modelValue');
    expect(emitted?.at(-1)).toEqual(['2024-01-15']);
    expect(bodyEl('[data-dp-panel]')).toBeNull();
  });

  it('the clear button emits the empty string and closes the panel', async () => {
    const wrapper = mountPicker('2024-01-10');

    await openPanel(wrapper);
    await clickBody('[data-dp-clear]');

    const emitted = wrapper.emitted('update:modelValue');
    expect(emitted?.at(-1)).toEqual(['']);
    expect(bodyEl('[data-dp-panel]')).toBeNull();
  });
});

describe('DatePicker — week layout', () => {
  it('starts the week on Monday (June 2025 begins on a Sunday → six leading blanks)', async () => {
    const wrapper = mountPicker('2025-06-15');

    await openPanel(wrapper);

    const cells = Array.from(document.body.querySelectorAll('[data-dp-grid] > *'));
    expect(cells.slice(0, 6).every((c) => c.hasAttribute('data-dp-empty'))).toBe(true);
    expect(cells[6]?.getAttribute('data-dp-day')).toBe('1');
  });

  it('puts a month that begins on Monday in the first column (September 2025)', async () => {
    const wrapper = mountPicker('2025-09-15');

    await openPanel(wrapper);

    const first = document.body.querySelector('[data-dp-grid] > *');
    expect(first?.getAttribute('data-dp-day')).toBe('1');
  });
});

describe('DatePicker — roles and focus', () => {
  it('exposes the panel as a labeled dialog with plain day buttons', async () => {
    const wrapper = mountPicker('');

    await openPanel(wrapper);

    const panel = bodyEl('.atl-popover-panel');
    expect(panel?.getAttribute('role')).toBe('dialog');
    expect(panel?.getAttribute('aria-label')).toBe('Choose a date');

    const day = bodyEl('[data-dp-day="1"]');
    expect(day?.hasAttribute('role')).toBe(false);
    expect(day?.hasAttribute('aria-pressed')).toBe(false);
    expect(day?.getAttribute('aria-label')).not.toBeNull();

    expect(bodyEl('[data-dp-panel]')?.hasAttribute('role')).toBe(false);
  });

  it('marks today with aria-current="date"', async () => {
    const wrapper = mountPicker('');

    await openPanel(wrapper);

    const todayDay = new Date().getDate();
    expect(bodyEl(`[data-dp-day="${todayDay}"]`)?.getAttribute('aria-current')).toBe('date');
    expect(document.body.querySelectorAll('[aria-current="date"]')).toHaveLength(1);
  });

  it('moves focus into the panel on open and restores the trigger on close', async () => {
    const wrapper = mountPicker('');

    const trigger = wrapper.find('[data-dp-trigger]').element as HTMLElement;
    trigger.focus();

    await openPanel(wrapper);
    expect(document.activeElement).toBe(bodyEl('[data-dp-prev]'));

    await clickBody('.atl-popover-backdrop');
    await nextTick();
    expect(document.activeElement).toBe(trigger);
  });
});
