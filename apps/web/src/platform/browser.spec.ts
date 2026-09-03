import { afterEach, describe, expect, it, vi } from 'vitest';
import { createBrowserPlatformTransport } from './browser';

describe('createBrowserPlatformTransport — openExternal', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('opens the URL in a new tab without an opener and resolves to an empty result', async () => {
    const openSpy = vi.spyOn(window, 'open').mockReturnValue({} as Window);
    const transport = createBrowserPlatformTransport();

    await expect(transport.openExternal('https://example.com/path')).resolves.toEqual({});

    expect(openSpy).toHaveBeenCalledWith('https://example.com/path', '_blank', 'noopener,noreferrer');
  });

  it('still resolves to an empty result when window.open returns null', async () => {
    vi.spyOn(window, 'open').mockReturnValue(null);
    const transport = createBrowserPlatformTransport();

    await expect(transport.openExternal('https://example.com')).resolves.toEqual({});
  });
});

describe('createBrowserPlatformTransport — publicBase', () => {
  it('returns the current window origin', () => {
    const transport = createBrowserPlatformTransport();

    expect(transport.publicBase()).toBe(globalThis.location.origin);
  });
});

describe('createBrowserPlatformTransport — createWorkspaceEventSource', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('constructs an EventSource targeting the V2 acta events route for the given workspace', () => {
    const captured: string[] = [];
    class FakeEventSource {
      constructor(url: string) {
        captured.push(url);
      }
    }
    vi.stubGlobal('EventSource', FakeEventSource);

    const transport = createBrowserPlatformTransport();
    transport.createWorkspaceEventSource('acme');

    expect(captured).toEqual(['/api/v2/acta/workspaces/acme/events']);
  });
});
