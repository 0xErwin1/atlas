<script setup lang="ts">
import { computed, ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';

const props = withDefaults(
  defineProps<{
    title: string;
    /** Actionable hint from the API problem — the only error prose shown to users. */
    hint?: string;
    /**
     * Raw problem `detail`. Accepted so callers can pass a whole problem, but
     * DELIBERATELY never rendered: detail may carry internal traces. REQ-W28
     * forbids surfacing stack traces / raw detail.
     */
    detail?: string;
    type?: string;
    status?: number;
    requestId?: string;
    /** Wrap the content in the design's bordered card with a 32px context bar. */
    framed?: boolean;
    /** Mono context label shown in the framed card's top bar (e.g. "Atlas · Tasks"). */
    topLabel?: string;
  }>(),
  {
    hint: undefined,
    detail: undefined,
    type: undefined,
    status: undefined,
    requestId: undefined,
    framed: false,
    topLabel: undefined,
  },
);

const emit = defineEmits<{
  retry: [];
  copy: [requestId: string];
}>();

const copied = ref(false);

async function copyError() {
  const id = props.requestId;
  if (id === undefined) return;

  emit('copy', id);

  try {
    await navigator.clipboard.writeText(id);
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    // Clipboard unavailable (insecure context / denied): the emit still fires
    // so a parent can fall back to its own feedback.
  }
}

const message = computed(() => props.hint ?? 'Something went wrong. Your work is safe — retry in a moment.');

const diagnostics = computed(() => {
  const parts: string[] = [];
  if (props.type) parts.push(props.type);
  if (props.status !== undefined) parts.push(String(props.status));
  if (props.requestId) parts.push(`trace ${props.requestId}`);
  return parts.join(' · ');
});
</script>

<template>
  <div
    v-if="framed"
    data-state="error"
    class="flex flex-col"
    style="width: 100%; height: 100%; background-color: var(--c-background); border: 1px solid var(--c-border); border-radius: 3px; overflow: hidden;"
  >
    <div
      class="flex items-center"
      style="gap: 8px; height: 32px; flex: 0 0 32px; padding: 0 10px; border-bottom: 1px solid var(--c-border); background-color: var(--c-panel);"
    >
      <button
        type="button"
        class="atl-gbtn"
        style="width: 22px; height: 22px;"
        aria-label="Toggle panel"
      >
        <Icon name="panel-right" :size="13" />
      </button>
      <span
        v-if="topLabel"
        style="font-size: var(--fs-sm); color: var(--c-muted); font-family: var(--font-mono);"
      >
        {{ topLabel }}
      </span>
    </div>

    <div
      class="flex flex-col items-center justify-center text-center"
      style="flex: 1; gap: 12px; padding: 24px; min-height: 0;"
    >
      <div
        class="text-left"
        style="
          width: 340px;
          max-width: 100%;
          background-color: var(--c-banner-err-bg);
          border: 1px solid rgba(240, 113, 120, 0.5);
          border-radius: var(--r-md);
          padding: 11px 13px;
        "
      >
        <div
          class="flex items-center"
          style="gap: 8px; font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-banner-err-fg); margin-bottom: 5px;"
        >
          <Icon name="triangle-alert" :size="15" />
          {{ title }}
        </div>
        <div
          style="font-size: 12.5px; color: var(--c-banner-err-fg); opacity: 0.92; line-height: 1.45; margin-bottom: 7px;"
        >
          {{ message }}
        </div>
        <div
          v-if="diagnostics"
          style="font-size: var(--fs-xs); font-family: var(--font-mono); color: var(--c-banner-err-fg); opacity: 0.7;"
        >
          {{ diagnostics }}
        </div>
      </div>

      <div class="flex" style="gap: 8px;">
        <button
          type="button"
          data-action="retry"
          class="inline-flex items-center justify-center gap-1 cursor-pointer select-none"
          style="
            height: var(--h-button);
            padding: 0 10px;
            border-radius: var(--r-md);
            border: 1px solid transparent;
            background-color: var(--c-primary);
            color: var(--c-primary-fg);
            font-family: var(--font-mono);
            font-size: var(--fs-sm);
            font-weight: var(--fw-medium);
          "
          @click="emit('retry')"
        >
          <Icon name="refresh-cw" :size="14" />
          Retry
        </button>
        <button
          v-if="requestId"
          type="button"
          data-action="copy"
          class="inline-flex items-center justify-center gap-1 cursor-pointer select-none"
          style="
            height: var(--h-button);
            padding: 0 10px;
            border-radius: var(--r-md);
            border: 1px solid var(--c-border);
            background: transparent;
            color: var(--c-foreground);
            font-family: var(--font-mono);
            font-size: var(--fs-sm);
            font-weight: var(--fw-medium);
          "
          @click="copyError"
        >
          <Icon :name="copied ? 'check' : 'copy'" :size="14" />
          {{ copied ? 'Copied' : 'Copy error' }}
        </button>
      </div>
    </div>
  </div>

  <div
    v-else
    data-state="error"
    class="flex flex-col items-center justify-center text-center"
    style="gap: 12px; padding: 24px; flex: 1; min-height: 0;"
  >
    <div
      class="text-left"
      style="
        width: 340px;
        max-width: 100%;
        background-color: var(--c-banner-err-bg);
        border: 1px solid rgba(240, 113, 120, 0.5);
        border-radius: var(--r-md);
        padding: 11px 13px;
      "
    >
      <div
        class="flex items-center"
        style="gap: 8px; font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-banner-err-fg); margin-bottom: 5px;"
      >
        <Icon name="triangle-alert" :size="15" />
        {{ title }}
      </div>
      <div
        style="font-size: 12.5px; color: var(--c-banner-err-fg); opacity: 0.92; line-height: 1.45; margin-bottom: 7px;"
      >
        {{ message }}
      </div>
      <div
        v-if="diagnostics"
        style="font-size: var(--fs-xs); font-family: var(--font-mono); color: var(--c-banner-err-fg); opacity: 0.7;"
      >
        {{ diagnostics }}
      </div>
    </div>

    <div class="flex" style="gap: 8px;">
      <button
        type="button"
        data-action="retry"
        class="inline-flex items-center justify-center gap-1 cursor-pointer select-none"
        style="
          height: var(--h-button);
          padding: 0 10px;
          border-radius: var(--r-md);
          border: 1px solid transparent;
          background-color: var(--c-primary);
          color: var(--c-primary-fg);
          font-family: var(--font-mono);
          font-size: var(--fs-sm);
          font-weight: var(--fw-medium);
        "
        @click="emit('retry')"
      >
        <Icon name="refresh-cw" :size="14" />
        Retry
      </button>
      <button
        v-if="requestId"
        type="button"
        data-action="copy"
        class="inline-flex items-center justify-center gap-1 cursor-pointer select-none"
        style="
          height: var(--h-button);
          padding: 0 10px;
          border-radius: var(--r-md);
          border: 1px solid var(--c-border);
          background: transparent;
          color: var(--c-foreground);
          font-family: var(--font-mono);
          font-size: var(--fs-sm);
          font-weight: var(--fw-medium);
        "
        @click="copyError"
      >
        <Icon :name="copied ? 'check' : 'copy'" :size="14" />
        {{ copied ? 'Copied' : 'Copy error' }}
      </button>
    </div>
  </div>
</template>
