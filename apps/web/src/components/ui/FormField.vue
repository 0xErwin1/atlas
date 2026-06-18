<script setup lang="ts">
import { computed, ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';

const props = withDefaults(
  defineProps<{
    label?: string;
    modelValue: string;
    type?: 'text' | 'password' | 'email';
    placeholder?: string;
    /** Inline validation message; when set the field renders in the error state. */
    error?: string | null;
    /** Non-error helper text shown under the field. */
    helper?: string;
    autocomplete?: string;
    id?: string;
    mono?: boolean;
    disabled?: boolean;
  }>(),
  {
    label: '',
    type: 'text',
    placeholder: '',
    error: null,
    helper: '',
    autocomplete: undefined,
    id: undefined,
    mono: false,
    disabled: false,
  },
);

const emit = defineEmits<{
  'update:modelValue': [value: string];
  blur: [];
  keydown: [event: KeyboardEvent];
}>();

const showPassword = ref(false);

const isPassword = computed(() => props.type === 'password');

const inputType = computed(() => {
  if (isPassword.value) return showPassword.value ? 'text' : 'password';
  return props.type;
});

function onInput(event: Event) {
  emit('update:modelValue', (event.target as HTMLInputElement).value);
}
</script>

<template>
  <div class="atl-field">
    <label v-if="label" :for="id" class="atl-field-label">{{ label }}</label>

    <div
      class="atl-field-box"
      :style="{ borderColor: error ? 'var(--c-danger)' : 'var(--c-border)' }"
    >
      <input
        :id="id"
        :value="modelValue"
        :type="inputType"
        :placeholder="placeholder"
        :autocomplete="autocomplete"
        :disabled="disabled"
        class="atl-field-input"
        :style="{ fontFamily: mono ? 'var(--font-mono)' : 'var(--font-ui)' }"
        :aria-invalid="error ? 'true' : undefined"
        @input="onInput"
        @blur="emit('blur')"
        @keydown="emit('keydown', $event)"
      />
      <button
        v-if="isPassword"
        type="button"
        tabindex="-1"
        class="atl-field-eye"
        :aria-label="showPassword ? 'Hide password' : 'Show password'"
        @click="showPassword = !showPassword"
      >
        <Icon :name="showPassword ? 'eye-off' : 'eye'" :size="14" />
      </button>
    </div>

    <div v-if="error" class="atl-field-error">
      <Icon name="triangle-alert" :size="12" />
      {{ error }}
    </div>
    <div v-else-if="helper" class="atl-field-helper">{{ helper }}</div>
  </div>
</template>

<style scoped>
.atl-field {
  display: flex;
  flex-direction: column;
}

.atl-field-label {
  display: block;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
  margin-bottom: 5px;
}

.atl-field-box {
  display: flex;
  align-items: center;
  gap: 8px;
  height: var(--h-input);
  padding: 0 4px 0 10px;
  background-color: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
}

.atl-field-input {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  color: var(--c-foreground);
  font-size: var(--fs-base);
}

.atl-field-input::placeholder {
  color: var(--c-muted);
}

.atl-field-input:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.atl-field-eye {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
}

.atl-field-error {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 11.5px;
  color: var(--c-danger);
  margin-top: 5px;
}

.atl-field-helper {
  font-size: 11.5px;
  color: var(--c-muted);
  margin-top: 5px;
}
</style>
