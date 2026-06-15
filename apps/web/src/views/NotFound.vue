<script setup lang="ts">
import { useRouter } from 'vue-router';

const router = useRouter();
</script>

<template>
  <div
    class="flex flex-col items-center justify-center"
    style="height: 100vh; background-color: var(--c-background); color: var(--c-foreground);"
  >
    <p style="font-family: var(--font-mono); font-size: var(--fs-xl); color: var(--c-muted);">
      404
    </p>
    <p style="font-size: var(--fs-sm); color: var(--c-muted); margin-top: 8px;">Page not found</p>
    <button
      type="button"
      style="
        margin-top: 16px;
        padding: 0 10px;
        height: var(--h-button);
        border-radius: var(--r-md);
        background-color: var(--c-raised);
        border: 1px solid var(--c-border);
        color: var(--c-foreground);
        font-family: var(--font-mono);
        font-size: var(--fs-sm);
        cursor: pointer;
      "
      @click="router.back()"
    >
      Go back
    </button>
  </div>
</template>
