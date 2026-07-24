import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { getOrigin, login, replace, setOrigin } = vi.hoisted(() => ({
  getOrigin: vi.fn(),
  login: vi.fn(),
  replace: vi.fn(),
  setOrigin: vi.fn(),
}));

vi.mock('vue-router', () => ({
  useRouter: () => ({ currentRoute: { value: { query: {} } }, replace }),
}));

vi.mock('@/stores/auth', () => ({
  useAuthStore: () => ({ login }),
}));

vi.mock('@/platform/transport', () => ({
  getPlatformTransport: () => ({
    isDesktop: true,
    getOrigin,
    setOrigin,
  }),
}));

import Login from '@/views/Login.vue';
import loginSource from '@/views/Login.vue?raw';

describe('desktop login server selection', () => {
  beforeEach(() => {
    getOrigin.mockResolvedValue({ data: { origin: 'https://atlas.iperez.dev' } });
    setOrigin.mockResolvedValue({ data: { origin: 'https://custom.example:8443' } });
    login.mockResolvedValue({ ok: true });
  });

  it('prefills the persisted origin and saves a normalized custom origin before login', async () => {
    const wrapper = mount(Login);
    await flushPromises();

    const origin = wrapper.get('#server-origin');
    expect((origin.element as HTMLInputElement).value).toBe('https://atlas.iperez.dev');

    await origin.setValue('https://custom.example:8443/');
    await wrapper.get('#username').setValue('maintainer');
    await wrapper.get('#password').setValue('secret');
    await wrapper.get('form').trigger('submit');
    await flushPromises();

    expect(setOrigin).toHaveBeenCalledWith('https://custom.example:8443/');
    expect(setOrigin.mock.invocationCallOrder[0]).toBeLessThan(login.mock.invocationCallOrder[0] ?? 0);
    expect(login).toHaveBeenCalledWith({ username: 'maintainer', password: 'secret' });
    expect(replace).toHaveBeenCalledWith('/n');
  });

  it('keeps the sign-in column at a usable width in the desktop window', () => {
    const columnRule = loginSource.match(/\.login-col\s*\{([^}]*)\}/)?.[1];

    expect(columnRule).toMatch(/\bwidth:\s*100%\s*;/);
    expect(columnRule).toMatch(/\bmax-width:\s*380px\s*;/);
  });
});
