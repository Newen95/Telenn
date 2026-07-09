<script setup lang="ts">
// Library browser: collapsible sections (instruments / effects / presets) with
// a search field that filters all of them. Items are dragged from here into a
// track's chain (insert), onto a chain head (replace instrument) or the
// new-track zone. The presets section is also a drop target: dragging an
// effect node onto it saves that node as a preset (handled by the App
// controller, which owns the pointer plumbing; here we only mark the zone).

import { computed, onMounted, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type ModuleDesc } from "../../stores/project";
import { useChainDrag } from "../chain/dragdrop";
import Icon from "../../ui/Icon.vue";

interface NamedPreset {
  name: string;
  module: string;
  params: Record<string, number>;
}

const { t } = useI18n();
const project = useProjectStore();
const drag = useChainDrag();

const instruments = ref<ModuleDesc[]>([]);
const effects = ref<ModuleDesc[]>([]);
const presets = ref<NamedPreset[]>([]);

const search = ref("");
const open = reactive({ instruments: true, effects: true, presets: true });

const match = (name: string): boolean =>
  name.toLowerCase().includes(search.value.trim().toLowerCase());

const shownInstruments = computed(() => instruments.value.filter((m) => match(m.name)));
const shownEffects = computed(() => effects.value.filter((m) => match(m.name)));
const shownPresets = computed(() => presets.value.filter((p) => match(p.name)));

async function reloadPresets(): Promise<void> {
  const names = await project.listPresets();
  presets.value = await Promise.all(
    names.map(async (name) => {
      const p = await project.readPreset(name);
      return { name, module: p.module, params: p.params };
    }),
  );
}

onMounted(async () => {
  const modules = await project.listModules();
  instruments.value = modules.filter((m) => m.category === "instrument");
  effects.value = modules.filter((m) => m.category === "effect");
  await reloadPresets();
});

watch(() => project.presetsVersion, reloadPresets);

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
</script>

<template>
  <div class="browser">
    <input
      v-model="search"
      class="search"
      type="search"
      :placeholder="t('browser.search')"
      spellcheck="false"
    />

    <section class="group">
      <button type="button" class="group-head" @click="open.instruments = !open.instruments">
        <Icon name="chevron-down" :size="12" :class="{ closed: !open.instruments }" />
        <span class="group-title">{{ t("browser.instruments") }}</span>
        <span class="count num">{{ shownInstruments.length }}</span>
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
      </div>
    </section>

    <section class="group">
      <button type="button" class="group-head" @click="open.effects = !open.effects">
        <Icon name="chevron-down" :size="12" :class="{ closed: !open.effects }" />
        <span class="group-title">{{ t("browser.modules") }}</span>
        <span class="count num">{{ shownEffects.length }}</span>
      </button>
      <div v-if="open.effects" class="items">
        <div
          v-for="m in shownEffects"
          :key="m.id"
          class="item"
          @pointerdown="dragModule($event, m)"
        >
          {{ m.name }}
        </div>
      </div>
    </section>

    <!-- Drop an effect node here to save it as a preset. -->
    <section
      class="group presets"
      :class="{ savable: drag.state.source?.kind === 'node' }"
      data-save-preset
    >
      <button type="button" class="group-head" @click="open.presets = !open.presets">
        <Icon name="chevron-down" :size="12" :class="{ closed: !open.presets }" />
        <span class="group-title">{{ t("browser.presets") }}</span>
        <span class="count num">{{ shownPresets.length }}</span>
      </button>
      <div v-if="open.presets" class="items">
        <div
          v-for="p in shownPresets"
          :key="p.name"
          class="item preset"
          @pointerdown="dragPreset($event, p)"
        >
          {{ p.name }}
        </div>
        <span v-if="presets.length === 0" class="empty">{{ t("browser.dropToSave") }}</span>
      </div>
    </section>
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
  transition: background-color var(--t-fast), border-color var(--t-fast);
}

.item:hover {
  background: var(--surface-2);
  border-color: var(--accent);
}

.item.instrument {
  border-left: 2px solid var(--sig-notes);
}

.item.preset {
  border-style: dashed;
}

.presets {
  border: 1px dashed transparent;
  border-radius: var(--radius-md);
  transition: border-color var(--t-fast), background-color var(--t-fast);
}

/* Highlighted while a node is being dragged: drop it here to save a preset. */
.presets.savable {
  border-color: var(--sig-mod);
  background: rgba(217, 164, 65, 0.08);
}

.empty {
  font-size: 0.75rem;
  color: var(--text-3);
}
</style>
