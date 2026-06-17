<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';

withDefaults(
  defineProps<{
    icon?: string;
  }>(),
  {
    icon: 'plus',
  },
);

defineEmits<{
  click: [event: MouseEvent];
}>();
</script>

<template>
  <button
    type="button"
    class="atl-gbtn"
    style="
      width: 100%;
      justify-content: flex-start;
      height: 26px;
      gap: 7px;
      color: var(--c-foreground);
    "
    @click="$emit('click', $event)"
  >
    <Icon :name="icon" :size="14" />
    <slot />
  </button>
</template>
