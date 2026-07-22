import { afterEach, describe, expect, it, vi } from 'vitest';
import { createBodySyncScheduler, EDITOR_BODY_SYNC_MS } from '@/lib/editorBodySync';

describe('createBodySyncScheduler', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('does not apply immediately — only after the debounce window', () => {
    vi.useFakeTimers();
    const apply = vi.fn();
    const sync = createBodySyncScheduler(apply);

    sync.schedule('first');
    expect(apply).not.toHaveBeenCalled();

    vi.advanceTimersByTime(EDITOR_BODY_SYNC_MS - 1);
    expect(apply).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(apply).toHaveBeenCalledTimes(1);
    expect(apply).toHaveBeenCalledWith('first');
  });

  it('coalesces rapid keystrokes into a single apply of the latest markdown', () => {
    vi.useFakeTimers();
    const apply = vi.fn();
    const sync = createBodySyncScheduler(apply);

    sync.schedule('a');
    sync.schedule('ab');
    sync.schedule('abc');
    vi.advanceTimersByTime(EDITOR_BODY_SYNC_MS);

    expect(apply).toHaveBeenCalledTimes(1);
    expect(apply).toHaveBeenCalledWith('abc');
  });

  it('flush applies pending markdown immediately and cancel drops it', () => {
    vi.useFakeTimers();
    const apply = vi.fn();
    const sync = createBodySyncScheduler(apply);

    sync.schedule('pending');
    sync.flush();
    expect(apply).toHaveBeenCalledWith('pending');

    apply.mockClear();
    sync.schedule('dropped');
    sync.cancel();
    vi.advanceTimersByTime(EDITOR_BODY_SYNC_MS);
    expect(apply).not.toHaveBeenCalled();
  });
});
