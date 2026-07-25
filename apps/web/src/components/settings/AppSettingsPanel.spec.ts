import { type DOMWrapper, flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const {
  getWindowDecorations,
  setWindowDecorations,
  getZoom,
  setZoom,
  getStartOnLogin,
  setStartOnLogin,
  getSystemTray,
  setSystemTray,
} = vi.hoisted(() => ({
  getWindowDecorations: vi.fn(),
  setWindowDecorations: vi.fn(),
  getZoom: vi.fn(),
  setZoom: vi.fn(),
  getStartOnLogin: vi.fn(),
  setStartOnLogin: vi.fn(),
  getSystemTray: vi.fn(),
  setSystemTray: vi.fn(),
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
    getStartOnLogin,
    setStartOnLogin,
    getSystemTray,
    setSystemTray,
  }),
}));

import AppSettingsPanel from '@/components/settings/AppSettingsPanel.vue';

function activeOptionLabel(wrapper: ReturnType<typeof mount>): string | undefined {
  return wrapper.find('button.atl-seg-opt.on').text();
}

function checkbox(
  wrapper: ReturnType<typeof mount>,
  label: string,
): Omit<DOMWrapper<HTMLInputElement>, 'exists'> {
  return wrapper.get<HTMLInputElement>(`input[aria-label="${label}"]`);
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
    getStartOnLogin.mockReset();
    setStartOnLogin.mockReset();
    getSystemTray.mockReset();
    setSystemTray.mockReset();
    getWindowDecorations.mockResolvedValue({ data: { window_decorations: true } });
    setWindowDecorations.mockResolvedValue({ data: { window_decorations: false } });
    getZoom.mockResolvedValue({ data: { window_decorations: true, zoom_factor: 1 } });
    setZoom.mockResolvedValue({ data: { window_decorations: true, zoom_factor: 1.1 } });
    getStartOnLogin.mockResolvedValue({ data: { start_on_login: false } });
    setStartOnLogin.mockResolvedValue({ data: { start_on_login: true } });
    getSystemTray.mockResolvedValue({ data: { system_tray: true } });
    setSystemTray.mockResolvedValue({ data: { system_tray: false } });
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

  it('uses the host-returned autostart and tray preferences', async () => {
    getStartOnLogin.mockResolvedValue({ data: { start_on_login: true } });
    getSystemTray.mockResolvedValue({ data: { system_tray: false } });

    const wrapper = await mountPanel();

    expect(checkbox(wrapper, 'Start on login').element.checked).toBe(true);
    expect(checkbox(wrapper, 'Show system tray icon').element.checked).toBe(false);
  });

  it('shows the host error and retains the known autostart value when reading fails', async () => {
    getStartOnLogin.mockResolvedValue({ error: 'Autostart preference is unavailable' });

    const wrapper = await mountPanel();

    expect(checkbox(wrapper, 'Start on login').element.checked).toBe(false);
    expect(wrapper.text()).toContain('Autostart preference is unavailable');
  });

  it('shows the host error and retains the known tray value when reading fails', async () => {
    getSystemTray.mockResolvedValue({ error: 'System tray preference is unavailable' });

    const wrapper = await mountPanel();

    expect(checkbox(wrapper, 'Show system tray icon').element.checked).toBe(true);
    expect(wrapper.text()).toContain('System tray preference is unavailable');
  });

  it('shows an actionable fallback and retains the known autostart value when the read rejects', async () => {
    getStartOnLogin.mockRejectedValue(new Error('ipc channel closed'));

    const wrapper = await mountPanel();

    expect(checkbox(wrapper, 'Start on login').element.checked).toBe(false);
    expect(wrapper.text()).toContain('Unable to read the start on login setting');
  });

  it('shows an actionable fallback and retains the known tray value when the read rejects', async () => {
    getSystemTray.mockRejectedValue(new Error('ipc channel closed'));

    const wrapper = await mountPanel();

    expect(checkbox(wrapper, 'Show system tray icon').element.checked).toBe(true);
    expect(wrapper.text()).toContain('Unable to read the system tray setting');
  });

  it('keeps the autostart checkbox checked and shows the host error when writing fails', async () => {
    getStartOnLogin.mockResolvedValue({ data: { start_on_login: true } });
    setStartOnLogin.mockResolvedValue({ error: 'Unable to disable login launch' });

    const wrapper = await mountPanel();
    await checkbox(wrapper, 'Start on login').setValue(false);
    await flushPromises();

    expect(checkbox(wrapper, 'Start on login').element.checked).toBe(true);
    expect(wrapper.text()).toContain('Unable to disable login launch');
  });

  it('disables desktop preference controls while a write is pending', async () => {
    let resolveWrite: ((value: { data: { start_on_login: boolean } }) => void) | undefined;
    setStartOnLogin.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveWrite = resolve;
        }),
    );

    const wrapper = await mountPanel();
    await checkbox(wrapper, 'Start on login').setValue(true);

    expect(wrapper.get('input[aria-label="Start on login"]').attributes('disabled')).toBeDefined();
    expect(wrapper.get('input[aria-label="Show system tray icon"]').attributes('disabled')).toBeDefined();

    resolveWrite?.({ data: { start_on_login: true } });
    await flushPromises();

    expect(wrapper.get('input[aria-label="Start on login"]').attributes('disabled')).toBeUndefined();
  });

  it('uses the host-returned tray value and explains that restarting is required', async () => {
    setSystemTray.mockResolvedValue({ data: { system_tray: true } });

    const wrapper = await mountPanel();
    await checkbox(wrapper, 'Show system tray icon').setValue(false);
    await flushPromises();

    expect(setSystemTray).toHaveBeenCalledWith(false);
    expect(checkbox(wrapper, 'Show system tray icon').element.checked).toBe(true);
    expect(wrapper.text()).toContain('Restart Atlas Desktop to apply this change.');
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
