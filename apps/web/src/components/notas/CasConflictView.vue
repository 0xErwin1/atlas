<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import type { ConflictHunk, MergeSegment } from '@/composables/useCasMerge';

const props = defineProps<{
  open: boolean;
  segments: MergeSegment[];
}>();

const emit = defineEmits<{
  resolve: [content: string];
  cancel: [];
}>();

type Side = 'mine' | 'theirs';

interface ConflictState {
  /** Index of this conflict within `props.segments`. */
  segmentIndex: number;
  hunk: ConflictHunk;
  /** The side last picked, for highlighting; null until the user chooses. */
  picked: Side | null;
  /** The current resolved text (a picked side or a manual edit). */
  value: string;
  /** True once the user has made any explicit choice (pick or edit). */
  resolved: boolean;
}

const conflicts = ref<ConflictState[]>([]);

watch(
  () => [props.open, props.segments] as const,
  () => {
    if (!props.open) return;
    conflicts.value = props.segments
      .map((seg, index) => ({ seg, index }))
      .filter(
        (e): e is { seg: Extract<MergeSegment, { kind: 'conflict' }>; index: number } =>
          e.seg.kind === 'conflict',
      )
      .map(({ seg, index }) => ({
        segmentIndex: index,
        hunk: seg.hunk,
        picked: null,
        value: seg.hunk.mine,
        resolved: false,
      }));
  },
  { immediate: true },
);

const allResolved = computed(() => conflicts.value.length > 0 && conflicts.value.every((c) => c.resolved));

function pick(conflictPos: number, side: Side): void {
  const state = conflicts.value[conflictPos];
  if (state === undefined) return;
  state.picked = side;
  state.value = side === 'mine' ? state.hunk.mine : state.hunk.theirs;
  state.resolved = true;
}

function onEdit(conflictPos: number, event: Event): void {
  const state = conflicts.value[conflictPos];
  if (state === undefined) return;
  const target = event.target as HTMLTextAreaElement;
  state.value = target.value;
  state.picked = null;
  state.resolved = true;
}

function resolve(): void {
  if (!allResolved.value) return;

  const resolvedByIndex = new Map<number, string>();
  for (const c of conflicts.value) {
    resolvedByIndex.set(c.segmentIndex, c.value);
  }

  const content = props.segments
    .map((seg, index) => (seg.kind === 'stable' ? seg.text : (resolvedByIndex.get(index) ?? seg.hunk.mine)))
    .join('\n');

  emit('resolve', content);
}

function cancel(): void {
  emit('cancel');
}
</script>

<template>
  <div
    v-if="open"
    role="dialog"
    aria-modal="true"
    aria-label="Resolve edit conflict"
    class="fixed inset-0 flex items-center justify-center"
    style="z-index: 100; background: var(--c-overlay); padding: 24px;"
    @click.self="cancel"
  >
    <div
      class="flex flex-col min-h-0"
      style="
        width: 640px;
        max-width: 100%;
        max-height: 100%;
        background: var(--c-panel);
        border: 1px solid var(--c-border);
        border-radius: var(--r-lg);
        box-shadow: var(--shadow-lg);
        overflow: hidden;
      "
    >
      <header
        class="flex items-center gap-2 shrink-0"
        style="height: 40px; padding: 0 14px; border-bottom: 1px solid var(--c-border);"
      >
        <Icon name="triangle-alert" :size="15" :style="{ color: 'var(--c-warning)' }" />
        <span style="font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-foreground);">
          Resolve edit conflict
        </span>
        <span
          style="margin-left: auto; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);"
        >
          {{ conflicts.length }} {{ conflicts.length === 1 ? 'conflict' : 'conflicts' }}
        </span>
      </header>

      <p
        class="shrink-0"
        style="padding: 10px 14px 0; font-size: var(--fs-sm); color: var(--c-muted); line-height: var(--lh-normal);"
      >
        This document changed on the server while you were editing. Pick a side or edit each
        region — nothing is overwritten until you resolve every conflict.
      </p>

      <div class="flex-1 overflow-y-auto" style="padding: 12px 14px; display: flex; flex-direction: column; gap: 14px;">
        <section
          v-for="(c, pos) in conflicts"
          :key="c.segmentIndex"
          style="border: 1px solid var(--c-border); border-radius: var(--r-md); background: var(--c-raised); overflow: hidden;"
        >
          <div class="flex" style="border-bottom: 1px solid var(--c-border);">
            <button
              type="button"
              :data-test="`pick-mine-${pos}`"
              class="flex-1 flex items-center justify-between gap-2 cursor-pointer"
              :style="`
                padding: 7px 10px;
                background: ${c.picked === 'mine' ? 'var(--c-selection)' : 'transparent'};
                border: none;
                border-right: 1px solid var(--c-border);
                color: var(--c-foreground);
                font-family: var(--font-mono);
                font-size: var(--fs-xs);
                font-weight: var(--fw-semibold);
              `"
              @click="pick(pos, 'mine')"
            >
              <span>YOURS</span>
              <Icon
                v-if="c.picked === 'mine'"
                name="check"
                :size="13"
                :style="{ color: 'var(--c-primary)' }"
              />
            </button>
            <button
              type="button"
              :data-test="`pick-theirs-${pos}`"
              class="flex-1 flex items-center justify-between gap-2 cursor-pointer"
              :style="`
                padding: 7px 10px;
                background: ${c.picked === 'theirs' ? 'var(--c-selection)' : 'transparent'};
                border: none;
                color: var(--c-foreground);
                font-family: var(--font-mono);
                font-size: var(--fs-xs);
                font-weight: var(--fw-semibold);
              `"
              @click="pick(pos, 'theirs')"
            >
              <span>SERVER</span>
              <Icon
                v-if="c.picked === 'theirs'"
                name="check"
                :size="13"
                :style="{ color: 'var(--c-primary)' }"
              />
            </button>
          </div>

          <div class="grid" style="grid-template-columns: 1fr 1fr;">
            <pre
              style="
                margin: 0;
                padding: 9px 10px;
                border-right: 1px solid var(--c-border);
                font-family: var(--font-mono);
                font-size: var(--fs-sm);
                line-height: var(--lh-normal);
                color: var(--c-foreground);
                white-space: pre-wrap;
                word-break: break-word;
              "
            >{{ c.hunk.mine }}</pre>
            <pre
              style="
                margin: 0;
                padding: 9px 10px;
                font-family: var(--font-mono);
                font-size: var(--fs-sm);
                line-height: var(--lh-normal);
                color: var(--c-foreground);
                white-space: pre-wrap;
                word-break: break-word;
              "
            >{{ c.hunk.theirs }}</pre>
          </div>

          <div style="border-top: 1px solid var(--c-border); padding: 8px 10px;">
            <label
              style="display: block; font-size: var(--fs-xs); color: var(--c-muted); margin-bottom: 4px; font-family: var(--font-mono);"
            >
              Resolution
            </label>
            <textarea
              :data-test="`edit-${pos}`"
              :value="c.value"
              rows="2"
              spellcheck="false"
              style="
                width: 100%;
                resize: vertical;
                padding: 7px 9px;
                background: var(--c-input);
                border: 1px solid var(--c-border);
                border-radius: var(--r-md);
                color: var(--c-foreground);
                font-family: var(--font-mono);
                font-size: var(--fs-sm);
                line-height: var(--lh-normal);
              "
              @input="onEdit(pos, $event)"
            />
          </div>
        </section>
      </div>

      <footer
        class="flex items-center gap-2 shrink-0"
        style="height: 48px; padding: 0 14px; border-top: 1px solid var(--c-border);"
      >
        <span
          v-if="!allResolved"
          style="font-size: var(--fs-xs); color: var(--c-muted); font-family: var(--font-mono);"
        >
          Resolve every region to continue
        </span>
        <div class="flex items-center gap-2" style="margin-left: auto;">
          <Btn variant="ghost" data-test="cancel" @click="cancel">Cancel</Btn>
          <Btn variant="primary" data-test="resolve" :disabled="!allResolved" @click="resolve">
            Resolve and save
          </Btn>
        </div>
      </footer>
    </div>
  </div>
</template>
