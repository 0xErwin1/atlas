<script setup lang="ts">
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import SharePanel from '@/components/share/SharePanel.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import ActivityComments from '@/components/tareas/ActivityComments.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import InspectorTabs from '@/components/ui/InspectorTabs.vue';
import MetaRow from '@/components/ui/MetaRow.vue';
import { relativeTime } from '@/lib/relativeTime';
import { useTaskDetailStore } from '@/stores/taskDetail';

type TaskDto = components['schemas']['TaskDto'];

/**
 * Right-side task inspector (hi-fi TareasDetail): a 280px dock with Details /
 * References / Activity / Share tabs, holding the secondary collections that
 * would otherwise stack inline below the task body. The Activity tab is one
 * combined feed of system activity and user comments (with a composer), and is
 * the default — the conversation the product surfaces most. The body keeps the
 * title, meta card, description and sub-tasks; this panel owns the rest.
 */

const props = withDefaults(
  defineProps<{
    task: TaskDto;
    ws: string;
    /** Panel width in px (the caller makes the dock resizable). */
    width?: number;
  }>(),
  { width: 280 },
);

const detail = useTaskDetailStore();

type Tab = 'details' | 'activity' | 'share';
const TABS: Array<{ id: Tab; label: string; icon: string }> = [
  { id: 'details', label: 'Details', icon: 'file' },
  { id: 'activity', label: 'Activity', icon: 'message-square' },
  { id: 'share', label: 'Share', icon: 'user' },
];
const active = ref('activity');

const creator = props.task.created_by;
const creatorName = creator.display_name ?? (creator.type === 'api_key' ? 'Agent' : 'User');
const isAgentCreator = creator.type === 'api_key';

function retryDetail(): void {
  void detail.loadAll(props.ws, props.task.readable_id);
}
</script>

<template>
  <aside class="atl-tv-inspector" :style="{ width: `${props.width}px`, flex: `0 0 ${props.width}px` }">
    <InspectorTabs :tabs="TABS" v-model:active="active" />

    <div class="atl-tv-inspector-body">
      <div v-if="active === 'details'" class="atl-tv-inspector-scroll">
        <LoadingState
          v-if="
            detail.assignees.length === 0 &&
            detail.references.length === 0 &&
            detail.subtasks.length === 0 &&
            (detail.collectionStatus.assignees === 'pending' ||
              detail.collectionStatus.references === 'pending' ||
              detail.collectionStatus.subtasks === 'pending')
          "
          label="Loading task details…"
        />
        <ErrorState
          v-else-if="
            detail.collectionStatus.assignees === 'error' ||
            detail.collectionStatus.references === 'error' ||
            detail.collectionStatus.subtasks === 'error'
          "
          title="Could not load task details"
          :hint="
            detail.collectionErrors.assignees ??
            detail.collectionErrors.references ??
            detail.collectionErrors.subtasks ??
            undefined
          "
          @retry="retryDetail"
        />
        <template v-else>
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
      </div>

      <LoadingState
        v-else-if="
          active === 'activity' &&
          detail.activity.length === 0 &&
          detail.comments.length === 0 &&
          (detail.collectionStatus.activity === 'pending' || detail.collectionStatus.comments === 'pending')
        "
        label="Loading activity…"
      />
      <ErrorState
        v-else-if="
          active === 'activity' &&
          (detail.collectionStatus.activity === 'error' || detail.collectionStatus.comments === 'error')
        "
        title="Could not load activity"
        :hint="detail.collectionErrors.activity ?? detail.collectionErrors.comments ?? undefined"
        @retry="retryDetail"
      />
      <ActivityComments v-else-if="active === 'activity'" :ws="ws" :readable-id="task.readable_id" pinned />

      <div v-else class="atl-tv-inspector-scroll">
        <SharePanel :resource-label="`${task.readable_id} · task`" />
      </div>
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

.atl-tv-inspector-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

/* Details / Share tabs scroll as a whole; the Activity tab manages its own feed
   scroll and docks the composer, so it fills this body without an outer scroll. */
.atl-tv-inspector-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 10px;
}
</style>
