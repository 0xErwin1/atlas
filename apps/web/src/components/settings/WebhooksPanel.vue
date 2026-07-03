<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue';
import { z } from 'zod';
import ExpandableRow from '@/components/settings/ExpandableRow.vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import RowAction from '@/components/settings/RowAction.vue';
import SettingsTable from '@/components/settings/SettingsTable.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import Btn from '@/components/ui/Btn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import FormField from '@/components/ui/FormField.vue';
import Icon from '@/components/ui/Icon.vue';
import MultiSelect, { type MultiSelectOption } from '@/components/ui/MultiSelect.vue';
import { EVENT_TYPES } from '@/lib/eventTypes';
import { formatDate } from '@/lib/format';
import { validateForm } from '@/lib/validation';
import { useUiStore } from '@/stores/ui';
import {
  type IntegrationConfigCreatedDto,
  type IntegrationConfigDto,
  useWebhooksStore,
  type WebhookCreatedDto,
  type WebhookDeliveryDto,
  type WebhookDto,
} from '@/stores/webhooks';
import { useWorkspaceStore } from '@/stores/workspace';

const store = useWebhooksStore();
const ui = useUiStore();
const wsStore = useWorkspaceStore();

const ws = computed(() => wsStore.activeWorkspaceSlug);

const eventOptions: MultiSelectOption[] = EVENT_TYPES.map((e) => ({ value: e, label: e }));

// ---------------------------------------------------------------------------
// Outbound webhooks
// ---------------------------------------------------------------------------

type WebhookMode = 'list' | 'new' | 'secret';
const webhookMode = ref<WebhookMode>('list');

const form = reactive({ target_url: '', label: '', event_types: [] as string[] });
const formErrors = reactive<{ target_url: string | null; event_types: string | null }>({
  target_url: null,
  event_types: null,
});
const savingWebhook = ref(false);

const createdWebhook = ref<WebhookCreatedDto | null>(null);
const copiedWebhookSecret = ref(false);

const webhookTogglePending = ref<Record<string, boolean>>({});
const webhookDeleteTarget = ref<{ id: string; label: string } | null>(null);

const expandedWebhookId = ref<string | null>(null);
const deliveriesByWebhook = ref<Record<string, WebhookDeliveryDto[]>>({});
const deliveriesLoading = ref<Record<string, boolean>>({});

const urlSchema = z.object({ target_url: z.string().trim().min(1, 'Target URL is required') });

function webhookTitle(w: WebhookDto): string {
  return w.label && w.label.trim() !== '' ? w.label : w.target_url;
}

function startNewWebhook(): void {
  form.target_url = '';
  form.label = '';
  form.event_types = [];
  formErrors.target_url = null;
  formErrors.event_types = null;
  webhookMode.value = 'new';
}

function cancelNewWebhook(): void {
  webhookMode.value = 'list';
}

async function submitNewWebhook(): Promise<void> {
  if (ws.value === null) return;

  formErrors.target_url = null;
  formErrors.event_types = null;

  const result = validateForm(urlSchema, { target_url: form.target_url });
  if (!result.ok) {
    formErrors.target_url = result.errors.target_url ?? null;
    return;
  }

  if (form.event_types.length === 0) {
    formErrors.event_types = 'Select at least one event type';
    return;
  }

  const label = form.label.trim() === '' ? null : form.label.trim();

  savingWebhook.value = true;
  const created = await store.createWebhook(ws.value, {
    target_url: result.data.target_url,
    event_types: form.event_types,
    label,
    scope_type: 'workspace',
  });
  savingWebhook.value = false;

  if (created === null) {
    if (store.error) ui.showBanner(store.error, 'error');
    return;
  }

  createdWebhook.value = created;
  copiedWebhookSecret.value = false;
  webhookMode.value = 'secret';
}

async function copyWebhookSecret(): Promise<void> {
  if (createdWebhook.value === null) return;
  try {
    await navigator.clipboard.writeText(createdWebhook.value.secret);
    copiedWebhookSecret.value = true;
  } catch {
    ui.showBanner('Clipboard is not available', 'error');
  }
}

function doneWebhookSecret(): void {
  createdWebhook.value = null;
  webhookMode.value = 'list';
}

async function toggleWebhookActive(w: WebhookDto): Promise<void> {
  if (ws.value === null || webhookTogglePending.value[w.id] === true) return;

  webhookTogglePending.value = { ...webhookTogglePending.value, [w.id]: true };
  const ok = await store.updateWebhook(ws.value, w.id, { is_active: !w.is_active });
  webhookTogglePending.value = { ...webhookTogglePending.value, [w.id]: false };

  if (!ok && store.error) ui.showBanner(store.error, 'error');
}

function toggleDeliveries(webhookId: string): void {
  if (expandedWebhookId.value === webhookId) {
    expandedWebhookId.value = null;
    return;
  }

  expandedWebhookId.value = webhookId;

  if (deliveriesByWebhook.value[webhookId] === undefined) {
    void loadDeliveries(webhookId);
  }
}

async function loadDeliveries(webhookId: string): Promise<void> {
  if (ws.value === null) return;

  deliveriesLoading.value = { ...deliveriesLoading.value, [webhookId]: true };
  await store.loadDeliveries(ws.value, webhookId);
  deliveriesLoading.value = { ...deliveriesLoading.value, [webhookId]: false };

  if (store.error) {
    ui.showBanner(store.error, 'error');
    return;
  }

  deliveriesByWebhook.value = { ...deliveriesByWebhook.value, [webhookId]: store.deliveries };
}

function deliveriesFor(webhookId: string): WebhookDeliveryDto[] {
  return deliveriesByWebhook.value[webhookId] ?? [];
}

async function confirmDeleteWebhook(): Promise<void> {
  const target = webhookDeleteTarget.value;
  webhookDeleteTarget.value = null;
  if (target === null || ws.value === null) return;

  const ok = await store.deleteWebhook(ws.value, target.id);
  if (ok) ui.showBanner('Webhook deleted', 'success');
  else if (store.error) ui.showBanner(store.error, 'error');
}

// ---------------------------------------------------------------------------
// Inbound integrations
// ---------------------------------------------------------------------------

type IntegrationMode = 'list' | 'secret';
const integrationMode = ref<IntegrationMode>('list');

const creatingIntegration = ref(false);
const createdIntegration = ref<IntegrationConfigCreatedDto | null>(null);
const copiedIntegrationSecret = ref(false);

const integrationTogglePending = ref<Record<string, boolean>>({});
const integrationDeleteTarget = ref<{ id: string; integration: string } | null>(null);

async function addGithubIntegration(): Promise<void> {
  if (ws.value === null || creatingIntegration.value) return;

  creatingIntegration.value = true;
  const created = await store.createIntegration(ws.value, 'github');
  creatingIntegration.value = false;

  if (created === null) {
    if (store.error) ui.showBanner(store.error, 'error');
    return;
  }

  createdIntegration.value = created;
  copiedIntegrationSecret.value = false;
  integrationMode.value = 'secret';
}

async function copyIntegrationSecret(): Promise<void> {
  if (createdIntegration.value === null) return;
  try {
    await navigator.clipboard.writeText(createdIntegration.value.secret);
    copiedIntegrationSecret.value = true;
  } catch {
    ui.showBanner('Clipboard is not available', 'error');
  }
}

function doneIntegrationSecret(): void {
  createdIntegration.value = null;
  integrationMode.value = 'list';
}

async function toggleIntegrationActive(i: IntegrationConfigDto): Promise<void> {
  if (ws.value === null || integrationTogglePending.value[i.id] === true) return;

  integrationTogglePending.value = { ...integrationTogglePending.value, [i.id]: true };
  const ok = await store.setIntegrationActive(ws.value, i.id, !i.is_active);
  integrationTogglePending.value = { ...integrationTogglePending.value, [i.id]: false };

  if (!ok && store.error) ui.showBanner(store.error, 'error');
}

async function confirmDeleteIntegration(): Promise<void> {
  const target = integrationDeleteTarget.value;
  integrationDeleteTarget.value = null;
  if (target === null || ws.value === null) return;

  const ok = await store.deleteIntegration(ws.value, target.id);
  if (ok) ui.showBanner('Integration deleted', 'success');
  else if (store.error) ui.showBanner(store.error, 'error');
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

async function loadAll(slug: string): Promise<void> {
  expandedWebhookId.value = null;
  deliveriesByWebhook.value = {};

  await Promise.all([store.loadWebhooks(slug), store.loadIntegrations(slug)]);

  if (store.error) ui.showBanner(store.error, 'error');
}

onMounted(() => {
  if (ws.value !== null) void loadAll(ws.value);
});

watch(ws, (slug) => {
  webhookMode.value = 'list';
  integrationMode.value = 'list';
  if (slug !== null) void loadAll(slug);
});
</script>

<template>
  <div>
    <PanelHeader
      title="Webhooks & Events"
      subtitle="Send workspace events to outbound webhooks, and let inbound integrations act on this workspace."
    />

    <!-- ============================ Section A ============================ -->
    <section class="atl-section">
      <!-- One-time webhook secret -->
      <div v-if="webhookMode === 'secret' && createdWebhook">
        <div class="atl-section-head">
          <div class="atl-section-title">New webhook secret</div>
        </div>

        <div class="atl-secret-box">
          <div class="atl-secret-warn">
            <Icon name="triangle-alert" :size="14" style="flex: 0 0 auto;" />
            Copy this now — you won't be able to see it again.
          </div>
          <div style="padding: 14px; background: var(--c-raised);">
            <div style="font-size: 12px; color: var(--c-muted); margin-bottom: 8px;">
              Signing secret for
              <span style="font-family: var(--font-mono); color: var(--c-foreground);">{{ createdWebhook.target_url }}</span>
            </div>
            <div class="flex items-center" style="gap: 8px;">
              <div class="atl-secret-value">{{ createdWebhook.secret }}</div>
              <button type="button" class="atl-copybtn" @click="copyWebhookSecret">
                <Icon :name="copiedWebhookSecret ? 'check' : 'copy'" :size="14" />{{ copiedWebhookSecret ? 'Copied' : 'Copy' }}
              </button>
            </div>
            <div class="atl-secret-hint">
              This secret signs outbound payloads (HMAC-SHA256) so your endpoint can verify them.
            </div>
          </div>
        </div>

        <div class="flex" style="justify-content: flex-end; margin-top: 16px;">
          <Btn variant="secondary" @click="doneWebhookSecret">Done</Btn>
        </div>
      </div>

      <!-- New webhook form -->
      <div v-else-if="webhookMode === 'new'">
        <div class="atl-section-head">
          <div class="atl-section-title">New webhook</div>
        </div>

        <div class="flex flex-col" style="gap: 14px; max-width: 480px;">
          <FormField
            label="Target URL"
            :model-value="form.target_url"
            placeholder="https://example.com/atlas/webhook"
            mono
            :error="formErrors.target_url"
            @update:model-value="(v) => { form.target_url = v; formErrors.target_url = null; }"
          />

          <FormField
            label="Label (optional)"
            :model-value="form.label"
            placeholder="Deploy notifier"
            @update:model-value="(v) => { form.label = v; }"
          />

          <div class="atl-field">
            <label class="atl-field-label">Event types</label>
            <MultiSelect
              v-model="form.event_types"
              placeholder="Select events"
              icon="zap"
              :options="eventOptions"
            />
            <div v-if="formErrors.event_types" class="atl-field-error">
              <Icon name="triangle-alert" :size="12" />
              {{ formErrors.event_types }}
            </div>
          </div>
        </div>

        <div class="flex" style="gap: 8px; margin-top: 20px;">
          <Btn variant="primary" :disabled="savingWebhook" @click="submitNewWebhook">Create webhook</Btn>
          <Btn variant="secondary" @click="cancelNewWebhook">Cancel</Btn>
        </div>
      </div>

      <!-- Webhook list -->
      <div v-else>
        <div class="atl-section-head">
          <div class="atl-section-text">
            <div class="atl-section-title">Outbound webhooks</div>
            <div class="atl-section-sub">HTTPS endpoints Atlas POSTs signed event payloads to.</div>
          </div>
          <Btn variant="primary" @click="startNewWebhook">
            <Icon name="plus" :size="14" />Add webhook
          </Btn>
        </div>

        <EmptyState
          v-if="store.webhooks.length === 0"
          compact
          icon="webhook"
          title="No webhooks yet"
          hint="Add a webhook to receive workspace events on your own endpoint."
        >
          <template #actions>
            <Btn variant="primary" @click="startNewWebhook"><Icon name="plus" :size="14" />Add webhook</Btn>
          </template>
        </EmptyState>

        <SettingsTable v-else>
          <template #head>
            <div style="flex: 0 0 26px;"></div>
            <div style="flex: 2;">Endpoint</div>
            <div style="flex: 2;">Events</div>
            <div style="flex: 0 0 70px;">Active</div>
            <div style="flex: 0 0 210px;"></div>
          </template>

          <ExpandableRow
            v-for="w in store.webhooks"
            :key="w.id"
            :expanded="expandedWebhookId === w.id"
            :style="{ height: '44px', '--erow-actions-basis': '210px' }"
            @toggle="toggleDeliveries(w.id)"
          >
            <template #summary>
              <div style="flex: 0 0 26px;">
                <Icon name="webhook" :size="15" style="color: var(--c-chart-5);" />
              </div>
              <div class="atl-wh-endpoint">
                <span class="atl-wh-label">{{ webhookTitle(w) }}</span>
                <span class="atl-wh-url">{{ w.target_url }}</span>
              </div>
              <div class="atl-wh-events">
                <span v-for="ev in w.event_types" :key="ev" class="atl-chip">{{ ev }}</span>
              </div>
              <div style="flex: 0 0 70px;">
                <button
                  type="button"
                  role="switch"
                  class="atl-switch"
                  :class="{ 'atl-switch--on': w.is_active }"
                  :aria-checked="w.is_active"
                  :disabled="webhookTogglePending[w.id] === true"
                  :aria-label="w.is_active ? 'Deactivate webhook' : 'Activate webhook'"
                  data-webhook-toggle
                  @click.stop="toggleWebhookActive(w)"
                >
                  <span class="atl-switch-knob" />
                </button>
              </div>
            </template>

            <template #actions>
              <RowAction
                tone="danger"
                title="Delete webhook"
                @click="webhookDeleteTarget = { id: w.id, label: webhookTitle(w) }"
              >
                Delete
              </RowAction>
            </template>

            <template #panel>
              <div class="atl-deliveries-head">Recent deliveries</div>

              <div v-if="deliveriesLoading[w.id]" class="atl-deliveries-empty">Loading&hellip;</div>

              <div v-else-if="deliveriesFor(w.id).length === 0" class="atl-deliveries-empty">
                No delivery attempts yet.
              </div>

              <div v-else class="atl-deliveries-list">
                <div v-for="d in deliveriesFor(w.id)" :key="d.id" class="atl-delivery-row">
                  <span
                    class="atl-delivery-outcome"
                    :class="d.outcome === 'success' ? 'atl-delivery-ok' : 'atl-delivery-fail'"
                  >
                    <Icon :name="d.outcome === 'success' ? 'check' : 'x'" :size="12" />
                    {{ d.outcome }}
                  </span>
                  <span class="atl-delivery-status">{{ d.status_code ?? '—' }}</span>
                  <span class="atl-delivery-attempt">attempt {{ d.attempt_no }}</span>
                  <span class="atl-delivery-time">{{ formatDate(d.created_at) }}</span>
                  <span v-if="d.error" class="atl-delivery-error">{{ d.error }}</span>
                </div>
              </div>
            </template>
          </ExpandableRow>
        </SettingsTable>
      </div>
    </section>

    <!-- ============================ Section B ============================ -->
    <section class="atl-section">
      <!-- One-time integration secret -->
      <div v-if="integrationMode === 'secret' && createdIntegration">
        <div class="atl-section-head">
          <div class="atl-section-title">New integration secret</div>
        </div>

        <div class="atl-secret-box">
          <div class="atl-secret-warn">
            <Icon name="triangle-alert" :size="14" style="flex: 0 0 auto;" />
            Copy this now — you won't be able to see it again.
          </div>
          <div style="padding: 14px; background: var(--c-raised);">
            <div style="font-size: 12px; color: var(--c-muted); margin-bottom: 8px;">
              Signing secret for
              <span style="font-family: var(--font-mono); color: var(--c-foreground);">{{ createdIntegration.integration }}</span>
            </div>
            <div class="flex items-center" style="gap: 8px;">
              <div class="atl-secret-value">{{ createdIntegration.secret }}</div>
              <button type="button" class="atl-copybtn" @click="copyIntegrationSecret">
                <Icon :name="copiedIntegrationSecret ? 'check' : 'copy'" :size="14" />{{ copiedIntegrationSecret ? 'Copied' : 'Copy' }}
              </button>
            </div>
            <div class="atl-secret-hint">
              This secret signs inbound GitHub webhook payloads — set it as the webhook secret in GitHub.
            </div>
          </div>
        </div>

        <div class="flex" style="justify-content: flex-end; margin-top: 16px;">
          <Btn variant="secondary" @click="doneIntegrationSecret">Done</Btn>
        </div>
      </div>

      <!-- Integration list -->
      <div v-else>
        <div class="atl-section-head">
          <div class="atl-section-text">
            <div class="atl-section-title">Inbound integrations</div>
            <div class="atl-section-sub">External services that send signed events into this workspace.</div>
          </div>
          <Btn variant="primary" :disabled="creatingIntegration" @click="addGithubIntegration">
            <Icon name="plus" :size="14" />Add GitHub
          </Btn>
        </div>

        <EmptyState
          v-if="store.integrations.length === 0"
          compact
          icon="git-branch"
          title="No integrations yet"
          hint="Add the GitHub integration to let GitHub events act on this workspace."
        >
          <template #actions>
            <Btn variant="primary" :disabled="creatingIntegration" @click="addGithubIntegration">
              <Icon name="plus" :size="14" />Add GitHub
            </Btn>
          </template>
        </EmptyState>

        <SettingsTable v-else>
          <template #head>
            <div style="flex: 0 0 26px;"></div>
            <div style="flex: 2;">Integration</div>
            <div style="flex: 1.4;">Created</div>
            <div style="flex: 0 0 70px;">Active</div>
            <div style="flex: 0 0 120px;"></div>
          </template>

          <ExpandableRow
            v-for="i in store.integrations"
            :key="i.id"
            :expanded="false"
            :expandable="false"
            :style="{ height: '44px', '--erow-actions-basis': '120px' }"
          >
            <template #summary>
              <div style="flex: 0 0 26px;">
                <Icon name="git-branch" :size="15" style="color: var(--c-chart-5);" />
              </div>
              <div class="atl-int-name">{{ i.integration }}</div>
              <div class="atl-int-meta">{{ formatDate(i.created_at) }}</div>
              <div style="flex: 0 0 70px;">
                <button
                  type="button"
                  role="switch"
                  class="atl-switch"
                  :class="{ 'atl-switch--on': i.is_active }"
                  :aria-checked="i.is_active"
                  :disabled="integrationTogglePending[i.id] === true"
                  :aria-label="i.is_active ? 'Deactivate integration' : 'Activate integration'"
                  data-integration-toggle
                  @click.stop="toggleIntegrationActive(i)"
                >
                  <span class="atl-switch-knob" />
                </button>
              </div>
            </template>

            <template #actions>
              <RowAction
                tone="danger"
                title="Delete integration"
                @click="integrationDeleteTarget = { id: i.id, integration: i.integration }"
              >
                Delete
              </RowAction>
            </template>
          </ExpandableRow>
        </SettingsTable>
      </div>
    </section>

    <ConfirmDialog
      :open="webhookDeleteTarget !== null"
      tone="danger"
      title="Delete this webhook?"
      message="Atlas stops sending events to this endpoint immediately."
      :detail="webhookDeleteTarget?.label"
      detail-icon="webhook"
      note="This can't be undone — you'll need to recreate the webhook and reconfigure your endpoint."
      confirm-label="Delete webhook"
      confirm-icon="trash-2"
      @confirm="confirmDeleteWebhook"
      @cancel="webhookDeleteTarget = null"
    />

    <ConfirmDialog
      :open="integrationDeleteTarget !== null"
      tone="danger"
      title="Delete this integration?"
      message="Inbound events for this integration are rejected immediately and its signing secret is revoked."
      :detail="integrationDeleteTarget?.integration"
      detail-icon="git-branch"
      note="This can't be undone — you'll need to recreate the integration and reconfigure the external service."
      confirm-label="Delete integration"
      confirm-icon="trash-2"
      @confirm="confirmDeleteIntegration"
      @cancel="integrationDeleteTarget = null"
    />
  </div>
</template>

<style scoped>
.atl-section {
  margin-bottom: 28px;
}

.atl-section-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}

.atl-section-text {
  min-width: 0;
}

.atl-section-title {
  font-size: 13px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-section-sub {
  font-size: 12px;
  color: var(--c-muted);
  margin-top: 2px;
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

.atl-field-error {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  color: var(--c-danger);
  margin-top: 5px;
}

.atl-wh-endpoint {
  flex: 2;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.atl-wh-label {
  font-size: 13px;
  color: var(--c-foreground);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-wh-url {
  font-size: 11px;
  font-family: var(--font-mono);
  color: var(--c-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-wh-events {
  flex: 2;
  min-width: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 3px;
  align-content: center;
}

.atl-chip {
  height: 17px;
  display: inline-flex;
  align-items: center;
  padding: 0 5px;
  font-size: 10.5px;
  font-family: var(--font-mono);
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: 3px;
  color: var(--c-muted);
}

.atl-int-name {
  flex: 2;
  font-size: 13px;
  color: var(--c-foreground);
}

.atl-int-meta {
  flex: 1.4;
  font-size: 12px;
  color: var(--c-muted);
}

.atl-switch {
  flex: 0 0 auto;
  position: relative;
  width: 34px;
  height: 20px;
  padding: 0;
  border: 1px solid var(--c-border);
  border-radius: 9999px;
  background: var(--c-input);
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s;
}

.atl-switch--on {
  background: var(--c-primary);
  border-color: var(--c-primary);
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
  background: var(--c-primary-fg, #fff);
}

.atl-deliveries-head {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 8px;
}

.atl-deliveries-empty {
  font-size: 12px;
  color: var(--c-muted);
  padding: 4px 0;
}

.atl-deliveries-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.atl-delivery-row {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
}

.atl-delivery-outcome {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
  font-weight: var(--fw-semibold);
  text-transform: capitalize;
}

.atl-delivery-ok {
  color: var(--c-success, #6bbf59);
}

.atl-delivery-fail {
  color: var(--c-danger);
}

.atl-delivery-status {
  flex: 0 0 auto;
  font-family: var(--font-mono);
  color: var(--c-foreground);
}

.atl-delivery-attempt {
  flex: 0 0 auto;
  color: var(--c-muted);
}

.atl-delivery-time {
  flex: 0 0 auto;
  color: var(--c-muted);
}

.atl-delivery-error {
  flex: 1;
  min-width: 0;
  color: var(--c-danger);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
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

.atl-secret-hint {
  font-size: 11.5px;
  color: var(--c-muted);
  margin-top: 10px;
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
