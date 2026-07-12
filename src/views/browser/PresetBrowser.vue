<script setup lang="ts">
// Library browser: two families mirroring the signal types, with a search
// field filtering both. INSTRUMENTS (Notes magenta) = the raw engines plus the
// saved chain presets (named instrument + effects, dashed); EFFETS (Audio
// teal) = the effect modules plus the saved effect presets (dashed). Items
// are dragged from here into a track's chain (insert), onto a chain head
// (replace instrument / whole chain) or the new-track zone. The whole browser
// is also the save drop zone: dragging an effect node or an instrument card
// over it highlights the family it will land in (the App controller owns the
// pointer plumbing; here we only mark the zone).

import { computed, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type ModuleDesc, type Preset } from "../../stores/project";
import { useChainDrag } from "../chain/dragdrop";
import Icon from "../../ui/Icon.vue";

interface NamedPreset {
  name: string;
  module: string;
  params: Record<string, number>;
}

interface NamedChainPreset {
  name: string;
  nodes: Preset[];
}

const { t } = useI18n();
const project = useProjectStore();
const drag = useChainDrag();

const instruments = ref<ModuleDesc[]>([]);
const effects = ref<ModuleDesc[]>([]);
const presets = ref<NamedPreset[]>([]);
const chainPresets = ref<NamedChainPreset[]>([]);

const search = ref("");
const open = reactive({ instruments: true, effects: true });

const match = (name: string): boolean =>
  name.toLowerCase().includes(search.value.trim().toLowerCase());

const shownInstruments = computed(() => instruments.value.filter((m) => match(m.name)));
const shownEffects = computed(() => effects.value.filter((m) => match(m.name)));
const shownPresets = computed(() => presets.value.filter((p) => match(p.name)));
const shownChainPresets = computed(() => chainPresets.value.filter((p) => match(p.name)));

async function reloadPresets(): Promise<void> {
  const names = await project.listPresets();
  presets.value = await Promise.all(
    names.map(async (name) => {
      const p = await project.readPreset(name);
      return { name, module: p.module, params: p.params };
    }),
  );
  const chainNames = await project.listChainPresets();
  chainPresets.value = await Promise.all(
    chainNames.map(async (name) => ({
      name,
      nodes: (await project.readChainPreset(name)).nodes,
    })),
  );
}

onMounted(async () => {
  const modules = await project.listModules();
  instruments.value = modules.filter((m) => m.category === "instrument");
  effects.value = modules.filter((m) => m.category === "effect");
  await reloadPresets();
});

watch(() => project.presetsVersion, reloadPresets);

// While a card is dragged towards the save zone, light up the family it will
// join: an instrument card (whole chain) saves among the instruments, an
// effect node among the effects.
const savableChain = computed(() => drag.state.source?.kind === "chain");
const savableFx = computed(() => drag.state.source?.kind === "node");

/** "plucked → reverb" — a chain preset's composition, shown on hover. */
const chainTitle = (p: NamedChainPreset): string => p.nodes.map((n) => n.module).join(" → ");

function dragModule(e: PointerEvent, m: ModuleDesc): void {
  e.preventDefault();
  drag.arm(
    { kind: "module", module: m.id, label: m.name, category: m.category },
    e.clientX,
    e.clientY,
  );
}

function dragPreset(e: PointerEvent, p: NamedPreset): void {
  e.preventDefault();
  drag.arm({ kind: "preset", name: p.name, module: p.module, params: p.params }, e.clientX, e.clientY);
}

function dragChainPreset(e: PointerEvent, p: NamedChainPreset): void {
  e.preventDefault();
  drag.arm({ kind: "chainPreset", name: p.name, nodes: p.nodes }, e.clientX, e.clientY);
}
</script>

<template>
  <div class="browser" data-save-preset>
    <input
      v-model="search"
      class="search"
      type="search"
      :placeholder="t('browser.search')"
      spellcheck="false"
    />

    <section class="group" :class="{ 'savable-notes': savableChain }">
      <button type="button" class="group-head" @click="open.instruments = !open.instruments">
        <Icon name="chevron-down" :size="12" :class="{ closed: !open.instruments }" />
        <span class="group-title">{{ t("browser.instruments") }}</span>
        <span class="count num">{{ shownInstruments.length + shownChainPresets.length }}</span>
      </button>
      <div v-if="open.instruments" class="items">
        <div
          v-for="m in shownInstruments"
          :key="m.id"
          class="item instrument"
          @pointerdown="dragModule($event, m)"
        >
          {{ m.name }}
        </div>
        <!-- Saved chain presets: named, ready-made instrument + effects. -->
        <div
          v-for="p in shownChainPresets"
          :key="p.name"
          class="item instrument preset"
          :title="chainTitle(p)"
          @pointerdown="dragChainPreset($event, p)"
        >
          {{ p.name }}
        </div>
      </div>
    </section>

    <section class="group" :class="{ 'savable-fx': savableFx }">
      <button type="button" class="group-head" @click="open.effects = !open.effects">
        <Icon name="chevron-down" :size="12" :class="{ closed: !open.effects }" />
        <span class="group-title">{{ t("browser.modules") }}</span>
        <span class="count num">{{ shownEffects.length + shownPresets.length }}</span>
      </button>
      <div v-if="open.effects" class="items">
        <div
          v-for="m in shownEffects"
          :key="m.id"
          class="item effect"
          @pointerdown="dragModule($event, m)"
        >
          {{ m.name }}
        </div>
        <!-- Saved effect presets: one module with its settings. -->
        <div
          v-for="p in shownPresets"
          :key="p.name"
          class="item effect preset"
          :title="p.module"
          @pointerdown="dragPreset($event, p)"
        >
          {{ p.name }}
        </div>
      </div>
    </section>

    <span class="hint">{{ t("browser.dropToSave") }}</span>
  </div>
</template>

<style scoped>
.browser {
  width: 100%;
  height: 100%;
  padding: 0.4rem 0.6rem;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}

.search {
  width: 100%;
  padding: 0.3rem 0.5rem;
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-size: 0.8rem;
  transition: border-color var(--t-fast);
}

.search:focus {
  outline: none;
  border-color: var(--accent);
}

.search::placeholder {
  color: var(--text-3);
}

.group-head {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  width: 100%;
  border: none;
  background: none;
  padding: 0.2rem 0;
  color: var(--text-2);
  cursor: pointer;
}

.group-head:hover {
  color: var(--text);
}

.group-head .closed {
  transform: rotate(-90deg);
}

.group-head svg {
  transition: transform var(--t-med);
}

.group-title {
  font-family: var(--font-display);
  font-size: 0.65rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.count {
  margin-left: auto;
  font-size: 0.65rem;
  color: var(--text-3);
}

.items {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
  padding-top: 0.25rem;
}

.item {
  padding: 0.3rem 0.5rem;
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--text);
  font-size: 0.85rem;
  cursor: grab;
  user-select: none;
  /* Same elevation as the chain cards: a graspable object on the tray. */
  box-shadow: var(--shadow-raised);
  transition: background-color var(--t-fast), border-color var(--t-fast);
}

.item:hover {
  background: var(--surface-2);
  border-color: var(--accent);
}

/* Signal-typed spines: magenta = consumes Notes, teal = processes Audio. */
.item.instrument {
  border-left: 2px solid var(--sig-notes);
}

.item.effect {
  border-left: 2px solid var(--sig-audio);
}

/* Dashed = a user preset (a saved sound), solid = a factory module. */
.item.preset {
  border-style: dashed;
}

/* Each family is a save drop target for its own kind of card; the tint says
   which family the dragged card will join. */
.group {
  border: 1px dashed transparent;
  border-radius: var(--radius-md);
  transition: border-color var(--t-fast), background-color var(--t-fast);
}

.group.savable-notes {
  border-color: var(--sig-notes);
  background: color-mix(in srgb, var(--sig-notes) 8%, transparent);
}

.group.savable-fx {
  border-color: var(--sig-audio);
  background: color-mix(in srgb, var(--sig-audio) 8%, transparent);
}

.hint {
  margin-top: auto;
  padding-top: 0.4rem;
  font-size: 0.7rem;
  color: var(--text-3);
}
</style>
