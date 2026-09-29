# Keyboard navigation uses a g-prefix chord with a 1.5s window

Status: **Accepted**

## Context & decision

Single keys collide with typing; Ctrl/Meta chords collide with the browser. A `g` leader with a 1.5-second window maps g+o/r/j/w/p/q/c to the main pages, mirrors `SHORTCUT_DOC` into the palette and help overlay, and is inert whenever an input, select, textarea, or contenteditable has focus. Ctrl+K remains the discoverable fallback path.
