<script setup lang="ts">
import { computed } from 'vue';
import AssigneeAvatars from '@/components/tareas/AssigneeAvatars.vue';
import { useAuthStore } from '@/stores/auth';
import type { ActorDto } from '@/stores/boards';

const props = defineProps<{ actors: ActorDto[] }>();

const auth = useAuthStore();

// Presence always includes the viewer (they are heartbeating themselves in), so
// the stack surfaces who *else* is on the resource and renders nothing when the
// viewer is alone. The avatar stack marks agents distinctly and reuses the same
// primitive as task assignees. Shared by board and document presence.
const others = computed<ActorDto[]>(() => {
  const me = auth.user;
  if (me?.id == null) return props.actors;

  return props.actors.filter((actor) => !(actor.id === me.id && actor.type === me.principal_type));
});
</script>

<template>
  <AssigneeAvatars v-if="others.length > 0" :assignees="others" :max="4" :size="20" />
</template>
