<script setup lang="ts">
/**
 * Version-history inspector panel (notas.jsx HistoryPanel). Renders the design's
 * revision timeline: a "Version history" heading, a +N/-N diff line, revision
 * rows (actor avatar + name + optional agent badge + change description +
 * timestamp + a Restore action) and a live agent-presence pill.
 *
 * The backend exposes no revision history yet, so the panel renders its
 * EMPTY/placeholder state. The `revisions` and `presence` props carry the design
 * structure for when a history backend lands; they are intentionally empty here
 * (no fabricated revisions). When `revisions` is non-empty the timeline renders.
 */
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Presence from '@/components/ui/Presence.vue';

interface Revision {
  id: string;
  actorName: string;
  actorInitials: string;
  agent: boolean;
  change: string;
  when: string;
}

const props = withDefaults(
  defineProps<{
    revisions?: Revision[];
    /** Net line delta of the current revision, or null when unknown. */
    diff?: { added: number; removed: number } | null;
    /** Label of the agent currently editing live, or null when none. */
    presence?: string | null;
  }>(),
  {
    revisions: () => [],
    diff: null,
    presence: null,
  },
);

const emit = defineEmits<{
  restore: [id: string];
}>();
</script>

<template>
  <div>
    <div
      style="
        font-size: 10px;
        font-weight: var(--fw-semibold);
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--c-muted);
        margin-bottom: 8px;
      "
    >
      Version history
    </div>

    <p
      v-if="props.revisions.length === 0"
      style="font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No version history yet.
    </p>

    <template v-else>
      <div
        v-if="props.diff"
        style="font-size: var(--fs-sm); color: var(--c-foreground); margin-bottom: 8px;"
      >
        <span style="color: var(--c-success); font-weight: var(--fw-bold);">+{{ props.diff.added }}</span>
        &nbsp;&nbsp;
        <span style="color: var(--c-warning); font-weight: var(--fw-bold);">&minus;{{ props.diff.removed }}</span>
        &nbsp;&nbsp;lines in current revision
      </div>

      <div
        v-for="rev in props.revisions"
        :key="rev.id"
        class="flex"
        style="gap: 8px; padding: 7px 0;"
      >
        <Avatar :name="rev.actorInitials" :agent="rev.agent" :size="22" />
        <div class="flex-1 min-w-0">
          <div
            class="flex items-center"
            style="gap: 6px; font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
          >
            {{ rev.actorName }}
            <AgentBadge v-if="rev.agent" />
          </div>
          <div style="font-size: var(--fs-sm); color: var(--c-muted); line-height: 1.4;">
            {{ rev.change }}
          </div>
          <div
            style="font-size: var(--fs-xs); color: var(--c-muted); font-family: var(--font-mono); margin-top: 1px;"
          >
            {{ rev.when }}&nbsp;&nbsp;·&nbsp;&nbsp;
            <button
              type="button"
              style="
                border: none;
                background: transparent;
                padding: 0;
                cursor: pointer;
                color: var(--c-primary);
                font-weight: var(--fw-bold);
                font-family: var(--font-mono);
                font-size: var(--fs-xs);
              "
              @click="emit('restore', rev.id)"
            >
              Restore
            </button>
          </div>
        </div>
      </div>

      <Presence v-if="props.presence" style="margin-top: 6px;">
        {{ props.presence }}
      </Presence>
    </template>
  </div>
</template>
