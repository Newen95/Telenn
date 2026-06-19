<script setup lang="ts">
// Browser of effect modules and saved presets. Items are dragged from here into a
// track's chain (insert). The presets section is also a drop target: dragging an
// effect node onto it saves that node as a preset (handled by the App controller,
// which owns the pointer plumbing; here we only mark the zone and the sources).

import { onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { useProjectStore, type ModuleDesc } from "../../stores/project";
import { useChainDrag } from "../chain/dragdrop";

interface NamedPreset {
  name: string;
  module: string;
  params: Record<string, number>;
}

const { t } = useI18n();
const project = useProjectStore();
const drag = useChainDrag();

const effects = ref<ModuleDesc[]>([]);
const presets = ref<NamedPreset[]>([]);

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
  // Only effects are insertable into chains (instruments are the fixed chain head).
  effects.value = (await project.listModules()).filter((m) => m.category === "effect");
  await reloadPresets();
});

watch(() => project.presetsVersion, reloadPresets);

function dragModule(e: PointerEvent, m: ModuleDesc): void {
  e.preventDefault();
  drag.arm({ kind: "module", module: m.id, label: m.name }, e.clientX, e.clientY);
}

function dragPreset(e: PointerEvent, p: NamedPreset): void {
  e.preventDefault();
  drag.arm({ kind: "preset", name: p.name, module: p.module, params: p.params }, e.clientX, e.clientY);
}
</script>

<template>
  <div class="browser">
    <section class="group">
      <h3 class="group-title">{{ t("browser.modules") }}</h3>
      <div class="items">
        <div
          v-for="m in effects"
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
      <h3 class="group-title">{{ t("browser.presets") }}</h3>
      <div class="items">
        <div
          v-for="p in presets"
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
  width: 200px;
  border-left: 1px solid #2c2c2e;
  padding: 0.4rem 0.6rem;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 0.8rem;
}

.group-title {
  margin: 0 0 0.3rem;
  font-size: 0.65rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: #8e8e93;
}

.items {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}

.item {
  padding: 0.3rem 0.5rem;
  border: 1px solid #3a3a3c;
  border-radius: 6px;
  background: #2c2c2e;
  color: #f6f6f6;
  font-size: 0.85rem;
  cursor: grab;
  user-select: none;
}

.item.preset {
  border-style: dashed;
}

.presets {
  border: 1px dashed transparent;
  border-radius: 6px;
  transition: border-color 120ms ease, background-color 120ms ease;
}

/* Highlighted while a node is being dragged: drop it here to save a preset. */
.presets.savable {
  border-color: #e0b050;
  background: rgba(224, 176, 80, 0.08);
}

.empty {
  font-size: 0.75rem;
  color: #6e6e73;
}
</style>
