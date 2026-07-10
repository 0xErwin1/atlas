import { describe, expect, it } from 'vitest';
import {
  createNoteResourceState,
  flushThenLoadNoteResource,
  type NoteTarget,
  runNoteResourceLoad,
} from '@/views/Notes.vue';

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;

  return {
    promise: new Promise<T>((resolvePromise, rejectPromise) => {
      resolve = resolvePromise;
      reject = rejectPromise;
    }),
    resolve,
    reject,
  };
}

const noteA: NoteTarget = { workspaceSlug: 'alpha', slug: 'note-a' };
const noteB: NoteTarget = { workspaceSlug: 'alpha', slug: 'note-b' };
const noteAInBeta: NoteTarget = { workspaceSlug: 'beta', slug: 'note-a' };

describe('Notes route resource loading', () => {
  it('hides the prior note immediately when the slug changes', async () => {
    const state = createNoteResourceState();
    const first = deferred<string>();
    const second = deferred<string>();

    const firstLoad = runNoteResourceLoad(state, noteA, () => first.promise);
    first.resolve('Note A');
    await firstLoad;

    const secondLoad = runNoteResourceLoad(state, noteB, () => second.promise);

    expect(state).toMatchObject({ target: noteB, status: 'pending', hasContent: false });

    second.resolve('Note B');
    await secondLoad;
  });

  it('treats a workspace-only change as a new target', async () => {
    const state = createNoteResourceState();
    const first = deferred<string>();
    const second = deferred<string>();

    const firstLoad = runNoteResourceLoad(state, noteA, () => first.promise);
    first.resolve('Alpha note');
    await firstLoad;

    const secondLoad = runNoteResourceLoad(state, noteAInBeta, () => second.promise);

    expect(state).toMatchObject({ target: noteAInBeta, status: 'pending', hasContent: false });

    second.resolve('Beta note');
    await secondLoad;
  });

  it('rejects a late response from a superseded target', async () => {
    const state = createNoteResourceState();
    const first = deferred<string>();
    const second = deferred<string>();

    const firstLoad = runNoteResourceLoad(state, noteA, () => first.promise);
    const secondLoad = runNoteResourceLoad(state, noteB, () => second.promise);

    second.resolve('Note B');
    await secondLoad;
    first.resolve('Stale note A');
    await firstLoad;

    expect(state).toMatchObject({ target: noteB, status: 'ready', hasContent: true });
  });

  it('preserves content during a same-target refresh', async () => {
    const state = createNoteResourceState();
    const first = deferred<string>();
    const refresh = deferred<string>();

    const firstLoad = runNoteResourceLoad(state, noteA, () => first.promise);
    first.resolve('Note A');
    await firstLoad;

    const refreshLoad = runNoteResourceLoad(state, noteA, () => refresh.promise);

    expect(state).toMatchObject({ target: noteA, status: 'pending', hasContent: true });

    refresh.resolve('Updated note A');
    await refreshLoad;
  });

  it('replaces a current-target load with an error state', async () => {
    const state = createNoteResourceState();
    const failingLoad = runNoteResourceLoad(state, noteA, async () => {
      throw new Error('Document unavailable');
    });

    await failingLoad;

    expect(state).toMatchObject({
      target: noteA,
      status: 'error',
      hasContent: false,
      error: 'Document unavailable',
    });
  });

  it('waits for a pending save before loading a new target', async () => {
    const state = createNoteResourceState();
    const save = deferred<void>();
    const document = deferred<string>();
    let loadStarted = false;

    const transition = flushThenLoadNoteResource(
      state,
      noteB,
      () => save.promise,
      () => {
        loadStarted = true;
        return document.promise;
      },
    );

    expect(loadStarted).toBe(false);

    save.resolve();
    await Promise.resolve();

    expect(loadStarted).toBe(true);

    document.resolve('Note B');
    await transition;
  });
});
