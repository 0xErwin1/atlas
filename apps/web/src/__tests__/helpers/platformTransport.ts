import { vi } from 'vitest';
import type { PlatformTransport } from '@/platform/transport';

/**
 * Builds a fully-typed `PlatformTransport` fake for tests. Defaults resolve
 * to a known, non-empty `publicBase` and a successful `openExternal` so most
 * call sites only need to override the one or two members they exercise.
 */
export function fakePlatformTransport(overrides: Partial<PlatformTransport> = {}): PlatformTransport {
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
    getZoom: vi.fn(),
    setZoom: vi.fn(),
    getStartOnLogin: vi.fn(),
    setStartOnLogin: vi.fn(),
    getSystemTray: vi.fn(),
    setSystemTray: vi.fn(),
    createWorkspaceEventSource: vi.fn(),
    readClipboardImage: vi.fn(async () => null),
    saveDownload: vi.fn(),
    openExternal: vi.fn(async () => ({})),
    publicBase: () => 'https://atlas.test',
    ...overrides,
  };
}
