<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Icon from '@/components/ui/Icon.vue';
import { activityPhrase } from '@/lib/activityPhrase';
import { relativeTime } from '@/lib/relativeTime';
import { type ActivityEntryDto, type ActorFilter, useActivityStore } from '@/stores/activity';
import { useWorkspaceStore } from '@/stores/workspace';

const store = useActivityStore();
const wsStore = useWorkspaceStore();

const activeWs = computed(() => wsStore.activeWorkspaceSlug ?? '');

type ActorTab = { value: ActorFilter; label: string };
const ACTOR_TABS: ActorTab[] = [
  { value: null, label: 'All' },
  { value: 'user', label: 'Humans' },
  { value: 'api_key', label: 'Agents' },
];

const fromDate = ref('');
const toDate = ref('');

const entries = computed(() => store.entries);
const isEmpty = computed(() => !store.loading && store.entries.length === 0);

type AccountStatus = 'deactivated' | 'pending';

function actorName(entry: ActivityEntryDto): string {
  const actor = entry.actor;
  return actor.display_name ?? (actor.type === 'api_key' ? 'Agent' : 'User');
}

function isAgent(entry: ActivityEntryDto): boolean {
  return entry.actor.type === 'api_key';
}

// For api_key actors the key purpose (agent/cli/bot/integration) is shown as a
// small label so the feed distinguishes an automation from a person at a glance.
function agentKindLabel(entry: ActivityEntryDto): string {
  const kind = entry.actor.key_type;
  return typeof kind === 'string' && kind.length > 0 ? kind : 'agent';
}

function statusOf(entry: ActivityEntryDto): AccountStatus | null {
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

function phraseOf(entry: ActivityEntryDto): string {
  return activityPhrase(entry.kind, entry.payload);
}

function timeOf(entry: ActivityEntryDto): string {
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
  await store.load(ws);
}

function selectActor(value: ActorFilter): void {
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

const hasRange = computed(() => fromDate.value !== '' || toDate.value !== '');

const sentinel = ref<HTMLElement | null>(null);
let observer: IntersectionObserver | null = null;

function observeSentinel(): void {
  if (observer !== null || sentinel.value === null) return;
  observer = new IntersectionObserver((records) => {
    for (const record of records) {
      if (record.isIntersecting) void store.loadMore(activeWs.value);
    }
  });
  observer.observe(sentinel.value);
}

onMounted(() => {
  store.reset();
  store.setActor(null);
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
      <div class="atl-panel-title">Activity</div>
      <div class="atl-panel-sub">
        Who did what across this workspace. You only see activity on tasks you can access.
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

    <div v-if="store.loading && entries.length === 0" class="atl-activity-status">
      Loading&hellip;
    </div>

    <div v-else-if="isEmpty" class="atl-activity-empty">
      <div class="atl-activity-empty-icon"><Icon name="history" :size="22" /></div>
      <div style="font-size: 14px; font-weight: var(--fw-semibold); color: var(--c-foreground);">
        No activity yet
      </div>
      <div style="font-size: 12.5px; color: var(--c-muted); margin-top: 5px; max-width: 320px; line-height: 1.5;">
        Changes to tasks you can access &mdash; moves, assignments, edits &mdash; will appear here.
      </div>
    </div>

    <div v-else class="atl-activity-feed">
      <div v-for="entry in entries" :key="entry.id" class="atl-activity-row">
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

            <RouterLink
              class="atl-activity-task"
              :to="{ name: 'task-detail', params: { readableId: entry.task_readable_id } }"
            >{{ entry.task_readable_id }}</RouterLink>
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
        @click="store.loadMore(activeWs)"
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

.atl-activity-task {
  font-weight: var(--fw-semibold);
  color: var(--c-primary);
  text-decoration: none;
  font-family: var(--font-mono);
  font-size: 12.5px;
}

.atl-activity-task:hover {
  text-decoration: underline;
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
