// Toast store: a small queue of transient notifications that auto-expire.
// Anything user-facing that would otherwise be invisible lands here — an
// export finishing, a watcher parse error. Discreet by design: a toast never
// asks anything, and clicking one dismisses it early.

import { ref } from "vue";
import { defineStore } from "pinia";

export type ToastKind = "ok" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  text: string;
  /** Optional raw detail under the message (e.g. a parser error, verbatim). */
  detail?: string;
}

/** Display time by kind: errors linger, confirmations get out of the way. */
export const TOAST_MS: Record<ToastKind, number> = { ok: 3500, error: 8000 };

export const useToastStore = defineStore("toasts", () => {
  const toasts = ref<Toast[]>([]);
  let nextId = 1;

  function dismiss(id: number): void {
    const i = toasts.value.findIndex((toast) => toast.id === id);
    if (i !== -1) toasts.value.splice(i, 1);
  }

  function push(kind: ToastKind, text: string, detail?: string): void {
    const id = nextId;
    nextId += 1;
    toasts.value.push({ id, kind, text, detail });
    setTimeout(() => dismiss(id), TOAST_MS[kind]);
  }

  return { toasts, push, dismiss };
});
