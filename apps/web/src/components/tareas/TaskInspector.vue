<script setup lang="ts">
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import ActivityFeed from '@/components/tareas/ActivityFeed.vue';
import ReferenceAdd from '@/components/tareas/ReferenceAdd.vue';
import ReferenceList from '@/components/tareas/ReferenceList.vue';
import SharePanel from '@/components/share/SharePanel.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import MetaRow from '@/components/ui/MetaRow.vue';
import { relativeTime } from '@/lib/relativeTime';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useUiStore } from '@/stores/ui';

type TaskDto = components['schemas']['TaskDto'];

/**
 * Right-side task inspector (hi-fi TareasDetail): a 280px dock with Details /
 * References / Activity / Share tabs, holding the secondary collections that
 * would otherwise stack inline below the task body. Defaults to Activity, the
 * design's active tab. The body keeps the title, meta card, description and
 * sub-tasks; this panel owns the rest.
 */

const props = defineProps<{
  task: TaskDto;
  ws: string;
}>();

const detail = useTaskDetailStore();
const ui = useUiStore();

type Tab = 'details' | 'references' | 'activity' | 'share';
const TABS: Array<{ id: Tab; label: string }> = [
  { id: 'details', label: 'Details' },
  { id: 'references', label: 'References' },
  { id: 'activity', label: 'Activity' },
  { id: 'share', label: 'Share' },
];
const active = ref<Tab>('activity');

const creator = props.task.created_by;
const creatorName = creator.display_name ?? (creator.type === 'api_key' ? 'Agent' : 'User');
const isAgentCreator = creator.type === 'api_key';

function fail(message: string | null): void {
  if (message !== null) ui.showBanner(message, 'error');
}

async function onAddReference(body: components['schemas']['CreateReferenceRequest']): Promise<void> {
  const ok = await detail.addReference(props.ws, props.task.readable_id, body);
  if (ok) ui.showBanner('Reference added', 'success');
  else fail(detail.error);
}

async function onRemoveReference(referenceId: string): Promise<void> {
  const ok = await detail.removeReference(props.ws, props.task.readable_id, referenceId);
  if (!ok) fail(detail.error);
}
</script>

<template>
  <aside class="atl-tv-inspector">
    <div class="atl-tv-inspector-tabs">
      <button
        v-for="tab in TABS"
        :key="tab.id"
        type="button"
        class="atl-itab"
        :aria-selected="active === tab.id"
        :class="{ on: active === tab.id }"
        @click="active = tab.id"
      >
        {{ tab.label }}
      </button>
    </div>

    <div class="atl-tv-inspector-body">
      <template v-if="active === 'details'">
        <MetaRow label="Created">
          <Avatar :name="creatorName" :agent="isAgentCreator" :size="18" />
          <span style="font-family: var(--font-mono);">{{ creatorName }}</span>
          <AgentBadge v-if="isAgentCreator" />
        </MetaRow>
        <MetaRow label="Created at">
          <span style="font-family: var(--font-mono); color: var(--c-muted);">{{ relativeTime(task.created_at) }}</span>
        </MetaRow>
        <MetaRow label="Updated">
          <span style="font-family: var(--font-mono); color: var(--c-muted);">{{ relativeTime(task.updated_at) }}</span>
        </MetaRow>
        <MetaRow label="Sub-tasks">
          <span style="font-family: var(--font-mono);">{{ detail.subtasks.length }}</span>
        </MetaRow>
        <MetaRow label="References">
          <span style="font-family: var(--font-mono);">{{ detail.references.length }}</span>
        </MetaRow>
        <MetaRow label="Assignees">
          <span style="font-family: var(--font-mono);">{{ detail.assignees.length }}</span>
        </MetaRow>
      </template>

      <template v-else-if="active === 'references'">
        <ReferenceList :references="detail.references" @remove="onRemoveReference" />
        <ReferenceAdd :ws="ws" @add="onAddReference" />
      </template>

      <template v-else-if="active === 'activity'">
        <ActivityFeed :items="detail.activity" />
      </template>

      <template v-else>
        <SharePanel :resource-label="`${task.readable_id} · task`" />
      </template>
    </div>
  </aside>
</template>

<style scoped>
.atl-tv-inspector {
  width: 280px;
  flex: 0 0 280px;
  display: flex;
  flex-direction: column;
  background: var(--c-panel);
  border-left: 1px solid var(--c-border);
}

.atl-tv-inspector-tabs {
  display: flex;
  align-items: flex-end;
  height: 36px;
  flex: 0 0 36px;
  padding: 0 4px;
  border-bottom: 1px solid var(--c-border);
}

.atl-itab {
  height: 28px;
  padding: 0 9px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-sm);
  font-weight: var(--fw-medium);
  color: var(--c-muted);
}

.atl-itab.on {
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
  box-shadow: inset 0 -2px 0 var(--c-primary);
}

.atl-tv-inspector-body {
  flex: 1;
  overflow-y: auto;
  padding: 10px;
}
</style>
