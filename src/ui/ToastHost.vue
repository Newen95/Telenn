<script setup lang="ts">
// Renders the toast queue as a quiet stack in the bottom-right corner. State
// lives in the toast store; this component only shows it. Click = dismiss.

import { useToastStore } from "../stores/toasts";

const toasts = useToastStore();
</script>

<template>
  <div class="toasts">
    <TransitionGroup name="toast">
      <div
        v-for="toast in toasts.toasts"
        :key="toast.id"
        class="toast"
        :class="toast.kind"
        role="status"
        @click="toasts.dismiss(toast.id)"
      >
        <span class="text">{{ toast.text }}</span>
        <span v-if="toast.detail" class="detail num">{{ toast.detail }}</span>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.toasts {
  position: fixed;
  right: 12px;
  bottom: 12px;
  z-index: 1200;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 6px;
  pointer-events: none;
}

.toast {
  pointer-events: auto;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  max-width: 360px;
  padding: 0.45rem 0.7rem;
  background: var(--surface);
  border: 1px solid var(--border-strong);
  border-left-width: 3px;
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-overlay);
  cursor: pointer;
}

.toast.ok {
  border-left-color: var(--ok);
}

.toast.error {
  border-left-color: var(--danger);
}

.text {
  font-size: 0.78rem;
  color: var(--text);
}

.detail {
  font-size: 0.68rem;
  color: var(--text-2);
  overflow-wrap: anywhere;
}

/* State-driven motion: a toast slides in from below and fades away; the stack
   above it settles via the move transition. */
.toast-enter-active,
.toast-leave-active,
.toast-move {
  transition: opacity var(--t-med), transform var(--t-med);
}

.toast-enter-from {
  opacity: 0;
  transform: translateY(8px);
}

.toast-leave-to {
  opacity: 0;
}

.toast-leave-active {
  position: absolute;
}
</style>
