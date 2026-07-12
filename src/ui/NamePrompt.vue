<script setup lang="ts">
// Small popover asking for a name (preset saves). The parent positions it
// (position: fixed at the drop point) and owns what a confirmed name means.
// Enter confirms (an empty name is refused), Escape or a click outside cancels.

import { onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";

const props = defineProps<{ initial?: string }>();
const emit = defineEmits<{ confirm: [name: string]; close: [] }>();

const { t } = useI18n();
const name = ref(props.initial ?? "");
const rootEl = ref<HTMLElement | null>(null);
const inputEl = ref<HTMLInputElement | null>(null);

function onWindowDown(e: PointerEvent): void {
  if (!rootEl.value?.contains(e.target as Node)) emit("close");
}

function onWindowKey(e: KeyboardEvent): void {
  if (e.key === "Escape") emit("close");
}

function confirm(): void {
  const trimmed = name.value.trim();
  if (trimmed) emit("confirm", trimmed);
}

onMounted(() => {
  inputEl.value?.focus();
  inputEl.value?.select(); // the suggested name is one keystroke from replaced
  // Deferred so the opening pointerup/click does not immediately close it.
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
  <div ref="rootEl" class="name-prompt">
    <input
      ref="inputEl"
      v-model="name"
      type="text"
      :placeholder="t('prompt.presetName')"
      spellcheck="false"
      @keydown.enter="confirm"
    />
  </div>
</template>

<style scoped>
.name-prompt {
  position: absolute;
  z-index: 150;
  padding: 0.25rem;
  background: var(--surface);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-overlay);
}

.name-prompt input {
  width: 11rem;
  padding: 0.3rem 0.5rem;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-size: 0.8rem;
}

.name-prompt input:focus {
  outline: none;
  border-color: var(--accent);
}

.name-prompt input::placeholder {
  color: var(--text-3);
}
</style>
