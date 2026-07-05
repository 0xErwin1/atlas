import { type DOMWrapper, flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import ApiKeysPanel from '@/components/settings/ApiKeysPanel.vue';
import WorkspaceAccessEditor from '@/components/settings/WorkspaceAccessEditor.vue';
import { type ApiKeyCreated, type ApiKeyDto, type ApiKeyGrantDto, useApiKeysStore } from '@/stores/apiKeys';
import { useWorkspaceStore, type WorkspaceDto } from '@/stores/workspace';

function key(over: Partial<ApiKeyDto> = {}): ApiKeyDto {
  return {
    id: 'k1',
    name: 'ci-bot',
    type: 'agent',
    created_at: '2024-01-01T00:00:00Z',
    is_global: false,
    scopes: [],
    ...over,
  } as ApiKeyDto;
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

// Each workspace row's role control is a Dropdown whose listbox teleports to
// <body>: open its trigger, then click the teleported option matching the label.
async function pickRole(row: DOMWrapper<Element>, label: string): Promise<void> {
  await row.find('button').trigger('click');
  await nextTick();

  const option = Array.from(document.body.querySelectorAll<HTMLElement>('li[role="option"]')).find(
    (li) => li.textContent?.trim() === label,
  );
  if (option === undefined) throw new Error(`option not found: ${label}`);

  option.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await nextTick();
}

async function roleOptionLabels(row: DOMWrapper<Element>): Promise<string[]> {
  await row.find('button').trigger('click');
  await nextTick();
  return Array.from(document.body.querySelectorAll<HTMLElement>('li[role="option"]')).map(
    (li) => li.textContent?.trim() ?? '',
  );
}

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
  const wrapper = mount(ApiKeysPanel, { attachTo: document.body });
  activeWrapper = wrapper;
  await flushPromises();
  await wrapper.find('[data-row]').trigger('click');
  await flushPromises();
  return wrapper;
}

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  document.body.innerHTML = '';
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

    const row = wrapper.findAll('[data-wsa-row]')[0];
    if (row === undefined) throw new Error('expected a workspace row');

    const labels = await roleOptionLabels(row);
    expect(labels).not.toContain('Admin');
    expect(labels).toContain('Viewer');
    expect(labels).toContain('Editor');
  });

  it('assigning a role calls setKeyWorkspaceRole(keyId, slug, role)', async () => {
    const store = setup([key({ is_global: false })]);
    const setRole = vi.spyOn(store, 'setKeyWorkspaceRole').mockResolvedValue(true);

    const wrapper = await mountExpanded();

    const row = wrapper.findAll('[data-wsa-row]')[1];
    if (row === undefined) throw new Error('expected a workspace row');

    await pickRole(row, 'Editor');

    expect(setRole).toHaveBeenCalledWith('k1', 'beta', 'editor');
  });
});

describe('ApiKeysPanel — capability scope grid', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('builds scopes from the create-form grid and passes them to createKey', async () => {
    const store = setup([]);
    const create = vi.spyOn(store, 'createKey').mockResolvedValue({
      id: 'k9',
      name: 'ci-bot',
      type: 'agent',
      created_at: '2024-01-01T00:00:00Z',
      is_global: false,
      scopes: [],
      secret: 'sk_test',
    } as ApiKeyCreated);

    const wrapper = mount(ApiKeysPanel, { attachTo: document.body });
    activeWrapper = wrapper;
    await flushPromises();

    const newBtn = wrapper.findAll('button').find((b) => b.text().includes('New key'));
    if (newBtn === undefined) throw new Error('expected a New key button');
    await newBtn.trigger('click');
    await nextTick();

    await wrapper.find('input[placeholder="ci-deploy"]').setValue('ci-bot');

    // Toggle out of canonical order to prove the grid emits a sorted set.
    await wrapper.find('[data-scope="projects:delete"]').setValue(true);
    await wrapper.find('[data-scope="tasks:read"]').setValue(true);

    const createBtn = wrapper.findAll('button').find((b) => b.text().includes('Create key'));
    if (createBtn === undefined) throw new Error('expected a Create key button');
    await createBtn.trigger('click');
    await flushPromises();

    expect(create).toHaveBeenCalledWith(
      expect.objectContaining({ scopes: ['tasks:read', 'projects:delete'] }),
    );
  });

  it('pre-populates the edit grid from the key scopes and saves the full replacement set', async () => {
    const store = setup([key({ scopes: ['tasks:read', 'docs:update'] })]);
    const save = vi.spyOn(store, 'setKeyScopes').mockResolvedValue(true);

    const wrapper = await mountExpanded();

    expect((wrapper.find('[data-scope="tasks:read"]').element as HTMLInputElement).checked).toBe(true);
    expect((wrapper.find('[data-scope="docs:update"]').element as HTMLInputElement).checked).toBe(true);
    expect((wrapper.find('[data-scope="boards:create"]').element as HTMLInputElement).checked).toBe(false);

    await wrapper.find('[data-scope="boards:create"]').setValue(true);
    await wrapper.find('[data-action="save-scopes"]').trigger('click');
    await flushPromises();

    expect(save).toHaveBeenCalledWith('k1', ['tasks:read', 'docs:update', 'boards:create']);
  });
});
