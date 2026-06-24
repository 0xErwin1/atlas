<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Icon from '@/components/ui/Icon.vue';
import { auditPhrase } from '@/lib/auditPhrase';
import { relativeTime } from '@/lib/relativeTime';
import { type AuditActorFilter, type AuditEntryDto, useAuditStore } from '@/stores/audit';
import { useWorkspaceStore } from '@/stores/workspace';

const store = useAuditStore();
const wsStore = useWorkspaceStore();

const activeWs = computed(() => wsStore.activeWorkspaceSlug ?? '');

type ActorTab = { value: AuditActorFilter; label: string };
const ACTOR_TABS: ActorTab[] = [
  { value: null, label: 'All' },
  { value: 'user', label: 'Users' },
  { value: 'api_key', label: 'Agents' },
];

type CategoryOption = { value: string; label: string; actions: string[] };
const CATEGORY_OPTIONS: CategoryOption[] = [
  { value: '', label: 'All actions', actions: [] },
  { value: 'membership', label: 'Memberships', actions: ['membership.role_changed', 'membership.removed'] },
  { value: 'grant', label: 'Grants', actions: ['grant.created', 'grant.revoked'] },
];

const fromDate = ref('');
const toDate = ref('');
const category = ref('');

const entries = computed(() => store.entries);
const isEmpty = computed(() => !store.loading && store.entries.length === 0);

type AccountStatus = 'deactivated' | 'pending';

function actorName(entry: AuditEntryDto): string {
  const actor = entry.actor;
  return actor.display_name ?? (actor.type === 'api_key' ? 'Agent' : 'User');
}

function isAgent(entry: AuditEntryDto): boolean {
  return entry.actor.type === 'api_key';
}

function agentKindLabel(entry: AuditEntryDto): string {
  const kind = entry.actor.key_type;
  return typeof kind === 'string' && kind.length > 0 ? kind : 'agent';
}

function statusOf(entry: AuditEntryDto): AccountStatus | null {
  const status = entry.actor.account_status;
  if (status === 'deactivated') return 'deactivated';
  if (status === 'pending') return 'pending';
  return null;
}

function statusLabel(status: AccountStatus): string {
  return status === 'deactivated' ? 'Deactivated' : 'Pending';
}

function statusTagClass(status: AccountStatus): string {
  return status === 'deactivated' ? 'atl-status-deactivated' : 'atl-status-pending';
}

function phraseOf(entry: AuditEntryDto): string {
  return auditPhrase(entry.action, entry.metadata, entry.target_label);
}

function timeOf(entry: AuditEntryDto): string {
  return relativeTime(entry.created_at);
}

// Native date inputs yield a date-only string ("2026-06-22"); the API expects an
// RFC3339 instant. Bound `from` at the start of the day and `to` at the end so
// the selected day is inclusive on both edges.
function rangeBounds(): { from: string | null; to: string | null } {
  const from = fromDate.value !== '' ? new Date(`${fromDate.value}T00:00:00`).toISOString() : null;
  const to = toDate.value !== '' ? new Date(`${toDate.value}T23:59:59.999`).toISOString() : null;
  return { from, to };
}

async function reload(): Promise<void> {
  const ws = activeWs.value;
  if (ws === '') return;
  await store.loadWorkspace(ws);
}

function selectActor(value: AuditActorFilter): void {
  if (store.actor === value) return;
  store.setActor(value);
  void reload();
}

function applyRange(): void {
  const bounds = rangeBounds();
  store.setRange(bounds.from, bounds.to);
  void reload();
}

function clearRange(): void {
  fromDate.value = '';
  toDate.value = '';
  store.setRange(null, null);
  void reload();
}

// The category filter narrows the feed to a family of actions. The backend
// accepts a single `action` verb, so a multi-verb category (e.g. "Memberships")
// is applied client-side after fetching; "All actions" clears it.
function applyCategory(): void {
  const selected = CATEGORY_OPTIONS.find((option) => option.value === category.value);
  const single = selected?.actions.length === 1 ? (selected.actions[0] ?? null) : null;
  store.setAction(single);
  void reload();
}

const activeCategoryActions = computed<string[] | null>(() => {
  const selected = CATEGORY_OPTIONS.find((option) => option.value === category.value);
  if (selected === undefined || selected.actions.length <= 1) return null;
  return selected.actions;
});

const visibleEntries = computed<AuditEntryDto[]>(() => {
  const actions = activeCategoryActions.value;
  if (actions === null) return entries.value;
  return entries.value.filter((entry) => actions.includes(entry.action));
});

const hasRange = computed(() => fromDate.value !== '' || toDate.value !== '');

const sentinel = ref<HTMLElement | null>(null);
let observer: IntersectionObserver | null = null;

function observeSentinel(): void {
  if (observer !== null || sentinel.value === null) return;
  observer = new IntersectionObserver((records) => {
    for (const record of records) {
      if (record.isIntersecting) void store.loadMoreWorkspace(activeWs.value);
    }
  });
  observer.observe(sentinel.value);
}

onMounted(() => {
  store.reset();
  store.setActor(null);
  store.setAction(null);
  store.setRange(null, null);
  void reload();
  observeSentinel();
});

onBeforeUnmount(() => {
  observer?.disconnect();
  observer = null;
});

watch(sentinel, () => observeSentinel());

watch(activeWs, (ws, prev) => {
  if (ws !== prev) void reload();
});
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div class="atl-panel-title">Security log</div>
      <div class="atl-panel-sub">
        Security-sensitive changes in this workspace &mdash; role changes, grants, and removals.
        Visible to workspace owners and admins.
      </div>
    </div>

    <div class="atl-activity-controls">
      <div class="atl-actor-tabs" role="tablist" aria-label="Filter by actor">
        <button
          v-for="tab in ACTOR_TABS"
          :key="tab.label"
          type="button"
          role="tab"
          class="atl-actor-tab"
          :class="{ on: store.actor === tab.value }"
          :aria-selected="store.actor === tab.value"
          @click="selectActor(tab.value)"
        >
          {{ tab.label }}
        </button>
      </div>

      <div class="atl-controls-right">
        <select
          v-model="category"
          class="atl-category-select"
          aria-label="Filter by action category"
          @change="applyCategory"
        >
          <option v-for="option in CATEGORY_OPTIONS" :key="option.value" :value="option.value">
            {{ option.label }}
          </option>
        </select>

        <div class="atl-date-range">
          <input
            v-model="fromDate"
            type="date"
            class="atl-date-input"
            aria-label="From date"
            @change="applyRange"
          />
          <span class="atl-date-sep">&ndash;</span>
          <input
            v-model="toDate"
            type="date"
            class="atl-date-input"
            aria-label="To date"
            @change="applyRange"
          />
          <button
            v-if="hasRange"
            type="button"
            class="atl-date-clear"
            title="Clear date range"
            @click="clearRange"
          >
            <Icon name="x" :size="13" />
          </button>
        </div>
      </div>
    </div>

    <div v-if="store.loading && entries.length === 0" class="atl-activity-status">
      Loading&hellip;
    </div>

    <div v-else-if="isEmpty" class="atl-activity-empty">
      <div class="atl-activity-empty-icon"><Icon name="shield" :size="22" /></div>
      <div style="font-size: 14px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
        No security events yet
      </div>
      <div style="font-size: 12.5px; color: var(--c-muted); margin-top: 5px; max-width: 320px; line-height: 1.5;">
        Role changes, grants, and member removals in this workspace will appear here.
      </div>
    </div>

    <div v-else class="atl-activity-feed">
      <div v-for="entry in visibleEntries" :key="entry.id" class="atl-activity-row">
        <Avatar
          :name="actorName(entry)"
          :agent="isAgent(entry)"
          :size="28"
          class="atl-activity-avatar"
        />

        <div class="atl-activity-body">
          <div class="atl-activity-line">
            <span class="atl-activity-actor">{{ actorName(entry) }}</span>

            <span v-if="isAgent(entry)" class="atl-activity-keytype">{{ agentKindLabel(entry) }}</span>

            <span
              v-if="statusOf(entry) !== null"
              class="atl-role-badge atl-status-badge"
              :class="statusTagClass(statusOf(entry)!)"
              :title="statusOf(entry) === 'deactivated' ? 'This account is deactivated' : 'Pending activation'"
            >{{ statusLabel(statusOf(entry)!) }}</span>

            <span class="atl-activity-verb">{{ phraseOf(entry) }}</span>
          </div>

          <div class="atl-activity-time">{{ timeOf(entry) }}</div>
        </div>
      </div>

      <div ref="sentinel" class="atl-activity-sentinel" aria-hidden="true"></div>

      <div v-if="store.loadingMore" class="atl-activity-status">Loading more&hellip;</div>

      <button
        v-else-if="store.hasMore"
        type="button"
        class="atl-activity-more"
        @click="store.loadMoreWorkspace(activeWs)"
      >
        Load more
      </button>
    </div>

    <div v-if="store.error !== null" class="atl-activity-error">
      <Icon name="alert-triangle" :size="13" />
      {{ store.error }}
    </div>
  </div>
</template>

<style scoped>
.atl-panel-head {
  margin-bottom: 16px;
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

.atl-activity-controls {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 10px;
  margin-bottom: 14px;
}

.atl-controls-right {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
}

.atl-actor-tabs {
  display: inline-flex;
  padding: 2px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
}

.atl-actor-tab {
  border: none;
  background: transparent;
  color: var(--c-muted);
  font-size: 12px;
  font-weight: var(--fw-medium);
  font-family: var(--font-ui);
  padding: 4px 12px;
  border-radius: var(--r-sm);
  cursor: pointer;
}

.atl-actor-tab:hover {
  color: var(--c-foreground);
}

.atl-actor-tab.on {
  background: var(--c-background);
  color: var(--c-foreground);
  font-weight: var(--fw-semibold);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
}

.atl-category-select {
  height: 28px;
  padding: 0 8px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-size: 12px;
  font-family: var(--font-ui);
  cursor: pointer;
}

.atl-date-range {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.atl-date-input {
  height: 28px;
  padding: 0 8px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-size: 12px;
  font-family: var(--font-ui);
}

.atl-date-sep {
  color: var(--c-muted);
  font-size: 12px;
}

.atl-date-clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 28px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
}

.atl-date-clear:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}

.atl-activity-feed {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
}

.atl-activity-row {
  display: flex;
  align-items: flex-start;
  gap: 11px;
  padding: 11px 13px;
  border-top: 1px solid var(--c-border);
}

.atl-activity-row:first-child {
  border-top: none;
}

.atl-activity-avatar {
  flex: 0 0 auto;
  margin-top: 1px;
}

.atl-activity-body {
  flex: 1;
  min-width: 0;
}

.atl-activity-line {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  font-size: 13px;
  line-height: 1.5;
  color: var(--c-muted);
}

.atl-activity-actor {
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-activity-keytype {
  display: inline-block;
  font-size: 9.5px;
  font-weight: var(--fw-bold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-agent);
  border: 1px solid var(--c-agent-border);
  background: var(--c-agent-bg);
  border-radius: var(--r-sm);
  padding: 1px 6px;
}

.atl-activity-verb {
  color: var(--c-muted);
}

.atl-activity-time {
  font-size: 11.5px;
  color: var(--c-muted);
  margin-top: 2px;
}

.atl-role-badge {
  display: inline-block;
  font-size: 10px;
  font-weight: var(--fw-bold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  border-radius: var(--r-sm);
  padding: 1px 7px;
}

.atl-status-deactivated {
  color: var(--c-danger);
  border: 1px solid color-mix(in srgb, var(--c-danger) 45%, transparent);
  background: color-mix(in srgb, var(--c-danger) 12%, transparent);
}

.atl-status-pending {
  color: var(--c-primary);
  border: 1px solid color-mix(in srgb, var(--c-primary) 45%, transparent);
  background: color-mix(in srgb, var(--c-primary) 12%, transparent);
}

.atl-activity-sentinel {
  height: 1px;
}

.atl-activity-status {
  font-size: 12.5px;
  color: var(--c-muted);
  padding: 12px 13px;
}

.atl-activity-more {
  display: block;
  width: 100%;
  padding: 10px 13px;
  border: none;
  border-top: 1px solid var(--c-border);
  background: transparent;
  color: var(--c-primary);
  font-size: 12.5px;
  font-weight: var(--fw-semibold);
  font-family: var(--font-ui);
  cursor: pointer;
}

.atl-activity-more:hover {
  background: var(--c-raised);
}

.atl-activity-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 54px 20px;
  border: 1px dashed var(--c-border);
  border-radius: 4px;
}

.atl-activity-empty-icon {
  width: 44px;
  height: 44px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--c-muted);
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  margin-bottom: 14px;
}

.atl-activity-error {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--c-danger);
}
</style>
