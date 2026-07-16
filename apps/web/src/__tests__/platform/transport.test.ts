import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  getPlatformTransport,
  type PlatformTransport,
  resetPlatformTransportForTest,
  setPlatformTransport,
} from '@/platform/transport';

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

function desktopTransport(): PlatformTransport {
  return {
    isDesktop: true,
    login: vi.fn(),
    me: vi.fn(),
    resume: vi.fn(),
    logout: vi.fn(),
    getOrigin: vi.fn(),
    setOrigin: vi.fn(),
    getWindowDecorations: vi.fn(),
    setWindowDecorations: vi.fn(),
    createWorkspaceEventSource: vi.fn(() => new FakeEventSource('desktop://events')),
  };
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
});
