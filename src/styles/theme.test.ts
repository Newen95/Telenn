import { describe, expect, it } from "vitest";

import { projectHues, theme, themeVars, trackColor, trackHue, unusedHue } from "./theme";

describe("themeVars", () => {
  it("derives one CSS custom property per token", () => {
    const vars = themeVars();
    expect(vars["--bg"]).toBe(theme.color.bg);
    expect(vars["--accent-bright"]).toBe(theme.color.accentBright);
    expect(vars["--sig-notes"]).toBe(theme.color.sigNotes);
    expect(vars["--track-0"]).toBe(theme.track[0]);
    expect(vars["--radius-md"]).toBe(theme.radius.md);
    expect(vars["--t-fast"]).toBe(theme.transition.fast);
    expect(vars["--font-mono"]).toBe(theme.font.mono);
  });

  it("emits only custom-property names", () => {
    for (const name of Object.keys(themeVars())) {
      expect(name).toMatch(/^--[a-z0-9-]+$/);
    }
  });
});

describe("trackColor", () => {
  it("wraps past the palette and never goes out of range", () => {
    expect(trackColor(0)).toBe(theme.track[0]);
    expect(trackColor(theme.track.length)).toBe(theme.track[0]);
    expect(trackColor(theme.track.length + 2)).toBe(theme.track[2]);
  });
});

describe("trackHue", () => {
  it("prefers the persisted color", () => {
    expect(trackHue({ id: "lead", color: "#123456" })).toBe("#123456");
  });

  it("falls back to a stable id-derived hue (reorder-proof)", () => {
    const hue = trackHue({ id: "lead" });
    expect(theme.track).toContain(hue);
    expect(trackHue({ id: "lead" })).toBe(hue); // same id → same hue, always
  });
});

describe("projectHues", () => {
  const tracks = [{ id: "lead" }, { id: "bass" }, { id: "pad" }, { id: "drums" }];

  it("gives colliding ids distinct hues (up to a full palette)", () => {
    const hues = [...projectHues(tracks).values()];
    expect(new Set(hues).size).toBe(tracks.length);
  });

  it("is order-independent: reordering tracks repaints nothing", () => {
    const before = projectHues(tracks);
    const after = projectHues([...tracks].reverse());
    for (const t of tracks) expect(after.get(t.id)).toBe(before.get(t.id));
  });

  it("keeps explicit colors and steers fallbacks away from them", () => {
    const explicit = theme.track[0];
    const hues = projectHues([{ id: "a", color: explicit }, { id: "b" }, { id: "c" }]);
    expect(hues.get("a")).toBe(explicit);
    expect(hues.get("b")).not.toBe(explicit);
    expect(hues.get("c")).not.toBe(explicit);
    expect(hues.get("b")).not.toBe(hues.get("c"));
  });
});

describe("unusedHue", () => {
  it("picks among the hues not taken yet", () => {
    const taken = theme.track.slice(0, theme.track.length - 1);
    expect(unusedHue(taken)).toBe(theme.track[theme.track.length - 1]);
  });

  it("still yields a palette hue when everything is taken", () => {
    expect(theme.track).toContain(unusedHue([...theme.track]));
  });
});
