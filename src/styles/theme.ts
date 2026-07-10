// The design tokens — single source of truth for every color, radius, font and
// transition in the app. Vue components consume them as CSS custom properties
// (posed on :root by applyTheme at boot); the canvas renderer imports this
// module directly. No hex value lives anywhere else.
//
// Identity: "ardoise & cuivre". Telenn is Breton for harp — slate-blue darks
// (wet granite) under a warm copper accent (the wound strings). The copper
// always means "you are in control" (interactive/active); the signal colors
// keep their semantic role everywhere: notes are violet, audio is teal,
// modulation is amber.

export const theme = {
  color: {
    /** Deepest background (canvas wells, app root). */
    bg: "#14181d",
    /** Zone/panel background. */
    panel: "#1b2128",
    /** Cards, buttons, controls. */
    surface: "#232b34",
    /** Raised/hovered surface. */
    surface2: "#2a3540",
    border: "#2e3842",
    borderStrong: "#3a4652",
    /** Warm off-white — primary text. */
    text: "#e8e4dc",
    text2: "#9aa3ab",
    text3: "#5f6b76",
    /** Copper — every interactive/active affordance. */
    accent: "#c8873a",
    accentBright: "#e8a34c",
    accentDim: "rgba(200, 135, 58, 0.15)",
    /** Signal colors (semantic, identical everywhere). */
    sigNotes: "#8f7ad6",
    sigAudio: "#4fa8a0",
    sigMod: "#d9a441",
    danger: "#d96459",
    ok: "#6aa87a",
  },
  /** Muted per-track hues (timeline lanes, mixer strips, chain header). */
  track: [
    "#5b8a6f", // sage
    "#5b6f9e", // dusk blue
    "#9e6f5b", // clay
    "#7d5b9e", // heather
    "#8f8a55", // lichen
    "#557f8f", // slate teal
    "#9e5b6f", // rosewood
    "#6f9e5b", // moss
  ],
  /** Canvas-only shades (grid lines, keyboard keys) — same slate family. */
  canvas: {
    laneLine: "#1c232a",
    trackLine: "#0a0d10", // near-black separation between timeline tracks
    beatLine: "#222a33",
    barLine: "#33404c",
    keyWhite: "#ddd8ce",
    keyBlack: "#1a2027",
    keyRow: "#0e1216",
    keyLabel: "#5f6b76",
    gutterEdge: "#0a0d10",
  },
  radius: { xs: "2px", sm: "4px", md: "6px", lg: "10px" },
  transition: { fast: "120ms ease", med: "200ms ease" },
  font: {
    display: "'Space Grotesk Variable', 'Inter Variable', system-ui, sans-serif",
    body: "'Inter Variable', system-ui, sans-serif",
    mono: "'JetBrains Mono Variable', ui-monospace, 'Cascadia Mono', monospace",
  },
  shadow: {
    raised: "0 2px 8px rgba(0, 0, 0, 0.35)",
    overlay: "0 8px 32px rgba(0, 0, 0, 0.55)",
  },
} as const;

const kebab = (s: string): string => s.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);

/** The flat CSS custom-property map derived from `theme` (pure, testable). */
export function themeVars(): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const [k, v] of Object.entries(theme.color)) vars[`--${kebab(k)}`] = v;
  theme.track.forEach((c, i) => {
    vars[`--track-${i}`] = c;
  });
  for (const [k, v] of Object.entries(theme.radius)) vars[`--radius-${k}`] = v;
  for (const [k, v] of Object.entries(theme.transition)) vars[`--t-${k}`] = v;
  for (const [k, v] of Object.entries(theme.font)) vars[`--font-${k}`] = v;
  for (const [k, v] of Object.entries(theme.shadow)) vars[`--shadow-${k}`] = v;
  return vars;
}

/** Pose every token as a CSS custom property on the root element (at boot). */
export function applyTheme(root: HTMLElement = document.documentElement): void {
  for (const [name, value] of Object.entries(themeVars())) {
    root.style.setProperty(name, value);
  }
}

/** A track's hue by its index in the project (wraps past the palette). */
export function trackColor(index: number): string {
  const palette = theme.track;
  return palette[((index % palette.length) + palette.length) % palette.length];
}

/** FNV-1a over a string: cheap, deterministic, spreads short ids well. */
function fnv(text: string): number {
  let hash = 0x811c9dc5;
  for (let i = 0; i < text.length; i += 1) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

/**
 * The display hue of every track: its explicit persisted color when set, else
 * a fallback derived from its id — never from its position, so reordering
 * never repaints anything. Fallbacks probe the palette from the id's hash to
 * the next hue not already shown, in sorted-id order (order-independent), so
 * up to a full palette of tracks all read distinct.
 */
export function projectHues(tracks: { id: string; color?: string | null }[]): Map<string, string> {
  const hues = new Map<string, string>();
  const taken = new Set<string>();
  for (const track of tracks) {
    if (track.color) {
      hues.set(track.id, track.color);
      taken.add(track.color);
    }
  }
  const fallbacks = tracks.filter((t) => !t.color).sort((a, b) => a.id.localeCompare(b.id));
  for (const track of fallbacks) {
    let i = fnv(track.id) % theme.track.length;
    for (let step = 0; step < theme.track.length && taken.has(theme.track[i]); step += 1) {
      i = (i + 1) % theme.track.length;
    }
    hues.set(track.id, theme.track[i]);
    taken.add(theme.track[i]);
  }
  return hues;
}

/** One track's hue outside a project context (prefer `projectHues`). */
export function trackHue(track: { id: string; color?: string | null }): string {
  return track.color ?? theme.track[fnv(track.id) % theme.track.length];
}

/** A `#rrggbb` color as an rgba() string at the given opacity. */
export function withAlpha(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** A hue for a new track: random among the palette hues not in use yet (all
 *  taken → random over the whole palette). */
export function unusedHue(taken: string[]): string {
  const free = theme.track.filter((c) => !taken.includes(c));
  const pool = free.length > 0 ? free : [...theme.track];
  return pool[Math.floor(Math.random() * pool.length)];
}
