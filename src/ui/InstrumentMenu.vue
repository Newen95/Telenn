<script setup lang="ts">
// Small popover listing the registered instruments. Used by the timeline's
// "+ track" button (pick = create a track) and by the chain's instrument card
// (pick = replace the instrument). The parent positions it (wrap it in a
// `position: relative` box) and owns what a pick means.

import { onMounted, onUnmounted, ref } from "vue";

import { useProjectStore, type ModuleDesc } from "../stores/project";

const emit = defineEmits<{ pick: [module: ModuleDesc]; close: [] }>();

const project = useProjectStore();
const instruments = ref<ModuleDesc[]>([]);
const rootEl = ref<HTMLElement | null>(null);

function onWindowDown(e: PointerEvent): void {
  if (!rootEl.value?.contains(e.target as Node)) emit("close");
}

function onWindowKey(e: KeyboardEvent): void {
  if (e.key === "Escape") emit("close");
}

onMounted(async () => {
  instruments.value = (await project.listModules()).filter((m) => m.category === "instrument");
  // Deferred so the opening click does not immediately close the menu.
  setTimeout(() => {
    window.addEventListener("pointerdown", onWindowDown);
    window.addEventListener("keydown", onWindowKey);
  });
});

onUnmounted(() => {
  window.removeEventListener("pointerdown", onWindowDown);
  window.removeEventListener("keydown", onWindowKey);
});
</script>

<template>
  <div ref="rootEl" class="instrument-menu">
    <button
      v-for="m in instruments"
      :key="m.id"
      type="button"
      class="item"
      @click="emit('pick', m)"
    >
      {{ m.name }}
    </button>
  </div>
</template>

<style scoped>
.instrument-menu {
  position: absolute;
  z-index: 150;
  min-width: 160px;
  padding: 0.25rem;
  background: var(--surface);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-overlay);
}

.item {
  display: block;
  width: 100%;
  border: none;
  background: none;
  padding: 0.35rem 0.5rem;
  border-radius: var(--radius-sm);
  color: var(--text);
  font-size: 0.8rem;
  text-align: left;
  cursor: pointer;
  border-left: 2px solid var(--sig-notes);
  transition: background-color var(--t-fast);
}

.item:hover {
  background: var(--accent-dim);
}
</style>
