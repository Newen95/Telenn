<script setup lang="ts">
// The "string" VU — Telenn's signature meter. A taut string at rest (faint
// line); the signal lights it up: a bright teal fill whose reach IS the level,
// with a soft glow. Full scale turns it red. Vertical by default (chain cards,
// mixer); horizontal for wide contexts (the top bar).
//
// Drawn on a device-pixel-exact canvas (same recipe as the grid views): at
// fractional display scales, DOM hairlines antialias differently depending on
// where each meter happens to sit in the page — a canvas rasterizes every
// string identically. Purely presentational: the caller owns telemetry and
// ballistics.

import { onMounted, ref, watchEffect } from "vue";

import { theme, withAlpha } from "../styles/theme";

const props = withDefaults(
  defineProps<{
    rms: number;
    peak: number;
    /** Length of the string along its axis, px. */
    size?: number;
    orientation?: "vertical" | "horizontal";
  }>(),
  { size: 20, orientation: "vertical" },
);

/** Breadth of the meter across the string, CSS px. */
const BREADTH = 12;

const canvas = ref<HTMLCanvasElement | null>(null);

function draw(): void {
  const el = canvas.value;
  if (!el) return;
  const ctx = el.getContext("2d");
  if (!ctx) return;

  const dpr = window.devicePixelRatio || 1;
  const vertical = props.orientation === "vertical";
  const cssW = vertical ? BREADTH : props.size;
  const cssH = vertical ? props.size : BREADTH;
  const w = Math.max(1, Math.round(cssW * dpr));
  const h = Math.max(1, Math.round(cssH * dpr));
  if (el.width !== w || el.height !== h) {
    el.width = w;
    el.height = h;
  }

  ctx.clearRect(0, 0, w, h);
  const level = Math.min(1, props.peak);
  const color = props.peak >= 1 ? theme.color.danger : theme.color.sigAudio;

  // The string at rest: a faint line over the whole run, device-pixel exact.
  const coreW = Math.max(1, Math.round(1.5 * dpr));
  ctx.fillStyle = withAlpha(theme.color.sigAudio, 0.4);
  if (vertical) {
    ctx.fillRect(Math.round((w - coreW) / 2), 0, coreW, h);
  } else {
    ctx.fillRect(0, Math.round((h - coreW) / 2), w, coreW);
  }

  // The lit part: its reach is the level, with a soft glow and a rounded tip.
  if (level > 0.005) {
    const fillW = Math.max(2, Math.round(4 * dpr));
    const tip = Math.round(2 * dpr);
    ctx.save();
    ctx.shadowColor = withAlpha(color, 0.6);
    ctx.shadowBlur = 4 * dpr;
    ctx.fillStyle = color;
    ctx.beginPath();
    if (vertical) {
      const reach = Math.round(h * level);
      ctx.roundRect(Math.round((w - fillW) / 2), h - reach, fillW, reach, [tip, tip, 0, 0]);
    } else {
      const reach = Math.round(w * level);
      ctx.roundRect(0, Math.round((h - fillW) / 2), reach, fillW, [0, tip, tip, 0]);
    }
    ctx.fill();
    ctx.restore();
  }
}

onMounted(draw);
watchEffect(draw);
</script>

<template>
  <canvas
    ref="canvas"
    class="string-meter"
    :style="
      orientation === 'vertical'
        ? { width: `${BREADTH}px`, height: `${size}px` }
        : { width: `${size}px`, height: `${BREADTH}px` }
    "
  ></canvas>
</template>

<style scoped>
.string-meter {
  display: block;
  flex: none;
}
</style>
