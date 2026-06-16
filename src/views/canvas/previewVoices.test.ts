import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { usePreviewVoices, type PreviewSink } from "./previewVoices";

// The preview is strictly monophonic: at most one pitch is ever lit/sounding, a
// new strike replaces the previous, and the voice clears on its own. The release
// must target the *node it was struck on* — switching the target instrument
// mid-flight must not leave a note droning (the "phantom note" bug).

/** A sink that records every note on/off so we can assert the exact sequence. */
function recordingSink() {
  const events: string[] = [];
  const sink: PreviewSink = {
    on: (node, pitch) => events.push(`on ${node} ${pitch}`),
    off: (node, pitch) => events.push(`off ${node} ${pitch}`),
  };
  return { sink, events };
}

describe("preview voices", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("lights at most one pitch: a new strike replaces the previous", () => {
    const { sink, events } = recordingSink();
    const voices = usePreviewVoices(sink);
    voices.strike(0, 60, 150);
    expect(voices.has(60)).toBe(true);
    voices.strike(0, 62, 150); // crossing into a new row during a drag
    expect(voices.has(60)).toBe(false); // previous key turned off
    expect(voices.has(62)).toBe(true);
    expect(voices.any()).toBe(true);
    expect(events).toEqual(["on 0 60", "off 0 60", "on 0 62"]);
  });

  it("clears the lit key when the window elapses (no key stuck)", () => {
    const { sink, events } = recordingSink();
    const voices = usePreviewVoices(sink);
    voices.strike(0, 64, 150);
    expect(voices.any()).toBe(true);
    vi.advanceTimersByTime(150);
    expect(voices.any()).toBe(false);
    expect(voices.has(64)).toBe(false);
    expect(events).toEqual(["on 0 64", "off 0 64"]);
  });

  it("releases on the node it was struck on, not the new one", () => {
    // Audition pitch 60 on node 0 (one track), then on node 1 (after switching):
    // the release of 60 must go to node 0, or node 0 drones forever.
    const { sink, events } = recordingSink();
    const voices = usePreviewVoices(sink);
    voices.strike(0, 60, 150);
    voices.strike(1, 62, 150);
    expect(events).toEqual(["on 0 60", "off 0 60", "on 1 62"]);
  });

  it("release() silences the current voice immediately (unmount)", () => {
    const { sink, events } = recordingSink();
    const voices = usePreviewVoices(sink);
    voices.strike(0, 67, 150);
    voices.release();
    expect(voices.any()).toBe(false);
    expect(events).toEqual(["on 0 67", "off 0 67"]);
    vi.advanceTimersByTime(150); // the cancelled timer must not fire a second off
    expect(events).toEqual(["on 0 67", "off 0 67"]);
  });

  it("hold keeps a key sounding with no fixed duration, until release()", () => {
    const { sink, events } = recordingSink();
    const voices = usePreviewVoices(sink);
    voices.hold(0, 60); // pointer pressed on a gutter key
    expect(voices.has(60)).toBe(true);
    vi.advanceTimersByTime(1000); // a held key never auto-releases
    expect(voices.has(60)).toBe(true);
    voices.release(); // pointer up
    expect(voices.any()).toBe(false);
    expect(events).toEqual(["on 0 60", "off 0 60"]);
  });

  it("glissando: sliding to a new key releases the previous and sounds the new", () => {
    const { sink, events } = recordingSink();
    const voices = usePreviewVoices(sink);
    voices.hold(0, 60);
    voices.hold(0, 62);
    voices.hold(0, 64);
    expect(voices.has(64)).toBe(true);
    expect(voices.has(60)).toBe(false); // strictly monophonic
    expect(events).toEqual(["on 0 60", "off 0 60", "on 0 62", "off 0 62", "on 0 64"]);
  });

  it("holding the same key again does not re-trigger it (pointer jitter)", () => {
    const { sink, events } = recordingSink();
    const voices = usePreviewVoices(sink);
    voices.hold(0, 60);
    voices.hold(0, 60);
    expect(events).toEqual(["on 0 60"]);
  });

  it("a re-strike refreshes the window rather than stacking timers", () => {
    const { sink } = recordingSink();
    const voices = usePreviewVoices(sink);
    voices.strike(0, 60, 150);
    vi.advanceTimersByTime(100);
    voices.strike(0, 60, 150); // same pitch again before it expired
    vi.advanceTimersByTime(100); // 100ms after the re-strike: still within 150ms
    expect(voices.has(60)).toBe(true);
    vi.advanceTimersByTime(50);
    expect(voices.any()).toBe(false);
  });
});
