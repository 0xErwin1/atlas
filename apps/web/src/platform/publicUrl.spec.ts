import { afterEach, describe, expect, it, vi } from 'vitest';
import { fakePlatformTransport } from '../__tests__/helpers/platformTransport';
import { openPublicPath, toPublicUrl } from './publicUrl';
import { resetPlatformTransportForTest, setPlatformTransport } from './transport';

describe('toPublicUrl', () => {
  afterEach(() => {
    resetPlatformTransportForTest();
  });

  it('returns an absolute http(s) path unchanged', () => {
    setPlatformTransport(fakePlatformTransport());

    expect(toPublicUrl('https://elsewhere.example/x')).toBe('https://elsewhere.example/x');
    expect(toPublicUrl('http://elsewhere.example/x')).toBe('http://elsewhere.example/x');
  });

  it('returns null when the base is empty', () => {
    setPlatformTransport(fakePlatformTransport({ publicBase: () => '' }));

    expect(toPublicUrl('/t/task/AB-1')).toBeNull();
  });

  it('joins a path that already starts with a slash', () => {
    setPlatformTransport(fakePlatformTransport());

    expect(toPublicUrl('/t/task/AB-1')).toBe('https://atlas.test/t/task/AB-1');
  });

  it('joins a path that does not start with a slash', () => {
    setPlatformTransport(fakePlatformTransport());

    expect(toPublicUrl('t/task/AB-1')).toBe('https://atlas.test/t/task/AB-1');
  });
});

describe('openPublicPath', () => {
  afterEach(() => {
    resetPlatformTransportForTest();
  });

  it('returns unknown-base without calling openExternal when the base is empty', async () => {
    const openExternal = vi.fn(async () => ({}));
    setPlatformTransport(fakePlatformTransport({ publicBase: () => '', openExternal }));

    await expect(openPublicPath('/t/task/AB-1')).resolves.toBe('unknown-base');
    expect(openExternal).not.toHaveBeenCalled();
  });

  it('returns failed when openExternal resolves with an error', async () => {
    setPlatformTransport(fakePlatformTransport({ openExternal: vi.fn(async () => ({ error: 'x' })) }));

    await expect(openPublicPath('/t/task/AB-1')).resolves.toBe('failed');
  });

  it('returns opened when openExternal resolves with an empty result', async () => {
    setPlatformTransport(fakePlatformTransport({ openExternal: vi.fn(async () => ({})) }));

    await expect(openPublicPath('/t/task/AB-1')).resolves.toBe('opened');
  });

  it('returns opened when openExternal resolves with error: null', async () => {
    setPlatformTransport(fakePlatformTransport({ openExternal: vi.fn(async () => ({ error: null })) }));

    await expect(openPublicPath('/t/task/AB-1')).resolves.toBe('opened');
  });

  it('returns failed instead of rejecting when openExternal throws', async () => {
    setPlatformTransport(
      fakePlatformTransport({ openExternal: vi.fn(() => Promise.reject(new Error('bridge failure'))) }),
    );

    await expect(openPublicPath('/t/task/AB-1')).resolves.toBe('failed');
  });
});
