// UI store: the active track (shared by timeline, piano roll, chain and
// mixer) and the zoned-shell layout, persisted to localStorage so the app
// reopens the way it was left. Project data never lives here — song.json
// stays the single source of truth for musical state.

import { reactive, ref, watch } from "vue";
import { defineStore } from "pinia";

import {
  defaultLayout,
  loadLayout,
  LAYOUT_KEY,
  type BottomTab,
  type UiLayout,
} from "./uiLayout";

/** Debounce for layout writes: a splitter drag emits many sizes per second. */
const SAVE_DELAY_MS = 300;

export const useUiStore = defineStore("ui", () => {
  /** The track every per-track view (piano roll, chain, mixer) follows. */
  const activeTrackId = ref("");

  const layout = reactive<UiLayout>(defaultLayout());

  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  function init(storage: Pick<Storage, "getItem" | "setItem"> = localStorage): void {
    Object.assign(layout, loadLayout(storage.getItem(LAYOUT_KEY)));
    watch(
      layout,
      () => {
        if (saveTimer !== null) clearTimeout(saveTimer);
        saveTimer = setTimeout(() => {
          saveTimer = null;
          const { devOpen: _dev, ...persisted } = layout;
          storage.setItem(LAYOUT_KEY, JSON.stringify(persisted));
        }, SAVE_DELAY_MS);
      },
      { deep: true },
    );
  }

  function toggleBrowser(): void {
    layout.browser.open = !layout.browser.open;
  }

  /** Toggle the bottom zone; with a tab, open on (or switch to) that tab. */
  function toggleBottom(tab?: BottomTab): void {
    if (tab && (!layout.bottom.open || layout.bottom.tab !== tab)) {
      layout.bottom.open = true;
      layout.bottom.tab = tab;
      return;
    }
    layout.bottom.open = !layout.bottom.open;
  }

  function toggleDev(): void {
    layout.devOpen = !layout.devOpen;
  }

  return {
    activeTrackId,
    layout,
    init,
    toggleBrowser,
    toggleBottom,
    toggleDev,
  };
});
