import { describe, expect, it } from 'vitest';
import { z } from 'zod';
import {
  AUTHORIZATION_LEASE_MS,
  buildCacheKey,
  createCacheEnvelopeSchema,
  DEFAULT_CACHE_POLICY,
} from '@/cache/resourceCache';

const workspaceId = '018f8e6d-7c15-7c72-8a41-2f5295e0c0f1';
const principal = 'user:018f8e6d-7c15-7c72-8a41-2f5295e0c0f2';

describe('resource cache contracts', () => {
  it('builds distinct canonical keys for every authorized resource scope', () => {
    const base = {
      principal,
      workspaceId,
      resourceKind: 'task-list' as const,
      resourceId: 'workspace-tasks',
      query: {
        archived: false,
        labels: ['urgent', 'bug'],
      },
      setValuedQueryKeys: ['labels'],
    };

    expect(buildCacheKey(base)).toBe(
      `v1|p=${principal}|w=${workspaceId}|k=task-list|r=workspace-tasks|q={"archived":false,"labels":["bug","urgent"]}`,
    );
    expect(buildCacheKey({ ...base, principal: 'api_key:018f8e6d-7c15-7c72-8a41-2f5295e0c0f3' })).not.toBe(
      buildCacheKey(base),
    );
    expect(buildCacheKey({ ...base, workspaceId: '018f8e6d-7c15-7c72-8a41-2f5295e0c0f4' })).not.toBe(
      buildCacheKey(base),
    );
    expect(buildCacheKey({ ...base, resourceKind: 'task-detail', resourceId: 'workspace-tasks' })).not.toBe(
      buildCacheKey(base),
    );
    expect(buildCacheKey({ ...base, resourceId: 'another-resource' })).not.toBe(buildCacheKey(base));
    expect(buildCacheKey({ ...base, query: { archived: true, labels: ['bug', 'urgent'] } })).not.toBe(
      buildCacheKey(base),
    );
  });

  it('fails closed for noncanonical identities', () => {
    for (const invalidIdentity of [
      { principal: '', workspaceId },
      { principal: ` ${principal}`, workspaceId },
      { principal: 'user:not-a-uuid', workspaceId },
      { principal, workspaceId: ` ${workspaceId}` },
      { principal, workspaceId: workspaceId.toUpperCase() },
    ]) {
      expect(
        buildCacheKey({
          ...invalidIdentity,
          resourceKind: 'note-body',
          resourceId: 'note-a',
        }),
      ).toBeNull();
    }

    expect(AUTHORIZATION_LEASE_MS).toBe(24 * 60 * 60 * 1000);
    expect(DEFAULT_CACHE_POLICY.persistent.maxBytes).toBe(50 * 1024 * 1024);
  });

  it('rejects credential-bearing payloads at the cache envelope boundary', () => {
    const schema = createCacheEnvelopeSchema(z.object({ title: z.string() }).passthrough());

    expect(
      schema.safeParse({
        schema: 1,
        key: buildCacheKey({ principal, workspaceId, resourceKind: 'note-body', resourceId: 'note-a' }),
        payloadVersion: 1,
        storedAt: 1,
        validatedAt: 1,
        lastAccessedAt: 1,
        retentionExpiresAt: 2,
        bytes: 24,
        stale: false,
        tags: ['note:note-a'],
        payload: { title: 1 },
      }).success,
    ).toBe(false);
    expect(
      schema.safeParse({
        schema: 1,
        key: buildCacheKey({ principal, workspaceId, resourceKind: 'note-body', resourceId: 'note-a' }),
        payloadVersion: 1,
        storedAt: 1,
        validatedAt: 1,
        lastAccessedAt: 1,
        retentionExpiresAt: 2,
        bytes: 24,
        stale: false,
        tags: ['note:note-a'],
        payload: { title: 'note', authorization: 'Bearer secret' },
      }).success,
    ).toBe(false);
    expect(
      schema.safeParse({
        schema: 1,
        key: buildCacheKey({ principal, workspaceId, resourceKind: 'note-body', resourceId: 'note-a' }),
        payloadVersion: 1,
        storedAt: 1,
        validatedAt: 1,
        lastAccessedAt: 1,
        retentionExpiresAt: 2,
        bytes: 24,
        stale: false,
        tags: ['note:note-a'],
        payload: { title: 'note', attachmentBytes: new Uint8Array([1, 2, 3]) },
      }).success,
    ).toBe(false);
  });
});
