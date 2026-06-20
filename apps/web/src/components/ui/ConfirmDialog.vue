<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue';
import type { BtnVariant } from '@/components/ui/Btn.vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';

export type ConfirmTone = 'danger' | 'warning' | 'primary';

/**
 * A blocking confirmation modal. The parent owns visibility via `open` and reacts
 * to `confirm` / `cancel`. Escape, backdrop, and the close button all cancel, so a
 * dismissal never reads as a confirmation.
 *
 * Verbose by design: beyond the title + consequence (`message`), it can show the
 * exact affected resource (`detail`, mono) and the secondary fallout (`note`),
 * tone-colored to signal destructive (`danger`) vs reversible-but-lossy
 * (`warning`) actions.
 */
const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    message?: string;
    detail?: string;
    detailIcon?: string;
    note?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    confirmIcon?: string;
    icon?: string;
    tone?: ConfirmTone;
    /** @deprecated prefer `tone="danger"`; kept for existing call sites. */
    danger?: boolean;
    width?: number;
  }>(),
  {
    confirmLabel: 'Confirm',
    cancelLabel: 'Cancel',
    detailIcon: 'file',
    danger: false,
    width: 440,
  },
);

const emit = defineEmits<{
  confirm: [];
  cancel: [];
}>();

interface ToneStyle {
  accent: string;
  wash: string;
  border: string;
  variant: BtnVariant;
  icon: string;
}

const TONE: Record<ConfirmTone, ToneStyle> = {
  danger: {
    accent: 'var(--c-danger)',
    wash: 'rgba(240,113,120,0.12)',
    border: 'rgba(240,113,120,0.45)',
    variant: 'danger',
    icon: 'trash-2',
  },
  warning: {
    accent: 'var(--c-primary)',
    wash: 'rgba(255,180,84,0.12)',
    border: 'rgba(255,180,84,0.45)',
    variant: 'primary',
    icon: 'triangle-alert',
  },
  primary: {
    accent: 'var(--c-info)',
    wash: 'rgba(89,194,255,0.12)',
    border: 'rgba(89,194,255,0.45)',
    variant: 'primary',
    icon: 'info',
  },
};

const tone = computed<ConfirmTone>(() => props.tone ?? (props.danger ? 'danger' : 'primary'));
const toneStyle = computed<ToneStyle>(() => TONE[tone.value]);
const badgeIcon = computed(() => props.icon ?? toneStyle.value.icon);

function onKeydown(event: KeyboardEvent): void {
  if (props.open && event.key === 'Escape') emit('cancel');
}

onMounted(() => window.addEventListener('keydown', onKeydown));
onUnmounted(() => window.removeEventListener('keydown', onKeydown));
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 flex items-center justify-center"
      style="background: rgba(7, 10, 15, 0.66); z-index: 300; padding: 24px;"
      @mousedown.self="emit('cancel')"
    >
      <div
        role="dialog"
        aria-modal="true"
        :style="{
          width: `${width}px`,
          maxWidth: '100%',
          background: 'var(--c-raised)',
          border: '1px solid var(--c-border)',
          borderRadius: '4px',
          boxShadow: 'var(--shadow-lg)',
          overflow: 'hidden',
          fontFamily: 'var(--font-ui)',
        }"
      >
        <div class="flex items-start" style="gap: 12px; padding: 16px 16px 0;">
          <span
            class="flex items-center justify-center shrink-0"
            :style="{
              width: '32px',
              height: '32px',
              borderRadius: 'var(--r-md)',
              color: toneStyle.accent,
              background: toneStyle.wash,
              border: `1px solid ${toneStyle.border}`,
            }"
          >
            <Icon :name="badgeIcon" :size="17" />
          </span>

          <div class="flex-1 min-w-0" style="padding-top: 1px;">
            <h2 style="font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-foreground); margin: 0;">
              {{ title }}
            </h2>
            <p
              v-if="message"
              style="font-size: 12.5px; line-height: 1.5; color: var(--c-muted); margin: 5px 0 0;"
            >
              {{ message }}
            </p>
          </div>

          <button
            type="button"
            data-test="close"
            class="inline-flex items-center justify-center shrink-0 cursor-pointer"
            style="width: 22px; height: 22px; border: none; background: transparent; color: var(--c-muted); border-radius: var(--r-sm);"
            title="Close"
            aria-label="Close"
            @click="emit('cancel')"
          >
            <Icon name="x" :size="14" />
          </button>
        </div>

        <div
          v-if="detail"
          data-test="detail"
          class="flex items-center"
          style="margin: 12px 16px 0; padding: 7px 10px; gap: 8px; background: var(--c-background); border: 1px solid var(--c-border); border-radius: var(--r-md); font-family: var(--font-mono); font-size: 11.5px; color: var(--c-foreground); white-space: nowrap; overflow: hidden;"
        >
          <Icon :name="detailIcon" :size="13" style="color: var(--c-muted); flex: 0 0 auto;" />
          <span style="flex: 1; overflow: hidden; text-overflow: ellipsis;">{{ detail }}</span>
        </div>

        <div
          v-if="note"
          data-test="note"
          class="flex"
          style="gap: 8px; margin: 12px 16px 0; font-size: var(--fs-sm); line-height: 1.5; color: var(--c-muted);"
        >
          <Icon name="triangle-alert" :size="14" :style="{ color: toneStyle.accent, flex: '0 0 auto', marginTop: '1px' }" />
          <span>{{ note }}</span>
        </div>

        <div class="flex justify-end" style="gap: 8px; padding: 18px 16px 16px;">
          <Btn variant="secondary" data-test="cancel" @click="emit('cancel')">{{ cancelLabel }}</Btn>
          <Btn :variant="toneStyle.variant" data-test="confirm" @click="emit('confirm')">
            <Icon v-if="confirmIcon" :name="confirmIcon" :size="14" />
            {{ confirmLabel }}
          </Btn>
        </div>
      </div>
    </div>
  </Teleport>
</template>
