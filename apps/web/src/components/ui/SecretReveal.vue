<script setup lang="ts">
import { ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { useCopyToClipboard } from '@/composables/useCopyToClipboard';

/**
 * One-time reveal of a secret value (an API key secret, an activation link):
 * a warning banner, the value in a mono box, and a copy button. Extracted from
 * the identical markup ApiKeysPanel and UsersPanel used to hand-roll. An
 * optional `caption` slot renders a line above the value.
 */
defineProps<{
  value: string;
  warning: string;
}>();

const { copy } = useCopyToClipboard();
const copied = ref(false);

async function onCopy(secret: string): Promise<void> {
  if (await copy(secret)) copied.value = true;
}
</script>

<template>
  <div class="atl-secret-box">
    <div class="atl-secret-warn">
      <Icon name="triangle-alert" :size="14" style="flex: 0 0 auto;" />
      {{ warning }}
    </div>
    <div class="atl-secret-body">
      <div v-if="$slots.caption" class="atl-secret-caption"><slot name="caption" /></div>
      <div class="flex items-center" style="gap: 8px;">
        <div class="atl-secret-value" data-secret-value>{{ value }}</div>
        <button type="button" class="atl-copybtn" data-secret-copy @click="onCopy(value)">
          <Icon :name="copied ? 'check' : 'copy'" :size="14" />{{ copied ? 'Copied' : 'Copy' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
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

.atl-secret-body {
  padding: 14px;
  background: var(--c-raised);
}

.atl-secret-caption {
  font-size: 12px;
  color: var(--c-muted);
  margin-bottom: 8px;
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
