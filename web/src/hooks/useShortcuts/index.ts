// useShortcuts: barrel — one file per unit; every prior import path & name still resolves

/**
 * Global vim-style navigation shortcuts (§29):
 *
 *   /        open the command palette (search)         [owned by App.tsx]
 *   g o      Overview          g r   Runs              g j   Jobs
 *   g w      Workers           g p   Proxies           g q   Queue
 *   c        —                —      —                —     —
 *
 * The `g` prefix arms a 1.5 s window; a second unknown key disarms it.
 * While any text field (or contenteditable) has focus, or while a dialog
 * owns the keyboard (caller passes enabled=false), nothing is intercepted.
 */
export * from "./GOTO";
export * from "./isEditable";
export * from "./useShortcuts";
export * from "./SHORTCUT_DOC";
