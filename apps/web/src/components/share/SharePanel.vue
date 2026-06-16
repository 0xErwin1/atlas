<script setup lang="ts">
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import { useUiStore } from '@/stores/ui';

const props = withDefaults(
  defineProps<{
    resourceLabel?: string;
  }>(),
  {
    resourceLabel: '',
  },
);

const ui = useUiStore();
</script>

<template>
  <div class="flex flex-col" style="gap: 10px;">
    <p style="font-size: var(--fs-sm); color: var(--c-muted); line-height: 1.45; margin: 0;">
      Manage who — people, agents, and scripts — can access this resource and its visibility.
    </p>
    <Btn variant="secondary" data-action="open-share" @click="ui.openShare(props.resourceLabel)">
      <Icon name="user-plus" :size="14" />
      Share…
    </Btn>
  </div>
</template>
