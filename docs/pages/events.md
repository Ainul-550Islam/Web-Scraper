# Event Stream

| | |
|---|---|
| Route | `/events` |
| Nav group | Network & Flow |
| Shortcut | — (none registered) |
| Endpoints | `/events?limit&kind&since_seq` |
| Polling | 5 s |

The structured engine ring with an inspector. What is server-side and what is
not is labeled in the UI itself:

* **server-side:** `limit` (50 / 200 window buttons), `kind` (the exact
  `EVENT_KINDS` vocabulary from `web/src/lib/types`), `since_seq`. Those
  three become real query params — the same contract the Activity page uses,
  asserted against recorded calls in `web/src/test/command-center.test.tsx`.
* **client-side (labeled "this window only"):** the substring box. It
  filters the fetched tail in memory; no `q` param is ever sent because the
  route has none.

Selecting a row renders the full event through `JsonInspector`, which masks
**before** display and before copy: credential-shaped keys (token,
authorization, password, secret, webhook, API key, DSN, credentials) become
`***`, `user:pass@host` URLs lose their userinfo, and the copy button only
offers the masked serialization. Rendering is text nodes only — hostile JSON
strings cannot inject markup.

The header sub-line is honesty in one sentence: `ring N · last seq M ·
dropped K`, with a `degraded` chip when `dropped > 0` — the tail is a tail.

- **Source:** `web/src/features/events/EventsPage.tsx`
- **See also:** [activity](activity.md) · [api/05 events](../api/05-get-events-structured-event-tail.md)
