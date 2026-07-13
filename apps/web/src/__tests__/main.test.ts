import { afterEach, describe, expect, it, vi } from 'vitest';

const { disposeWorkspaceLiveUpdates } = vi.hoisted(() => ({
  disposeWorkspaceLiveUpdates: vi.fn(),
}));

vi.mock('@/lib/workspaceLiveUpdates', () => ({ disposeWorkspaceLiveUpdates }));
vi.mock('vue', async (importOriginal) => ({
  ...(await importOriginal<typeof import('vue')>()),
  createApp: () => ({ mount: vi.fn(), use: vi.fn() }),
}));
vi.mock('pinia', async (importOriginal) => ({
  ...(await importOriginal<typeof import('pinia')>()),
  createPinia: vi.fn(),
}));
vi.mock('@/router/index', () => ({ router: {} }));

import { registerWorkspaceLiveUpdatesPagehide } from '@/main';

describe('workspace live update page lifecycle', () => {
  afterEach(() => {
    disposeWorkspaceLiveUpdates.mockClear();
  });

  it('disposes on a non-persisted pagehide and registers the listener only once', () => {
    const cleanup = registerWorkspaceLiveUpdatesPagehide();
    registerWorkspaceLiveUpdatesPagehide();

    window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: false }));

    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
    cleanup();
  });

  it('retains the broker for persisted bfcache pagehide events', () => {
    const cleanup = registerWorkspaceLiveUpdatesPagehide();

    window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true }));

    expect(disposeWorkspaceLiveUpdates).not.toHaveBeenCalled();
    cleanup();
  });
});
