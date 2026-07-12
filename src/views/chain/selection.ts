//! Node selection + keyboard actions for the effect chain (M6).
//!
//! Pure and DOM-free (like `dragdrop.ts`): the component owns the pointer/focus
//! plumbing and reads `selected` for the highlight, while the *decision* — what a
//! keydown does given the selection and whether the chain owns the focus — lives
//! here so it is unit-tested. The focus guard mirrors the canvas rule: a keyboard
//! action only fires when the chain has the focus, never while a text field or a
//! piano-roll note is being edited.

/** What a keydown resolves to on the chain, or `null` to let it pass through. */
export type ChainKeyAction = "delete" | "duplicate" | null;

/** Keyboard modifiers relevant to chain shortcuts. */
export interface KeyMods {
  ctrl: boolean;
  meta: boolean;
}

export class ChainSelection {
  /** Currently selected node id (instrument included), or `null`. Delete and
   *  duplicate stay effect-only: the panel filters them out for the head. */
  selected: string | null = null;

  select(id: string | null): void {
    this.selected = id;
  }

  /** Drop the selection (e.g. after the selected node is deleted). */
  clear(): void {
    this.selected = null;
  }

  /**
   * Resolve a keydown. Inert unless the chain owns the focus and a node is
   * selected — so Delete never bites while editing notes or typing in a field.
   * Delete/Backspace remove the node; Ctrl/Cmd+D duplicate it.
   */
  action(key: string, mods: KeyMods, focused: boolean): ChainKeyAction {
    if (!focused || this.selected === null) return null;
    if (key === "Delete" || key === "Backspace") return "delete";
    if ((mods.ctrl || mods.meta) && (key === "d" || key === "D")) return "duplicate";
    return null;
  }
}
