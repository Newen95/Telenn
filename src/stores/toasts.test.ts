import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";

import { TOAST_MS, useToastStore } from "./toasts";

describe("toast store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("queues toasts and auto-expires them", () => {
    const store = useToastStore();
    store.push("ok", "done");
    expect(store.toasts).toHaveLength(1);
    expect(store.toasts[0]).toMatchObject({ kind: "ok", text: "done" });
    vi.advanceTimersByTime(TOAST_MS.ok);
    expect(store.toasts).toHaveLength(0);
  });

  it("keeps errors longer than confirmations", () => {
    const store = useToastStore();
    store.push("ok", "done");
    store.push("error", "failed", "raw detail");
    vi.advanceTimersByTime(TOAST_MS.ok);
    expect(store.toasts).toHaveLength(1);
    expect(store.toasts[0]).toMatchObject({ kind: "error", detail: "raw detail" });
    vi.advanceTimersByTime(TOAST_MS.error - TOAST_MS.ok);
    expect(store.toasts).toHaveLength(0);
  });

  it("dismisses on demand; the late expiry of a dismissed toast is a no-op", () => {
    const store = useToastStore();
    store.push("ok", "first");
    store.push("ok", "second");
    store.dismiss(store.toasts[0].id);
    expect(store.toasts.map((t) => t.text)).toEqual(["second"]);
    vi.advanceTimersByTime(TOAST_MS.ok);
    expect(store.toasts).toHaveLength(0);
  });
});
