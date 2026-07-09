<script setup lang="ts">
// The single top bar (FL Studio / Bitwig model): text menus on the left,
// the transport cluster in the center, master meter and engine health on the
// right. One row — the transport never moves, whatever the zones do.

import { computed, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { invoke } from "@tauri-apps/api/core";

import MasterMeter from "./MasterMeter.vue";
import Icon from "../../ui/Icon.vue";
import { useProjectStore } from "../../stores/project";
import { useTelemetryStore } from "../../stores/telemetry";
import { useUiStore } from "../../stores/ui";
import { DEMO_PATH, EXPORT_PATH } from "../../projectPaths";

const { t, locale } = useI18n();
const project = useProjectStore();
const telemetry = useTelemetryStore();
const ui = useUiStore();

// --- Menus ---

type MenuId = "file" | "edit" | "view";
const openMenu = ref<MenuId | null>(null);
const barEl = ref<HTMLElement | null>(null);

function toggleMenu(id: MenuId): void {
  openMenu.value = openMenu.value === id ? null : id;
}

/** An open menu follows the pointer across the bar (standard menu behavior). */
function hoverMenu(id: MenuId): void {
  if (openMenu.value !== null) openMenu.value = id;
}

function closeMenus(): void {
  openMenu.value = null;
}

function onWindowDown(e: PointerEvent): void {
  if (openMenu.value && !barEl.value?.contains(e.target as Node)) closeMenus();
}

function onWindowKey(e: KeyboardEvent): void {
  if (e.key === "Escape") closeMenus();
}

const statusMsg = ref("");

async function run(action: () => Promise<void>, done = ""): Promise<void> {
  closeMenus();
  try {
    await action();
    statusMsg.value = done;
  } catch (e) {
    statusMsg.value = String(e);
  }
}

const exportWav = () =>
  run(
    () => invoke("export_wav", { projectPath: DEMO_PATH, outPath: EXPORT_PATH }),
    t("menu.exported"),
  );
const reloadProject = () => run(() => invoke("load_project", { path: DEMO_PATH }));
const editUndo = () =>
  run(async () => {
    await invoke("edit_undo");
    await project.load();
  });
const editRedo = () =>
  run(async () => {
    await invoke("edit_redo");
    await project.load();
  });

function setLocale(l: "fr" | "en"): void {
  locale.value = l;
  localStorage.setItem("telenn.locale", l);
  closeMenus();
}

// --- Transport ---

const playing = ref(false);

async function play(): Promise<void> {
  await invoke("transport_play");
  playing.value = true;
}

async function stop(): Promise<void> {
  await invoke("transport_stop");
  playing.value = false;
}

// Metronome: session monitoring state (never saved to the song, never
// exported). The volume slider only shows while the click is on.
const metronomeOn = ref(false);
const metronomeVolume = ref(0.7);

async function toggleMetronome(): Promise<void> {
  metronomeOn.value = !metronomeOn.value;
  await invoke("metronome_set", { enabled: metronomeOn.value, volume: metronomeVolume.value });
}

async function onMetronomeVolume(e: Event): Promise<void> {
  metronomeVolume.value = Number((e.target as HTMLInputElement).value);
  await invoke("metronome_set", { enabled: metronomeOn.value, volume: metronomeVolume.value });
}

// CPU load and xrun count, streamed from the audio thread via telemetry. The
// readout turns red past 80 % of the real-time budget (close to underrunning).
const cpuPercent = computed(() => Math.round(telemetry.audioLoad * 100));
const cpuHot = computed(() => telemetry.audioLoad > 0.8);

onMounted(() => {
  window.addEventListener("pointerdown", onWindowDown);
  window.addEventListener("keydown", onWindowKey);
});

onUnmounted(() => {
  window.removeEventListener("pointerdown", onWindowDown);
  window.removeEventListener("keydown", onWindowKey);
});
</script>

<template>
  <header ref="barEl" class="bar">
    <!-- Menus -->
    <nav class="menus">
      <div class="menu">
        <button
          type="button"
          class="menu-btn"
          :class="{ open: openMenu === 'file' }"
          @click="toggleMenu('file')"
          @pointerenter="hoverMenu('file')"
        >
          {{ t("menu.file") }}
        </button>
        <div v-if="openMenu === 'file'" class="dropdown">
          <button type="button" class="item" @click="exportWav">
            {{ t("menu.exportWav") }}
          </button>
          <button type="button" class="item" @click="reloadProject">
            {{ t("menu.reloadProject") }}
          </button>
        </div>
      </div>

      <div class="menu">
        <button
          type="button"
          class="menu-btn"
          :class="{ open: openMenu === 'edit' }"
          @click="toggleMenu('edit')"
          @pointerenter="hoverMenu('edit')"
        >
          {{ t("menu.edit") }}
        </button>
        <div v-if="openMenu === 'edit'" class="dropdown">
          <button type="button" class="item" @click="editUndo">
            {{ t("menu.undo") }} <kbd>Ctrl+Z</kbd>
          </button>
          <button type="button" class="item" @click="editRedo">
            {{ t("menu.redo") }} <kbd>Ctrl+Shift+Z</kbd>
          </button>
        </div>
      </div>

      <div class="menu">
        <button
          type="button"
          class="menu-btn"
          :class="{ open: openMenu === 'view' }"
          @click="toggleMenu('view')"
          @pointerenter="hoverMenu('view')"
        >
          {{ t("menu.view") }}
        </button>
        <div v-if="openMenu === 'view'" class="dropdown">
          <button type="button" class="item" @click="ui.toggleBrowser(); closeMenus()">
            <span class="check">{{ ui.layout.browser.open ? "✓" : "" }}</span>
            {{ t("view.browser") }} <kbd>Alt+B</kbd>
          </button>
          <button type="button" class="item" @click="ui.toggleBottom('chain'); closeMenus()">
            <span class="check">{{ ui.layout.bottom.open && ui.layout.bottom.tab === 'chain' ? "✓" : "" }}</span>
            {{ t("view.chain") }} <kbd>Alt+E</kbd>
          </button>
          <button type="button" class="item" @click="ui.toggleBottom('mixer'); closeMenus()">
            <span class="check">{{ ui.layout.bottom.open && ui.layout.bottom.tab === 'mixer' ? "✓" : "" }}</span>
            {{ t("view.mixer") }} <kbd>Alt+M</kbd>
          </button>
          <div class="separator"></div>
          <button type="button" class="item" @click="ui.toggleDev(); closeMenus()">
            <span class="check">{{ ui.layout.devOpen ? "✓" : "" }}</span>
            {{ t("menu.devOverlay") }} <kbd>Ctrl+Alt+D</kbd>
          </button>
          <div class="separator"></div>
          <button type="button" class="item" @click="setLocale('fr')">
            <span class="check">{{ locale === "fr" ? "✓" : "" }}</span> Français
          </button>
          <button type="button" class="item" @click="setLocale('en')">
            <span class="check">{{ locale === "en" ? "✓" : "" }}</span> English
          </button>
        </div>
      </div>
      <span v-if="statusMsg" class="status">{{ statusMsg }}</span>
    </nav>

    <!-- Transport (always centered) -->
    <div class="transport">
      <button
        type="button"
        class="tbtn"
        :class="{ active: playing }"
        :title="t('transport.play')"
        @click="play"
      >
        <Icon name="play" :size="15" />
      </button>
      <button type="button" class="tbtn" :title="t('transport.stop')" @click="stop">
        <Icon name="stop" :size="15" />
      </button>
      <button
        type="button"
        class="tbtn"
        :class="{ active: metronomeOn }"
        :title="t('transport.metronome')"
        @click="toggleMetronome"
      >
        <Icon name="metronome" :size="15" />
      </button>
      <input
        v-if="metronomeOn"
        class="metronome-volume"
        type="range"
        min="0"
        max="1"
        step="0.01"
        :value="metronomeVolume"
        @input="onMetronomeVolume($event)"
      />
      <span class="bpm num">{{ project.bpm }} BPM</span>
    </div>

    <!-- Health -->
    <div class="health">
      <MasterMeter />
      <span class="engine-load num" :class="{ hot: cpuHot }">
        {{ t("meter.cpu") }} {{ cpuPercent }}% · {{ t("meter.xruns") }} {{ telemetry.xruns }}
      </span>
    </div>
  </header>
</template>

<style scoped>
.bar {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  gap: 0.8rem;
  padding: 0.3rem 0.8rem;
  background: var(--panel);
  border-bottom: 1px solid var(--border);
}

/* --- Menus --- */
.menus {
  display: flex;
  align-items: center;
  gap: 0.1rem;
  justify-self: start;
}

.menu {
  position: relative;
}

.menu-btn {
  display: flex;
  align-items: center;
  height: 1.9rem; /* same height as the transport buttons: one shared axis */
  border: none;
  background: none;
  padding: 0 0.55rem;
  border-radius: var(--radius-sm);
  color: var(--text-2);
  font-family: var(--font-display);
  font-size: 0.8rem;
  line-height: 1;
  cursor: pointer;
  transition: color var(--t-fast), background-color var(--t-fast);
}

.menu-btn:hover,
.menu-btn.open {
  color: var(--text);
  background: var(--surface);
}

.dropdown {
  position: absolute;
  top: 100%;
  left: 0;
  z-index: 100;
  min-width: 230px;
  padding: 0.25rem;
  background: var(--surface);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-overlay);
}

.item {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  width: 100%;
  border: none;
  background: none;
  padding: 0.35rem 0.5rem;
  border-radius: var(--radius-sm);
  color: var(--text);
  font-size: 0.8rem;
  text-align: left;
  cursor: pointer;
}

.item:hover {
  background: var(--accent-dim);
}

.item kbd {
  margin-left: auto;
  font-family: var(--font-mono);
  font-size: 0.65rem;
  color: var(--text-3);
}

.check {
  width: 0.9rem;
  flex: none;
  color: var(--accent);
}

.separator {
  height: 1px;
  margin: 0.25rem 0.4rem;
  background: var(--border);
}

.status {
  margin-left: 0.5rem;
  font-size: 0.72rem;
  color: var(--text-3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* --- Transport --- */
.transport {
  display: flex;
  align-items: center;
  gap: 0.4rem;
}

.tbtn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2.1rem;
  height: 1.9rem;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--text);
  cursor: pointer;
  transition: background-color var(--t-fast), border-color var(--t-fast), color var(--t-fast);
}

.tbtn:hover {
  background: var(--surface-2);
}

.tbtn.active {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--bg);
}

.metronome-volume {
  width: 70px;
  accent-color: var(--accent);
  cursor: pointer;
}

.bpm {
  margin-left: 0.5rem;
  font-size: 0.8rem;
  color: var(--text-2);
}

/* --- Health --- */
.health {
  display: flex;
  align-items: center;
  gap: 0.8rem;
  justify-self: end;
}

.engine-load {
  font-size: 0.72rem;
  color: var(--text-2);
}

.engine-load.hot {
  color: var(--danger);
}
</style>
