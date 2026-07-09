<script setup lang="ts">
// Hand-drawn inline icon set: one consistent 16×16 grid, 1.5px stroke,
// currentColor — icons inherit the text color of their context. Replaces the
// scattered unicode glyphs. Add a name here rather than importing an icon lib.

export type IconName =
  | "play"
  | "stop"
  | "metronome"
  | "chevron-down"
  | "chevron-left"
  | "chevron-right"
  | "close"
  | "plus";

withDefaults(defineProps<{ name: IconName; size?: number }>(), { size: 16 });
</script>

<template>
  <svg
    class="icon"
    :width="size"
    :height="size"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path v-if="name === 'play'" d="M4.5 2.8 L13 8 L4.5 13.2 Z" fill="currentColor" stroke="none" />
    <rect v-else-if="name === 'stop'" x="4" y="4" width="8" height="8" rx="1" fill="currentColor" stroke="none" />
    <template v-else-if="name === 'metronome'">
      <path d="M6.2 2.5 h3.6 l2.4 11 h-8.4 Z" />
      <path d="M8 10.5 L11.5 4.5" />
    </template>
    <path v-else-if="name === 'chevron-down'" d="M4 6.5 L8 10.5 L12 6.5" />
    <path v-else-if="name === 'chevron-left'" d="M9.5 4 L5.5 8 L9.5 12" />
    <path v-else-if="name === 'chevron-right'" d="M6.5 4 L10.5 8 L6.5 12" />
    <template v-else-if="name === 'close'">
      <path d="M4.5 4.5 L11.5 11.5" />
      <path d="M11.5 4.5 L4.5 11.5" />
    </template>
    <template v-else-if="name === 'plus'">
      <path d="M8 3.5 V12.5" />
      <path d="M3.5 8 H12.5" />
    </template>
  </svg>
</template>

<style scoped>
.icon {
  display: block;
  flex: none;
}
</style>
