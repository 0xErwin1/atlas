import { describe, expect, it } from 'vitest';
import { actorKey, isSelfActor, type LiveEnvelope } from '@/lib/eventTypes';

const envelope = (type: string, id: string): LiveEnvelope => ({
  id: 'evt-1',
  event_type: 'task.updated',
  version: 1,
  source: 'atlas',
  workspace_id: 'ws-1',
  occurred_at: '2026-01-01T00:00:00Z',
  actor: { type, id },
  data: { task_id: 't1', changed_fields: ['description'] },
});

describe('actorKey', () => {
  it('renders the principal as type:id', () => {
    expect(actorKey({ type: 'user', id: 'u1' })).toBe('user:u1');
    expect(actorKey({ type: 'api_key', id: 'k1' })).toBe('api_key:k1');
  });
});

describe('isSelfActor', () => {
  it('recognizes a frame produced by the signed-in user', () => {
    expect(isSelfActor(envelope('user', 'u1'), 'user:u1')).toBe(true);
  });

  it('recognizes a frame produced by the signed-in api key', () => {
    expect(isSelfActor(envelope('api_key', 'k1'), 'api_key:k1')).toBe(true);
  });

  it('does not treat another principal as self', () => {
    expect(isSelfActor(envelope('user', 'u2'), 'user:u1')).toBe(false);
  });

  it('does not match across principal types sharing an id', () => {
    expect(isSelfActor(envelope('api_key', 'u1'), 'user:u1')).toBe(false);
  });

  it('treats an unknown session as not self, so remote frames still apply', () => {
    expect(isSelfActor(envelope('user', 'u1'), null)).toBe(false);
    expect(isSelfActor(envelope('user', 'u1'), undefined)).toBe(false);
    expect(isSelfActor(envelope('user', 'u1'), '')).toBe(false);
  });
});

describe('isSelfActor with an untrusted envelope', () => {
  it('treats a frame carrying no actor as not self instead of throwing', () => {
    const malformed = { ...envelope('user', 'u1'), actor: undefined } as unknown as LiveEnvelope;

    expect(() => isSelfActor(malformed, 'user:u1')).not.toThrow();
    expect(isSelfActor(malformed, 'user:u1')).toBe(false);
  });

  it('treats a frame whose actor fields are not strings as not self', () => {
    const malformed = { ...envelope('user', 'u1'), actor: { type: 1, id: null } } as unknown as LiveEnvelope;

    expect(isSelfActor(malformed, 'user:u1')).toBe(false);
  });
});
