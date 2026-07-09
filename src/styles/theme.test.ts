import { describe, expect, it } from "vitest";

import { theme, themeVars, trackColor } from "./theme";

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
