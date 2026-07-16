import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getWindowDecorations, setWindowDecorations } = vi.hoisted(() => ({
  getWindowDecorations: vi.fn(),
  setWindowDecorations: vi.fn(),
}));

vi.mock('@/platform/transport', () => ({
  getPlatformTransport: () => ({
    isDesktop: true,
    getWindowDecorations,
    setWindowDecorations,
  }),
}));

import AppSettingsPanel from '@/components/settings/AppSettingsPanel.vue';

function activeOptionLabel(wrapper: ReturnType<typeof mount>): string | undefined {
  return wrapper.find('button.atl-seg-opt.on').text();
}

async function mountPanel() {
  const wrapper = mount(AppSettingsPanel);
  await flushPromises();
  return wrapper;
}

describe('AppSettingsPanel', () => {
  beforeEach(() => {
    getWindowDecorations.mockReset();
    setWindowDecorations.mockReset();
    getWindowDecorations.mockResolvedValue({ data: { window_decorations: true } });
    setWindowDecorations.mockResolvedValue({ data: { window_decorations: false } });
  });

  it('reads the stored preference on mount and marks the matching option active', async () => {
    getWindowDecorations.mockResolvedValue({ data: { window_decorations: false } });

    const wrapper = await mountPanel();

    expect(getWindowDecorations).toHaveBeenCalledTimes(1);
    expect(activeOptionLabel(wrapper)).toBe('Off');
  });

  it('shows decorations on when the stored preference is on', async () => {
    const wrapper = await mountPanel();

    expect(activeOptionLabel(wrapper)).toBe('On');
  });

  it('persists the boolean the chosen option maps to', async () => {
    const wrapper = await mountPanel();

    await wrapper.findAll('button.atl-seg-opt')[1]?.trigger('click');
    await flushPromises();

    expect(setWindowDecorations).toHaveBeenCalledWith(false);
    expect(activeOptionLabel(wrapper)).toBe('Off');
  });

  it('turns decorations back on from the off state', async () => {
    getWindowDecorations.mockResolvedValue({ data: { window_decorations: false } });
    setWindowDecorations.mockResolvedValue({ data: { window_decorations: true } });

    const wrapper = await mountPanel();
    await wrapper.findAll('button.atl-seg-opt')[0]?.trigger('click');
    await flushPromises();

    expect(setWindowDecorations).toHaveBeenCalledWith(true);
    expect(activeOptionLabel(wrapper)).toBe('On');
  });

  it('keeps the previous value and surfaces a message when the transport fails', async () => {
    setWindowDecorations.mockResolvedValue({ error: 'desktop window is unavailable' });

    const wrapper = await mountPanel();
    await wrapper.findAll('button.atl-seg-opt')[1]?.trigger('click');
    await flushPromises();

    expect(activeOptionLabel(wrapper)).toBe('On');
    expect(wrapper.text()).toContain('Unable to change the window decorations');
  });

  it('falls back to decorations on when the stored preference cannot be read', async () => {
    getWindowDecorations.mockResolvedValue({ error: 'desktop configuration is unavailable' });

    const wrapper = await mountPanel();

    expect(activeOptionLabel(wrapper)).toBe('On');
  });
});
