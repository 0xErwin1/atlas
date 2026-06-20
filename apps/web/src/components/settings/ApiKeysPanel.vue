<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { z } from 'zod';
import Btn from '@/components/ui/Btn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import { validateForm } from '@/lib/validation';
import { type ApiKeyCreated, useApiKeysStore } from '@/stores/apiKeys';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const keysStore = useApiKeysStore();
const ui = useUiStore();
const workspace = useWorkspaceStore();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

type Mode = 'list' | 'new' | 'secret';
const mode = ref<Mode>('list');

const form = reactive({ name: '', expires: '' });
const formErrors = reactive<{ name: string | null }>({ name: null });
const saving = ref(false);

const created = ref<ApiKeyCreated | null>(null);
const copied = ref(false);

const revokeTarget = ref<{ id: string; name: string } | null>(null);

const revokeDetail = computed(() => {
  const target = revokeTarget.value;
  if (target === null) return undefined;
  // The list never exposes the secret prefix (it is shown only once at
  // creation). The key id is the only stable identifier we can surface, so the
  // fragment is taken from it rather than from a fabricated secret prefix.
  return `${target.name} · ${target.id.slice(0, 8)}…`;
});

const nameSchema = z.object({ name: z.string().trim().min(1, 'Name is required') });

function fmtDate(iso: string | null | undefined): string {
  if (!iso) return 'Never';
  const d = new Date(iso);
  return d.toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: '2-digit' });
}

onMounted(() => {
  if (ws.value !== '') keysStore.loadKeys(ws.value);
});

function startNew(): void {
  form.name = '';
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
  const res = await keysStore.createKey(ws.value, result.data.name, expiresAt);
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
  await keysStore.loadKeys(ws.value);
}

async function confirmRevoke(): Promise<void> {
  const target = revokeTarget.value;
  revokeTarget.value = null;
  if (target === null) return;

  const ok = await keysStore.revokeKey(ws.value, target.id);
  if (ok) ui.showBanner('API key revoked', 'success');
  else if (keysStore.error) ui.showBanner(keysStore.error, 'error');
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
          Copy this now — you won’t be able to see it again.
        </div>
        <div style="padding: 14px; background: var(--c-raised);">
          <div style="font-size: 12px; color: var(--c-muted); margin-bottom: 8px;">
            Secret for key
            <span style="font-family: var(--font-mono); color: var(--c-foreground);">“{{ created.name }}”</span>
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
        <div class="atl-panel-sub">Provisions an agent identity, capped at editor.</div>
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
        Loading…
      </div>

      <div v-else-if="keysStore.keys.length === 0" class="atl-keys-empty">
        <div class="atl-keys-empty-icon"><Icon name="key" :size="22" /></div>
        <div style="font-size: 14px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
          No API keys yet
        </div>
        <div style="font-size: 12.5px; color: var(--c-muted); margin-top: 5px; max-width: 300px; line-height: 1.5;">
          Create a key to let an agent act on your behalf. Agents act capped at editor.
        </div>
        <div style="margin-top: 16px;">
          <Btn variant="primary" @click="startNew"><Icon name="plus" :size="14" />New key</Btn>
        </div>
      </div>

      <div v-else class="atl-keys-table">
        <div class="atl-keys-head">
          <div style="flex: 0 0 26px;"></div>
          <div style="flex: 2;">Name</div>
          <div style="flex: 1.4;">Created</div>
          <div style="flex: 1.4;">Expires</div>
          <div style="flex: 0 0 84px;"></div>
        </div>
        <div v-for="k in keysStore.keys" :key="k.id" class="atl-keys-row">
          <div style="flex: 0 0 26px;"><Icon name="key" :size="15" style="color: var(--c-chart-5);" /></div>
          <div class="atl-key-name">{{ k.name }}</div>
          <div class="atl-key-meta">{{ fmtDate(k.created_at) }}</div>
          <div class="atl-key-meta" :style="{ color: k.expires_at ? 'var(--c-foreground)' : 'var(--c-muted)' }">
            {{ fmtDate(k.expires_at) }}
          </div>
          <div style="flex: 0 0 84px; display: flex; justify-content: flex-end;">
            <button type="button" class="atl-revoke" @click="revokeTarget = { id: k.id, name: k.name }">
              Revoke
            </button>
          </div>
        </div>
      </div>

      <div class="atl-keys-helper">
        <Icon name="sparkles" :size="13" style="color: var(--c-chart-5);" />
        <span>Keys provision <span style="color: var(--c-chart-5); font-weight: var(--fw-semibold);">agent</span> identities — agents act capped at editor.</span>
      </div>
    </div>

    <ConfirmDialog
      :open="revokeTarget !== null"
      tone="danger"
      title="Revoke this API key?"
      message="The key stops working immediately. Any agent or script still using it loses access at once."
      :detail="revokeDetail"
      detail-icon="key"
      note="This can’t be undone — you’ll need to issue a new key and update anything that relied on it."
      confirm-label="Revoke key"
      confirm-icon="trash-2"
      @confirm="confirmRevoke"
      @cancel="revokeTarget = null"
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
