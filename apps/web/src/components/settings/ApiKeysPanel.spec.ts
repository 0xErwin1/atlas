import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ApiKeysPanel from '@/components/settings/ApiKeysPanel.vue';
import { type ApiKeyDto, type ApiKeyGrantDto, useApiKeysStore } from '@/stores/apiKeys';
import { useWorkspaceStore } from '@/stores/workspace';

function key(over: Partial<ApiKeyDto> = {}): ApiKeyDto {
  return {
    id: 'k1',
    name: 'ci-bot',
    type: 'agent',
    created_at: '2024-01-01T00:00:00Z',
    is_global: false,
    ...over,
  };
}

function grant(over: Partial<ApiKeyGrantDto> = {}): ApiKeyGrantDto {
  return {
    id: 'g1',
    resource_kind: 'workspace',
    resource_label: 'Acme',
    role: 'editor',
    workspace_slug: 'acme',
    ...over,
  };
}

function setup(keys: ApiKeyDto[], grants: ApiKeyGrantDto[] = []) {
  setActivePinia(createPinia());

  const ws = useWorkspaceStore();
  ws.activeWorkspaceSlug = 'acme';

  const store = useApiKeysStore();
  store.keys = keys;
  vi.spyOn(store, 'loadKeys').mockResolvedValue(undefined);
  vi.spyOn(store, 'loadKeyGrants').mockResolvedValue(grants);

  return store;
}

let activeWrapper: VueWrapper | null = null;

async function mountExpanded(): Promise<VueWrapper> {
  const wrapper = mount(ApiKeysPanel);
  activeWrapper = wrapper;
  await flushPromises();
  await wrapper.find('.atl-keys-row').trigger('click');
  await flushPromises();
  return wrapper;
}

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
});

describe('ApiKeysPanel — agent reach overview', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('shows the global pill when the key is global', async () => {
    setup([key({ is_global: true })]);

    const wrapper = await mountExpanded();

    expect(wrapper.find('.atl-global-pill').exists()).toBe(true);
    expect(wrapper.text()).toContain('Global');
  });

  it('summarizes grants grouped by role and resource kind when not global', async () => {
    setup(
      [key({ is_global: false })],
      [
        grant({ id: 'g1', role: 'editor', resource_kind: 'workspace', resource_label: 'Acme' }),
        grant({
          id: 'g2',
          role: 'editor',
          resource_kind: 'workspace',
          resource_label: 'Beta',
          workspace_slug: 'beta',
        }),
        grant({ id: 'g3', role: 'viewer', resource_kind: 'board', resource_label: 'Roadmap' }),
      ],
    );

    const wrapper = await mountExpanded();

    expect(wrapper.find('.atl-global-pill').exists()).toBe(false);
    const summary = wrapper.find('.atl-access-summary').text();
    expect(summary).toContain('Editor in 2 workspaces');
    expect(summary).toContain('Viewer in 1 board');
  });

  it('toggling the Global agent switch calls setKeyGlobal with the negated value', async () => {
    const store = setup([key({ is_global: false })]);
    const setGlobal = vi.spyOn(store, 'setKeyGlobal').mockResolvedValue(true);

    const wrapper = await mountExpanded();

    await wrapper.find('.atl-switch').trigger('click');

    expect(setGlobal).toHaveBeenCalledWith('k1', true);
  });

  it('renders granted-by when the grant carries it', async () => {
    setup(
      [key({ is_global: false })],
      [grant({ granted_by: { id: 'u1', display: 'Ada', principal_type: 'user' } })],
    );

    const wrapper = await mountExpanded();

    expect(wrapper.find('.atl-grant-by').text()).toContain('granted by Ada');
  });
});
