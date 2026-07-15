import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { useResourceStatusStore } from '@/stores/resourceStatus';

describe('useResourceStatusStore', () => {
  beforeEach(() => setActivePinia(createPinia()));

  it('keeps usable data visible when retrying after a failed request', () => {
    const store = useResourceStatusStore();

    store.setReady('note-b', true);
    store.setError('note-b', true);
    store.setRefreshing('note-b');

    expect(store.statusFor('note-b')).toBe('error-with-data');
    expect(store.usesFullLoader('note-b')).toBe(false);
  });

  it('uses a full loader only while an empty resource is loading', () => {
    const store = useResourceStatusStore();

    store.setRefreshing('task-b');

    expect(store.statusFor('task-b')).toBe('empty');
    expect(store.usesFullLoader('task-b')).toBe(true);
  });

  it('keeps hydrated data visible when an authoritative request fails despite an online hint, then recovers', () => {
    const store = useResourceStatusStore();

    store.setReady('note-b', true);
    store.beginRequest('note-b', true);
    store.recordRequestFailure('note-b', true);

    expect(store.statusFor('note-b')).toBe('error-with-data');
    expect(store.usesFullLoader('note-b')).toBe(false);

    store.beginRequest('note-b', false);
    store.recordRequestSuccess('note-b', true);

    expect(store.statusFor('note-b')).toBe('ready');
  });

  it('uses offline status only when a failed request agrees with the offline hint', () => {
    const store = useResourceStatusStore();

    store.setReady('task-b', true);
    store.beginRequest('task-b', false);
    store.recordRequestFailure('task-b', false);

    expect(store.statusFor('task-b')).toBe('offline');
    expect(store.usesFullLoader('task-b')).toBe(false);
  });
});
