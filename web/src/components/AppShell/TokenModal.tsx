// TokenModal — split from AppShell.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { Button } from "../ui";
import { useConnection } from "../../lib/connection";
import { ConfirmModal } from "../Overlays";


export function TokenModal({ onClose }: { onClose: () => void }) {
  const conn = useConnection();
  const [draft, setDraft] = useState("");
  return (
    <ConfirmModal
      title="Admin bearer token"
      confirmLabel={draft ? "Use token" : "Close"}
      onConfirm={async () => {
        if (draft.trim()) conn.setToken(draft.trim());
      }}
      onClose={onClose}
      body={
        <div style={{ display: "grid", gap: 10 }}>
          <p className="t-caption">
            The token is held in memory for this tab only — never localStorage, never the URL,
            never logged. Loopback binds need no token; anything else answers 401 until one is set.
          </p>
          <input
            className="input"
            type="password"
            autoComplete="off"
            spellCheck={false}
            aria-label="Bearer token"
            placeholder={conn.tokenSet ? "•••••••• (set — paste to replace)" : "paste admin bearer token"}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
          />
          {conn.tokenSet && (
            <Button variant="danger" onClick={() => { conn.setToken(null); onClose(); }}>
              Clear stored token
            </Button>
          )}
        </div>
      }
    />
  );
}
