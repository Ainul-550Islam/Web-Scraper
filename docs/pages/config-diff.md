# Config Diff

| | |
|---|---|
| Route | `/config-diff` |
| Nav group | Console |
| Shortcut | — (none registered) |
| Endpoints | none — fully client-side |
| Polling | none |

Paste two config documents (typically `harvest config inspect` output vs the
file on disk) and see what actually differs.

**Read-only by design.** The engine exposes no config-write endpoint (GET
/policy itself carries an honest "not implemented" clause for hot reload), so
this page has no apply, save, or mutate affordance — a diff tool that promised
to write would be lying. Both panes are plain textareas; parsing, canonical
comparison, and the diff all happen in the tab. Nothing is uploaded.

Details that keep it trustworthy:

* **Canonicalization first:** keys are sorted recursively before diffing, so
  key-order noise is never reported as a change ("identical after
  canonicalization" is a real result, not a shrug).
* **Line diff:** classic LCS over pretty-printed lines, hard-capped at
  4 000 lines per side — bigger documents are refused with a message, never
  silently truncated. Documents beyond a 2 MB JSON budget are refused too.
* **Copy affordance:** the `+`/`-` lines copy to the clipboard (minus any
  credential-shaped values? — not needed: the diff renders your own pasted
  text and never the server's masked config; nothing new is exposed by
  copying what you typed).
* Invalid JSON is an inline red error under the pane that caused it, naming
  left or right.

- **Source:** `web/src/features/config/ConfigDiff.tsx`
- **Tests:** `web/src/test/final-pages.test.tsx`
- **See also:** [config studio](config.md)
