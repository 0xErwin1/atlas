<script setup lang="ts">
import { computed } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import { useAuthStore } from '@/stores/auth';
import type { ActorDto } from '@/stores/boards';

const props = withDefaults(
  defineProps<{
    assignees: ActorDto[];
    /** Maximum avatars rendered before collapsing the rest into a "+N" chip. */
    max?: number;
    size?: number;
  }>(),
  {
    max: 3,
    size: 18,
  },
);

const auth = useAuthStore();

function isCurrentUser(actor: ActorDto): boolean {
  const me = auth.user;
  return me?.id != null && actor.id === me.id && actor.type === me.principal_type;
}

function displayName(actor: ActorDto): string {
  return actor.display_name ?? (actor.type === 'api_key' ? 'Agent' : 'User');
}

// Current user first (so the viewer recognizes their own tasks), the rest in
// their original order. A stable partition keeps the server ordering otherwise.
const ordered = computed<ActorDto[]>(() => {
  const mine = props.assignees.filter(isCurrentUser);
  const others = props.assignees.filter((a) => !isCurrentUser(a));
  return [...mine, ...others];
});

const visible = computed<ActorDto[]>(() => ordered.value.slice(0, props.max));

const overflow = computed<number>(() => Math.max(0, ordered.value.length - props.max));

const overflowNames = computed<string>(() => ordered.value.slice(props.max).map(displayName).join(', '));
</script>

<template>
  <span class="atl-assignees inline-flex items-center" style="gap: 3px;">
    <span
      v-for="actor in visible"
      :key="`${actor.type}:${actor.id}`"
      class="inline-flex"
      :class="{ 'atl-assignee-me': isCurrentUser(actor) }"
      :title="isCurrentUser(actor) ? `${displayName(actor)} (you)` : displayName(actor)"
    >
      <Avatar :name="displayName(actor)" :agent="actor.type === 'api_key'" :size="size" />
    </span>

    <span
      v-if="overflow > 0"
      class="atl-assignee-more inline-flex items-center justify-center shrink-0 select-none"
      :style="{
        width: `${size}px`,
        height: `${size}px`,
        borderRadius: '2px',
        backgroundColor: 'var(--c-raised)',
        border: '1px solid var(--c-border)',
        fontFamily: 'var(--font-mono)',
        fontSize: `${size <= 18 ? 9 : 10}px`,
        fontWeight: 700,
        color: 'var(--c-muted)',
        lineHeight: '1',
      }"
      :title="overflowNames"
    >
      +{{ overflow }}
    </span>
  </span>
</template>

<style scoped>
.atl-assignee-me {
  border-radius: 2px;
  box-shadow: 0 0 0 1.5px var(--c-primary);
}
</style>
