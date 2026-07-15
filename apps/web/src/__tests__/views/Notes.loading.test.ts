import { describe, expect, it } from 'vitest';
import {
  canApplyCachedDocument,
  createNoteResourceState,
  flushThenLoadNoteResource,
  hydrateNoteResource,
  type NoteTarget,
  retractNoteResourceForDeniedLoad,
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
  it('rejects a same-target cached document while the editor is dirty or the load is superseded', () => {
    const state = createNoteResourceState();
    state.target = noteA;
    state.sequence = 4;

    expect(canApplyCachedDocument(state, noteA, 4, true)).toBe(false);
    expect(canApplyCachedDocument(state, noteA, 3, false)).toBe(false);
    expect(canApplyCachedDocument(state, noteA, 4, false)).toBe(true);
  });

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

  it('accepts an exact cached target without reviving the former target', async () => {
    const state = createNoteResourceState();
    const first = deferred<string>();

    const firstLoad = runNoteResourceLoad(state, noteA, () => first.promise);
    first.resolve('Note A');
    await firstLoad;
    const nextLoad = runNoteResourceLoad(state, noteB, async () => 'Fresh note B');

    expect(hydrateNoteResource(state, noteB)).toBe(true);
    expect(state).toMatchObject({ target: noteB, status: 'pending', hasContent: true });
    expect(hydrateNoteResource(state, noteA)).toBe(false);

    await nextLoad;
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

  it('retracts cached content synchronously for forbidden and missing note loads', async () => {
    const state = createNoteResourceState();
    await runNoteResourceLoad(state, noteA, async () => 'Cached note A');

    retractNoteResourceForDeniedLoad(state, noteA, new Error('Forbidden'));

    expect(state).toMatchObject({
      target: noteA,
      status: 'error',
      hasContent: false,
      error: 'Forbidden',
    });

    await runNoteResourceLoad(state, noteB, async () => 'Cached note B');
    retractNoteResourceForDeniedLoad(state, noteB, new Error('Not Found'));

    expect(state).toMatchObject({
      target: noteB,
      status: 'error',
      hasContent: false,
      error: 'Not Found',
    });
  });

  it('waits for a pending save before loading a new target', async () => {
    const state = createNoteResourceState();
    const initialDocument = deferred<string>();
    const save = deferred<void>();
    const document = deferred<string>();
    let loadStarted = false;

    const initialLoad = runNoteResourceLoad(state, noteA, () => initialDocument.promise);
    initialDocument.resolve('Note A');
    await initialLoad;

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
    expect(state).toMatchObject({ target: noteB, status: 'pending', hasContent: false });

    save.resolve();
    await Promise.resolve();

    expect(loadStarted).toBe(true);

    document.resolve('Note B');
    await transition;
  });

  it('replaces a current transition with an error when its pending save fails', async () => {
    const state = createNoteResourceState();
    const save = deferred<void>();
    let loadStarted = false;

    const transition = flushThenLoadNoteResource(
      state,
      noteB,
      () => save.promise,
      async () => {
        loadStarted = true;
        return 'Note B';
      },
    );

    save.reject(new Error('Save failed'));
    const result = await transition;

    expect(loadStarted).toBe(false);
    expect(result).toMatchObject({ accepted: false, error: new Error('Save failed') });
    expect(state).toMatchObject({
      target: noteB,
      status: 'error',
      hasContent: false,
      error: 'Save failed',
    });
  });

  it('does not let a rejected obsolete save overwrite the latest transition state', async () => {
    const state = createNoteResourceState();
    const saveForB = deferred<void>();
    const saveForC = deferred<void>();
    const documentC = deferred<string>();

    const transitionToB = flushThenLoadNoteResource(
      state,
      noteB,
      () => saveForB.promise,
      async () => 'Note B',
    );
    const transitionToC = flushThenLoadNoteResource(
      state,
      noteAInBeta,
      () => saveForC.promise,
      () => documentC.promise,
    );

    saveForB.reject(new Error('Obsolete save failed'));
    await transitionToB;

    expect(state).toMatchObject({ target: noteAInBeta, status: 'pending', hasContent: false, error: null });

    saveForC.resolve();
    await Promise.resolve();
    documentC.resolve('Note C');
    await transitionToC;

    expect(state).toMatchObject({ target: noteAInBeta, status: 'ready', hasContent: true, error: null });
  });

  it('rejects an outgoing response that settles while its save is pending', async () => {
    const state = createNoteResourceState();
    const outgoingDocument = deferred<string>();
    const save = deferred<void>();
    const document = deferred<string>();

    const outgoingLoad = runNoteResourceLoad(state, noteA, () => outgoingDocument.promise);
    const transition = flushThenLoadNoteResource(
      state,
      noteB,
      () => save.promise,
      () => document.promise,
    );

    outgoingDocument.resolve('Late note A');
    await outgoingLoad;

    expect(state).toMatchObject({ target: noteB, status: 'pending', hasContent: false });

    save.resolve();
    await Promise.resolve();
    document.resolve('Note B');
    await transition;

    expect(state).toMatchObject({ target: noteB, status: 'ready', hasContent: true });
  });

  it('does not start a superseded transition after its pending save settles', async () => {
    const state = createNoteResourceState();
    const saveForB = deferred<void>();
    const saveForC = deferred<void>();
    const documentB = deferred<string>();
    const documentC = deferred<string>();
    let bLoadStarted = false;
    let cLoadStarted = false;

    const transitionToB = flushThenLoadNoteResource(
      state,
      noteB,
      () => saveForB.promise,
      () => {
        bLoadStarted = true;
        return documentB.promise;
      },
    );
    const transitionToC = flushThenLoadNoteResource(
      state,
      noteAInBeta,
      () => saveForC.promise,
      () => {
        cLoadStarted = true;
        return documentC.promise;
      },
    );

    expect(state).toMatchObject({ target: noteAInBeta, status: 'pending', hasContent: false });

    saveForB.resolve();
    await Promise.resolve();

    expect(bLoadStarted).toBe(false);
    expect(state).toMatchObject({ target: noteAInBeta, status: 'pending', hasContent: false });

    saveForC.resolve();
    await Promise.resolve();

    expect(cLoadStarted).toBe(true);
    documentC.resolve('Note C');
    await transitionToC;
    await transitionToB;

    expect(state).toMatchObject({ target: noteAInBeta, status: 'ready', hasContent: true });
  });
});
