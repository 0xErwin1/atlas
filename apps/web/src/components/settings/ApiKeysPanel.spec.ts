import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ApiKeysPanel from '@/components/settings/ApiKeysPanel.vue';
import WorkspaceAccessEditor from '@/components/settings/WorkspaceAccessEditor.vue';
import { type ApiKeyDto, type ApiKeyGrantDto, useApiKeysStore } from '@/stores/apiKeys';
import { useWorkspaceStore, type WorkspaceDto } from '@/stores/workspace';

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

function workspace(slug: string, name: string): WorkspaceDto {
  return {
    id: `ws-${slug}`,
    slug,
    name,
    created_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-01T00:00:00Z',
  };
}

const ADMIN_WORKSPACES = [workspace('acme', 'Acme'), workspace('beta', 'Beta')];

function setup(keys: ApiKeyDto[], grants: ApiKeyGrantDto[] = []) {
  setActivePinia(createPinia());

  const ws = useWorkspaceStore();
  ws.activeWorkspaceSlug = 'acme';
  ws.adminWorkspaces = ADMIN_WORKSPACES;
  vi.spyOn(ws, 'loadAdminWorkspaces').mockResolvedValue(undefined);

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

  it('renders granted-by on a sub-resource grant when it carries it', async () => {
    setup(
      [key({ is_global: false })],
      [
        grant({
          id: 'g9',
          resource_kind: 'board',
          resource_label: 'Roadmap',
          role: 'viewer',
          granted_by: { id: 'u1', display: 'Ada', principal_type: 'user' },
        }),
      ],
    );

    const wrapper = await mountExpanded();

    expect(wrapper.find('.atl-grant-by').text()).toContain('granted by Ada');
  });
});

describe('ApiKeysPanel — manage expander and workspace-access editor', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('expands via the labeled Manage control (not an icon-only button)', async () => {
    setup([key({ is_global: false })]);

    const wrapper = mount(ApiKeysPanel);
    activeWrapper = wrapper;
    await flushPromises();

    const manage = wrapper.find('[data-action="manage"]');
    expect(manage.exists()).toBe(true);
    expect(manage.text()).toContain('Manage');

    await manage.trigger('click');
    await flushPromises();

    expect(wrapper.findComponent(WorkspaceAccessEditor).exists()).toBe(true);
  });

  it('shows the global note and no editor for a global key', async () => {
    setup([key({ is_global: true })]);

    const wrapper = await mountExpanded();

    expect(wrapper.find('[data-global-note]').exists()).toBe(true);
    expect(wrapper.text()).toContain("Per-workspace grants aren't needed");
    expect(wrapper.findComponent(WorkspaceAccessEditor).exists()).toBe(false);
  });

  it('shows the WorkspaceAccessEditor for a non-global key with the key roles', async () => {
    setup(
      [key({ is_global: false })],
      [grant({ id: 'g1', workspace_slug: 'acme', role: 'editor', resource_kind: 'workspace' })],
    );

    const wrapper = await mountExpanded();

    const editor = wrapper.findComponent(WorkspaceAccessEditor);
    expect(editor.exists()).toBe(true);
    expect(editor.props('roles')).toEqual({ acme: 'editor' });
  });

  it('never offers the Admin role for an agent', async () => {
    setup([key({ is_global: false })]);

    const wrapper = await mountExpanded();

    const optionValues = wrapper.findAll('[data-wsa-role] option').map((o) => o.attributes('value'));
    expect(optionValues).not.toContain('admin');
    expect(optionValues).toContain('viewer');
    expect(optionValues).toContain('editor');
  });

  it('assigning a role calls setKeyWorkspaceRole(keyId, slug, role)', async () => {
    const store = setup([key({ is_global: false })]);
    const setRole = vi.spyOn(store, 'setKeyWorkspaceRole').mockResolvedValue(true);

    const wrapper = await mountExpanded();

    const select = wrapper.findAll('[data-wsa-role]')[1];
    if (select === undefined) throw new Error('expected a workspace row select');

    await select.setValue('editor');

    expect(setRole).toHaveBeenCalledWith('k1', 'beta', 'editor');
  });
});
