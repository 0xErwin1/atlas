import { afterEach, describe, expect, it, vi } from 'vitest';
import { createBrowserPlatformTransport } from '@/platform/browser';
import {
  getPlatformTransport,
  resetPlatformTransportForTest,
  setPlatformTransport,
} from '@/platform/transport';
import { fakePlatformTransport } from '../helpers/platformTransport';

class FakeEventSource {
  readyState = 0;
  onopen: ((event: Event) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;

  constructor(readonly url: string) {}

  addEventListener(): void {}

  close(): void {
    this.readyState = 2;
  }
}

function desktopTransport() {
  return fakePlatformTransport({
    createWorkspaceEventSource: vi.fn(() => new FakeEventSource('desktop://events')),
    saveDownload: () => Promise.resolve({ data: { path: '/downloads/file' } }),
  });
}

describe('platform transport', () => {
  afterEach(() => {
    resetPlatformTransportForTest();
    vi.unstubAllGlobals();
  });

  it('uses the browser transport when live updates start before main bootstraps', () => {
    vi.stubGlobal('EventSource', FakeEventSource);

    const source = getPlatformTransport().createWorkspaceEventSource('acme');

    expect(source).toBeInstanceOf(FakeEventSource);
    expect((source as FakeEventSource).url).toBe('/api/workspaces/acme/events');
  });

  it('allows the desktop bootstrap to override the browser default and test reset restores it', () => {
    vi.stubGlobal('EventSource', FakeEventSource);
    const desktop = desktopTransport();
    setPlatformTransport(desktop);

    expect(getPlatformTransport()).toBe(desktop);
    expect(getPlatformTransport().createWorkspaceEventSource('acme')).toBeInstanceOf(FakeEventSource);

    resetPlatformTransportForTest();
    const browserSource = getPlatformTransport().createWorkspaceEventSource('acme');

    expect(browserSource).toBeInstanceOf(FakeEventSource);
    expect((browserSource as FakeEventSource).url).toBe('/api/workspaces/acme/events');
  });

  it('returns explicit desktop-unavailable failures for desktop-only preferences in the browser', async () => {
    const transport = createBrowserPlatformTransport();

    await expect(transport.getStartOnLogin()).resolves.toEqual({
      error: 'Start on login is available in Atlas Desktop',
    });
    await expect(transport.setSystemTray(true)).resolves.toEqual({
      error: 'System tray settings are available in Atlas Desktop',
    });
  });
});
