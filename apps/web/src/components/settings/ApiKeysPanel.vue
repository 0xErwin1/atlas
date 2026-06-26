<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { z } from 'zod';
import ShareDialog from '@/components/share/ShareDialog.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Btn from '@/components/ui/Btn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import { validateForm } from '@/lib/validation';
import { type ApiKeyCreated, type ApiKeyDto, type ApiKeyGrantDto, useApiKeysStore } from '@/stores/apiKeys';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const keysStore = useApiKeysStore();
const ui = useUiStore();
const wsStore = useWorkspaceStore();

type Mode = 'list' | 'new' | 'secret';
const mode = ref<Mode>('list');

type KeyType = 'agent' | 'cli' | 'bot' | 'integration';
const KEY_TYPES: { value: KeyType; label: string }[] = [
  { value: 'agent', label: 'Agent' },
  { value: 'cli', label: 'CLI' },
  { value: 'bot', label: 'Bot' },
  { value: 'integration', label: 'Integration' },
];

const form = reactive({ name: '', type: 'agent' as KeyType, expires: '' });
const formErrors = reactive<{ name: string | null }>({ name: null });
const saving = ref(false);

const created = ref<ApiKeyCreated | null>(null);
const copied = ref(false);

const revokeTarget = ref<{ id: string; name: string } | null>(null);

const revokeDetail = computed(() => {
  const target = revokeTarget.value;
  if (target === null) return undefined;
  return `${target.name} · ${target.id.slice(0, 8)}...`;
});

const nameSchema = z.object({ name: z.string().trim().min(1, 'Name is required') });

function fmtDate(iso: string | null | undefined): string {
  if (!iso) return 'Never';
  const d = new Date(iso);
  return d.toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: '2-digit' });
}

function typeLabel(t: string): string {
  return KEY_TYPES.find((k) => k.value === t)?.label ?? t.toUpperCase();
}

onMounted(() => {
  keysStore.loadKeys();
});

function startNew(): void {
  form.name = '';
  form.type = 'agent';
  form.expires = '';
  formErrors.name = null;
  mode.value = 'new';
}

function cancelNew(): void {
  mode.value = 'list';
}

async function submitNew(): Promise<void> {
  formErrors.name = null;

  const result = validateForm(nameSchema, { name: form.name });
  if (!result.ok) {
    formErrors.name = result.errors.name ?? null;
    return;
  }

  const expiresAt = form.expires === '' ? null : new Date(form.expires).toISOString();

  saving.value = true;
  const res = await keysStore.createKey({
    name: result.data.name,
    type: form.type,
    expires_at: expiresAt,
    initial_grant: null,
  });
  saving.value = false;

  if (res === null) {
    if (keysStore.error) ui.showBanner(keysStore.error, 'error');
    return;
  }

  created.value = res;
  copied.value = false;
  mode.value = 'secret';
}

async function copySecret(): Promise<void> {
  if (created.value === null) return;
  try {
    await navigator.clipboard.writeText(created.value.secret);
    copied.value = true;
  } catch {
    ui.showBanner('Clipboard is not available', 'error');
  }
}

async function doneSecret(): Promise<void> {
  created.value = null;
  mode.value = 'list';
  await keysStore.loadKeys();
}

async function confirmRevoke(): Promise<void> {
  const target = revokeTarget.value;
  revokeTarget.value = null;
  if (target === null) return;

  const ok = await keysStore.revokeKey(target.id);
  if (ok) ui.showBanner('API key revoked', 'success');
  else if (keysStore.error) ui.showBanner(keysStore.error, 'error');
}

// ---------------------------------------------------------------------------
// Per-key grants section
// ---------------------------------------------------------------------------

const expandedKeyId = ref<string | null>(null);
const keyGrants = ref<Record<string, ApiKeyGrantDto[]>>({});
const grantsLoading = ref<Record<string, boolean>>({});

const shareDialogOpen = ref(false);
const shareDialogKey = ref<{ id: string; ws: string } | null>(null);

function toggleExpand(keyId: string): void {
  if (expandedKeyId.value === keyId) {
    expandedKeyId.value = null;
    return;
  }

  expandedKeyId.value = keyId;

  if (keyGrants.value[keyId] === undefined) {
    void loadGrants(keyId);
  }
}

async function loadGrants(keyId: string): Promise<void> {
  grantsLoading.value = { ...grantsLoading.value, [keyId]: true };

  const grants = await keysStore.loadKeyGrants(keyId);

  grantsLoading.value = { ...grantsLoading.value, [keyId]: false };

  if (grants !== null) {
    keyGrants.value = { ...keyGrants.value, [keyId]: grants };
  }
}

async function revokeGrant(keyId: string, grantId: string): Promise<void> {
  const ok = await keysStore.revokeKeyGrant(keyId, grantId);

  if (!ok) {
    if (keysStore.error) ui.showBanner(keysStore.error, 'error');
    return;
  }

  await loadGrants(keyId);
}

function openGrantDialog(keyId: string, wsSlug: string): void {
  shareDialogKey.value = { id: keyId, ws: wsSlug };
  shareDialogOpen.value = true;
}

function closeGrantDialog(): void {
  shareDialogOpen.value = false;

  if (shareDialogKey.value !== null) {
    void loadGrants(shareDialogKey.value.id);
    shareDialogKey.value = null;
  }
}

function grantsFor(keyId: string): ApiKeyGrantDto[] {
  return keyGrants.value[keyId] ?? [];
}

function resourceIcon(kind: string): string {
  const icons: Record<string, string> = {
    workspace: 'building-2',
    project: 'folder',
    folder: 'folder',
    document: 'file-text',
    board: 'layout-dashboard',
  };
  return icons[kind] ?? 'circle';
}

function grantLabel(g: ApiKeyGrantDto): string {
  if (g.resource_kind === 'workspace') return g.resource_label;
  if (g.resource_kind === 'project') return `${g.workspace_slug} / ${g.resource_label}`;
  return `${g.workspace_slug} / ${g.resource_label}`;
}

function defaultWsForKey(keyId: string): string {
  const grants = grantsFor(keyId);
  const first = grants[0];
  if (first !== undefined) return first.workspace_slug;
  return wsStore.activeWorkspaceSlug ?? '';
}

// ---------------------------------------------------------------------------
// Reach overview, global toggle and copy-id
// ---------------------------------------------------------------------------

const globalPending = ref<Record<string, boolean>>({});
const copiedKeyId = ref<string | null>(null);

function pluralKind(kind: string, count: number): string {
  return count === 1 ? kind : `${kind}s`;
}

/**
 * Terse, human-readable summary of a non-global key's reach, grouped by role and
 * resource kind, e.g. "Editor in 2 workspaces · Viewer in 1 board".
 */
function accessSummary(keyId: string): string {
  const grants = grantsFor(keyId);
  if (grants.length === 0) return 'No workspace access yet';

  const groups = new Map<string, { role: string; kind: string; count: number }>();

  for (const g of grants) {
    const groupKey = `${g.role}|${g.resource_kind}`;
    const existing = groups.get(groupKey);
    if (existing !== undefined) existing.count += 1;
    else groups.set(groupKey, { role: g.role, kind: g.resource_kind, count: 1 });
  }

  const segments: string[] = [];
  for (const { role, kind, count } of groups.values()) {
    const roleLabel = role.charAt(0).toUpperCase() + role.slice(1);
    segments.push(`${roleLabel} in ${count} ${pluralKind(kind, count)}`);
  }

  return segments.join(' · ');
}

async function onToggleGlobal(key: ApiKeyDto): Promise<void> {
  if (globalPending.value[key.id] === true) return;

  globalPending.value = { ...globalPending.value, [key.id]: true };

  const ok = await keysStore.setKeyGlobal(key.id, !key.is_global);

  globalPending.value = { ...globalPending.value, [key.id]: false };

  if (!ok && keysStore.error) ui.showBanner(keysStore.error, 'error');
}

async function copyKeyId(id: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(id);
    copiedKeyId.value = id;
    setTimeout(() => {
      if (copiedKeyId.value === id) copiedKeyId.value = null;
    }, 1500);
  } catch {
    ui.showBanner('Clipboard is not available', 'error');
  }
}

function grantedByLabel(g: ApiKeyGrantDto): string | null {
  if (g.granted_by === undefined || g.granted_by === null) return null;
  return g.granted_by.display;
}
</script>

<template>
  <div>
    <!-- Secret shown exactly once -->
    <div v-if="mode === 'secret' && created">
      <div class="atl-panel-head">
        <div class="atl-panel-title">API keys</div>
        <div class="atl-panel-sub">Let agents and scripts act on your behalf</div>
      </div>

      <div class="atl-secret-box">
        <div class="atl-secret-warn">
          <Icon name="triangle-alert" :size="14" style="flex: 0 0 auto;" />
          Copy this now — you won't be able to see it again.
        </div>
        <div style="padding: 14px; background: var(--c-raised);">
          <div style="font-size: 12px; color: var(--c-muted); margin-bottom: 8px;">
            Secret for key
            <span style="font-family: var(--font-mono); color: var(--c-foreground);">"{{ created.name }}"</span>
          </div>
          <div class="flex items-center" style="gap: 8px;">
            <div class="atl-secret-value">{{ created.secret }}</div>
            <button type="button" class="atl-copybtn" @click="copySecret">
              <Icon :name="copied ? 'check' : 'copy'" :size="14" />{{ copied ? 'Copied' : 'Copy' }}
            </button>
          </div>
        </div>
      </div>

      <div class="flex" style="justify-content: flex-end; margin-top: 16px;">
        <Btn variant="secondary" @click="doneSecret">Done</Btn>
      </div>
    </div>

    <!-- New key form -->
    <div v-else-if="mode === 'new'">
      <div class="atl-panel-head">
        <div class="atl-panel-title">New API key</div>
        <div class="atl-panel-sub">Provisions an identity capped at editor access.</div>
      </div>

      <div class="flex flex-col" style="gap: 14px; max-width: 430px;">
        <FormField
          label="Name"
          :model-value="form.name"
          placeholder="ci-deploy"
          mono
          :error="formErrors.name"
          @update:model-value="(v) => { form.name = v; formErrors.name = null; }"
        />

        <div class="atl-field">
          <label class="atl-field-label">Type</label>
          <div class="atl-select-box">
            <select
              v-model="form.type"
              class="atl-select-input"
            >
              <option v-for="t in KEY_TYPES" :key="t.value" :value="t.value">{{ t.label }}</option>
            </select>
          </div>
        </div>

        <FormField
          label="Expires (optional)"
          type="date"
          :model-value="form.expires"
          helper="Leave empty for a key that never expires."
          @update:model-value="(v) => { form.expires = v; }"
        />
      </div>

      <div class="flex" style="gap: 8px; margin-top: 20px;">
        <Btn variant="primary" :disabled="saving" @click="submitNew">Create key</Btn>
        <Btn variant="secondary" @click="cancelNew">Cancel</Btn>
      </div>
    </div>

    <!-- List / empty -->
    <div v-else>
      <div class="atl-panel-head atl-panel-head-row">
        <div>
          <div class="atl-panel-title">API keys</div>
          <div class="atl-panel-sub">Let agents and scripts act on your behalf</div>
        </div>
        <Btn variant="primary" @click="startNew">
          <Icon name="plus" :size="14" />New key
        </Btn>
      </div>

      <div v-if="keysStore.loading" style="font-size: 13px; color: var(--c-muted); padding: 8px;">
        Loading&hellip;
      </div>

      <div v-else-if="keysStore.keys.length === 0" class="atl-keys-empty">
        <div class="atl-keys-empty-icon"><Icon name="key" :size="22" /></div>
        <div style="font-size: 14px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
          No API keys yet
        </div>
        <div style="font-size: 12.5px; color: var(--c-muted); margin-top: 5px; max-width: 300px; line-height: 1.5;">
          Create a key to let an agent or script act on your behalf. Keys are capped at editor.
        </div>
        <div style="margin-top: 16px;">
          <Btn variant="primary" @click="startNew"><Icon name="plus" :size="14" />New key</Btn>
        </div>
      </div>

      <div v-else class="atl-keys-table">
        <div class="atl-keys-head">
          <div style="flex: 0 0 26px;"></div>
          <div style="flex: 2;">Name</div>
          <div style="flex: 1;">Type</div>
          <div style="flex: 1.4;">Created</div>
          <div style="flex: 1.4;">Last used</div>
          <div style="flex: 0 0 100px;"></div>
        </div>

        <template v-for="k in keysStore.keys" :key="k.id">
          <div
            class="atl-keys-row"
            :class="{ 'atl-keys-row--expanded': expandedKeyId === k.id }"
            style="cursor: pointer;"
            @click="toggleExpand(k.id)"
          >
            <div style="flex: 0 0 26px;">
              <Icon name="key" :size="15" style="color: var(--c-chart-5);" />
            </div>
            <div class="atl-key-name">{{ k.name }}</div>
            <div style="flex: 1;">
              <AgentBadge :label="typeLabel(k.type).toUpperCase()" />
            </div>
            <div class="atl-key-meta">{{ fmtDate(k.created_at) }}</div>
            <div class="atl-key-meta" :style="{ color: k.last_used_at ? 'var(--c-foreground)' : 'var(--c-muted)' }">
              {{ fmtDate(k.last_used_at) }}
            </div>
            <div style="flex: 0 0 100px; display: flex; justify-content: flex-end; gap: 6px;" @click.stop>
              <button
                type="button"
                class="atl-icon-btn"
                :title="expandedKeyId === k.id ? 'Collapse' : 'Manage access'"
                @click="toggleExpand(k.id)"
              >
                <Icon :name="expandedKeyId === k.id ? 'chevron-up' : 'shield'" :size="14" />
              </button>
              <button
                type="button"
                class="atl-revoke"
                @click="revokeTarget = { id: k.id, name: k.name }"
              >
                Revoke
              </button>
            </div>
          </div>

          <!-- Grants section (inline expand) -->
          <div v-if="expandedKeyId === k.id" class="atl-grants-panel">
            <div class="atl-key-overview">
              <div class="atl-key-id-line">
                <span class="atl-key-id-label">Key ID</span>
                <code class="atl-key-id">{{ k.id }}</code>
                <button
                  type="button"
                  class="atl-copyid"
                  :title="copiedKeyId === k.id ? 'Copied' : 'Copy ID'"
                  @click="copyKeyId(k.id)"
                >
                  <Icon :name="copiedKeyId === k.id ? 'check' : 'copy'" :size="12" />
                  {{ copiedKeyId === k.id ? 'Copied' : 'Copy ID' }}
                </button>
              </div>

              <div class="atl-access-line">
                <span class="atl-access-term">Reach</span>
                <span v-if="k.is_global" class="atl-global-pill">
                  <Icon name="globe" :size="12" />
                  Global — every workspace you can reach (editor)
                </span>
                <span v-else-if="!grantsLoading[k.id]" class="atl-access-summary">
                  {{ accessSummary(k.id) }}
                </span>
              </div>

              <div class="atl-global-toggle">
                <button
                  type="button"
                  role="switch"
                  class="atl-switch"
                  :class="{ 'atl-switch--on': k.is_global }"
                  :aria-checked="k.is_global"
                  :disabled="globalPending[k.id] === true"
                  aria-label="Global agent"
                  @click="onToggleGlobal(k)"
                >
                  <span class="atl-switch-knob" />
                </button>
                <div class="atl-global-copy">
                  <div class="atl-global-label">Global agent</div>
                  <div class="atl-global-help">
                    Reaches every workspace you can reach, capped at editor. The agent never
                    exceeds your own permissions.
                  </div>
                </div>
              </div>
            </div>

            <div v-if="grantsLoading[k.id]" class="atl-grants-loading">
              Loading access&hellip;
            </div>

            <div v-else>
              <div v-if="grantsFor(k.id).length === 0" class="atl-grants-empty">
                <Icon name="lock" :size="13" style="color: var(--c-muted);" />
                <span>No access granted. Use "Grant access" to add this key to a workspace or project.</span>
              </div>

              <div v-else class="atl-grants-list">
                <div
                  v-for="g in grantsFor(k.id)"
                  :key="g.id"
                  class="atl-grant-row"
                >
                  <Icon :name="resourceIcon(g.resource_kind)" :size="13" style="color: var(--c-muted); flex: 0 0 auto;" />
                  <div class="atl-grant-main">
                    <span class="atl-grant-label">{{ grantLabel(g) }}</span>
                    <span v-if="grantedByLabel(g) !== null" class="atl-grant-by">
                      granted by {{ grantedByLabel(g) }}
                      <AgentBadge
                        v-if="g.granted_by?.principal_type === 'api_key'"
                        label="AGENT"
                      />
                    </span>
                  </div>
                  <span class="atl-grant-role">{{ g.role }}</span>
                  <button
                    type="button"
                    class="atl-grant-revoke"
                    title="Revoke this grant"
                    @click="revokeGrant(k.id, g.id)"
                  >
                    <Icon name="x" :size="12" />
                  </button>
                </div>
              </div>

              <div class="atl-grants-footer">
                <button
                  type="button"
                  class="atl-grant-add"
                  :disabled="defaultWsForKey(k.id) === ''"
                  @click="openGrantDialog(k.id, defaultWsForKey(k.id))"
                >
                  <Icon name="plus" :size="13" />
                  Grant access
                </button>
              </div>
            </div>
          </div>
        </template>
      </div>

      <div class="atl-keys-helper">
        <Icon name="sparkles" :size="13" style="color: var(--c-chart-5);" />
        <span>Keys are account-level and workspace-independent — grant them access per workspace.</span>
      </div>
    </div>

    <ConfirmDialog
      :open="revokeTarget !== null"
      tone="danger"
      title="Revoke this API key?"
      message="The key stops working immediately. Any agent or script still using it loses access at once."
      :detail="revokeDetail"
      detail-icon="key"
      note="This can't be undone — you'll need to issue a new key and update anything that relied on it."
      confirm-label="Revoke key"
      confirm-icon="trash-2"
      @confirm="confirmRevoke"
      @cancel="revokeTarget = null"
    />

    <ShareDialog
      v-if="shareDialogOpen && shareDialogKey !== null"
      :open="shareDialogOpen"
      :ws="shareDialogKey.ws"
      @close="closeGrantDialog"
    />
  </div>
</template>

<style scoped>
.atl-panel-head {
  margin-bottom: 16px;
}

.atl-panel-head-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
}

.atl-panel-title {
  font-size: 15px;
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
}

.atl-panel-sub {
  font-size: 12px;
  color: var(--c-muted);
  margin-top: 3px;
}

.atl-field {
  display: flex;
  flex-direction: column;
}

.atl-field-label {
  display: block;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 5px;
}

.atl-select-box {
  display: flex;
  align-items: center;
  height: var(--h-input);
  padding: 0 4px 0 10px;
  background-color: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
}

.atl-select-input {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  color: var(--c-foreground);
  font-size: var(--fs-base);
  font-family: var(--font-ui);
  cursor: pointer;
}

.atl-keys-table {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
}

.atl-keys-head {
  display: flex;
  align-items: center;
  height: 28px;
  padding: 0 12px;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
}

.atl-keys-row {
  display: flex;
  align-items: center;
  height: 40px;
  padding: 0 12px;
  border-top: 1px solid var(--c-border);
  transition: background 0.1s;
}

.atl-keys-row:hover {
  background: var(--c-raised);
}

.atl-keys-row--expanded {
  background: var(--c-raised);
}

.atl-key-name {
  flex: 2;
  font-size: 13px;
  font-family: var(--font-mono);
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-key-meta {
  flex: 1.4;
  font-size: 12px;
  color: var(--c-muted);
}

.atl-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 24px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
}

.atl-icon-btn:hover {
  background: var(--c-background);
  color: var(--c-foreground);
}

.atl-revoke {
  height: 24px;
  padding: 0 10px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-danger);
  cursor: pointer;
  font-size: 12px;
}

.atl-revoke:hover {
  background: var(--c-raised);
}

/* Grants panel (inline below the key row) */
.atl-grants-panel {
  border-top: 1px solid var(--c-border);
  background: var(--c-background);
  padding: 10px 40px 10px 40px;
}

.atl-key-overview {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding-bottom: 10px;
  margin-bottom: 8px;
  border-bottom: 1px solid var(--c-border);
}

.atl-key-id-line {
  display: flex;
  align-items: center;
  gap: 8px;
}

.atl-key-id-label {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
  flex: 0 0 auto;
}

.atl-key-id {
  flex: 1;
  min-width: 0;
  font-family: var(--font-mono);
  font-size: 11.5px;
  color: var(--c-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-copyid {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: 0 0 auto;
  height: 22px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-muted);
  font-size: 11px;
  cursor: pointer;
}

.atl-copyid:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}

.atl-access-line {
  display: flex;
  align-items: center;
  gap: 8px;
}

.atl-access-term {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
  flex: 0 0 auto;
}

.atl-access-summary {
  font-size: 12.5px;
  color: var(--c-foreground);
}

.atl-global-pill {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  font-weight: var(--fw-medium);
  color: var(--c-agent);
  background: var(--c-agent-bg);
  border: 1px solid var(--c-agent-border);
  border-radius: var(--r-md);
  padding: 2px 8px;
}

.atl-global-toggle {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.atl-switch {
  flex: 0 0 auto;
  position: relative;
  width: 34px;
  height: 20px;
  margin-top: 1px;
  padding: 0;
  border: 1px solid var(--c-border);
  border-radius: 9999px;
  background: var(--c-input);
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s;
}

.atl-switch--on {
  background: var(--c-agent);
  border-color: var(--c-agent);
}

.atl-switch:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.atl-switch-knob {
  position: absolute;
  top: 50%;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 9999px;
  background: var(--c-foreground);
  transform: translateY(-50%);
  transition: left 0.15s;
}

.atl-switch--on .atl-switch-knob {
  left: 17px;
  background: var(--c-on-agent, #fff);
}

.atl-global-copy {
  min-width: 0;
}

.atl-global-label {
  font-size: 12.5px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-global-help {
  font-size: 11.5px;
  color: var(--c-muted);
  line-height: 1.45;
  margin-top: 2px;
  max-width: 440px;
}

.atl-grants-loading {
  font-size: 12px;
  color: var(--c-muted);
  padding: 6px 0;
}

.atl-grants-empty {
  display: flex;
  align-items: center;
  gap: 7px;
  font-size: 12px;
  color: var(--c-muted);
  padding: 4px 0;
}

.atl-grants-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.atl-grant-row {
  display: flex;
  align-items: center;
  gap: 7px;
  min-height: 28px;
  padding: 4px 6px;
  border-radius: var(--r-md);
}

.atl-grant-row:hover {
  background: var(--c-raised);
}

.atl-grant-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.atl-grant-label {
  font-size: 12.5px;
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-grant-by {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--c-muted);
}

.atl-grant-role {
  font-size: 11px;
  font-weight: var(--fw-semibold);
  color: var(--c-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  padding: 2px 6px;
  border: 1px solid var(--c-border);
  border-radius: 3px;
}

.atl-grant-revoke {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: none;
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
  opacity: 0;
}

.atl-grant-row:hover .atl-grant-revoke {
  opacity: 1;
}

.atl-grant-revoke:hover {
  background: var(--c-danger-bg, rgba(239, 68, 68, 0.1));
  color: var(--c-danger);
}

.atl-grants-footer {
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--c-border);
}

.atl-grant-add {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  font-size: 12px;
  cursor: pointer;
}

.atl-grant-add:hover {
  background: var(--c-raised);
}

.atl-grant-add:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.atl-keys-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 54px 20px;
  border: 1px dashed var(--c-border);
  border-radius: 4px;
}

.atl-keys-empty-icon {
  width: 44px;
  height: 44px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--c-chart-5);
  background: var(--c-agent-bg);
  border: 1px solid var(--c-agent-border);
  margin-bottom: 14px;
}

.atl-keys-helper {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--c-muted);
}

.atl-secret-box {
  border: 1px solid rgba(255, 180, 84, 0.45);
  border-radius: 4px;
  overflow: hidden;
}

.atl-secret-warn {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 9px 12px;
  background: rgba(255, 180, 84, 0.12);
  border-bottom: 1px solid rgba(255, 180, 84, 0.45);
  color: var(--c-primary);
  font-size: 12.5px;
  font-weight: var(--fw-semibold);
}

.atl-secret-value {
  flex: 1;
  min-width: 0;
  height: 36px;
  display: flex;
  align-items: center;
  padding: 0 11px;
  background: var(--c-background);
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  font-family: var(--font-mono);
  font-size: 13px;
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-copybtn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 36px;
  padding: 0 12px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: var(--c-raised);
  color: var(--c-foreground);
  cursor: pointer;
  font-size: 12.5px;
}
</style>
