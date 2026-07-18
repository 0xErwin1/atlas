import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getWindowDecorations, setWindowDecorations, getZoom, setZoom } = vi.hoisted(() => ({
  getWindowDecorations: vi.fn(),
  setWindowDecorations: vi.fn(),
  getZoom: vi.fn(),
  setZoom: vi.fn(),
}));

vi.mock('@/platform/transport', () => ({
  DEFAULT_ZOOM_FACTOR: 1,
  MIN_ZOOM_FACTOR: 0.5,
  MAX_ZOOM_FACTOR: 3,
  ZOOM_FACTOR_STEP: 0.1,
  getPlatformTransport: () => ({
    isDesktop: true,
    getWindowDecorations,
    setWindowDecorations,
    getZoom,
    setZoom,
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
    getZoom.mockReset();
    setZoom.mockReset();
    getWindowDecorations.mockResolvedValue({ data: { window_decorations: true } });
    setWindowDecorations.mockResolvedValue({ data: { window_decorations: false } });
    getZoom.mockResolvedValue({ data: { window_decorations: true, zoom_factor: 1 } });
    setZoom.mockResolvedValue({ data: { window_decorations: true, zoom_factor: 1.1 } });
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

  it('keeps the previous value and surfaces the message the host reported', async () => {
    setWindowDecorations.mockResolvedValue({ error: 'desktop window is unavailable' });

    const wrapper = await mountPanel();
    await wrapper.findAll('button.atl-seg-opt')[1]?.trigger('click');
    await flushPromises();

    expect(activeOptionLabel(wrapper)).toBe('On');
    expect(wrapper.text()).toContain('desktop window is unavailable');
  });

  it('falls back to decorations on when the stored preference cannot be read', async () => {
    getWindowDecorations.mockResolvedValue({ error: 'desktop configuration is unavailable' });

    const wrapper = await mountPanel();

    expect(activeOptionLabel(wrapper)).toBe('On');
  });

  it('recovers when the bridge itself rejects instead of returning a result', async () => {
    setWindowDecorations.mockRejectedValue(new Error('ipc channel closed'));

    const wrapper = await mountPanel();
    await wrapper.findAll('button.atl-seg-opt')[1]?.trigger('click');
    await flushPromises();

    expect(wrapper.text()).toContain('Unable to change the window decorations');

    setWindowDecorations.mockResolvedValue({ data: { window_decorations: false } });
    await wrapper.findAll('button.atl-seg-opt')[1]?.trigger('click');
    await flushPromises();

    expect(activeOptionLabel(wrapper)).toBe('Off');
  });

  it('stays usable when reading the stored preference rejects', async () => {
    getWindowDecorations.mockRejectedValue(new Error('ipc channel closed'));

    const wrapper = await mountPanel();
    await wrapper.findAll('button.atl-seg-opt')[1]?.trigger('click');
    await flushPromises();

    expect(activeOptionLabel(wrapper)).toBe('Off');
  });

  it('reflects the stored zoom factor on mount', async () => {
    getZoom.mockResolvedValue({ data: { window_decorations: true, zoom_factor: 1.5 } });

    const wrapper = await mountPanel();

    expect(getZoom).toHaveBeenCalledTimes(1);
    expect(wrapper.find('.atl-zoom-value').text()).toBe('150%');
  });

  it('zooms in by one step and syncs to the value the host reports', async () => {
    const wrapper = await mountPanel();

    await wrapper.find('button[aria-label="Zoom in"]').trigger('click');
    await flushPromises();

    expect(setZoom).toHaveBeenCalledWith(expect.closeTo(1.1, 5));
    expect(wrapper.find('.atl-zoom-value').text()).toBe('110%');
  });

  it('keeps the previous zoom and surfaces the message the host reported', async () => {
    setZoom.mockResolvedValue({ error: 'desktop window zoom is unavailable' });

    const wrapper = await mountPanel();
    await wrapper.find('button[aria-label="Zoom in"]').trigger('click');
    await flushPromises();

    expect(wrapper.find('.atl-zoom-value').text()).toBe('100%');
    expect(wrapper.text()).toContain('desktop window zoom is unavailable');
  });
});
