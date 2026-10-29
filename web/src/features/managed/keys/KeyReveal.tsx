// KeyReveal — the one-time reveal.
//
// The plaintext exists in exactly one place: the response of the POST that
// created (or rotated) the key, held in React state for as long as the modal
// is open. It is never written to localStorage, sessionStorage, cookies or the
// URL, and it is gone the moment the modal closes — which is why `onClose`
// clears the copy before it runs.

import { useState } from "react";
import { Button, CopyButton } from "../../../components/ui";
import { Icon } from "../../../components/Icons";
import { Modal } from "../../../components/Overlays";

export function KeyReveal({
  apiKey,
  keyId,
  title,
  onClose,
}: {
  apiKey: string;
  keyId: string;
  title: string;
  onClose: () => void;
}) {
  // `confirmed` starts false on every mount; the page remounts this dialog per
  // key (key={apiKey}) so a rotation cannot leave the old confirmation behind.
  const [confirmed, setConfirmed] = useState(false);

  function close() {
    // The string is dropped before the parent can do anything else with it.
    onClose();
  }

  return (
    <Modal
      title={title}
      onClose={close}
      footer={
        <>
          <Button onClick={close}>Done — I have stored it</Button>
        </>
      }
    >
      <p className="t-caption" style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <Icon.Warn size={13} />
        This is the only time this key will be shown. It is stored server-side as a SHA-256
        digest; nothing else in the console can read it back.
      </p>

      <div className="reveal">
        <code className="t-code reveal-value" data-testid="revealed-key">
          {apiKey}
        </code>
        <CopyButton text={apiKey} label="Copy key" onCopied={() => setConfirmed(true)} />
      </div>

      <p className="t-caption">Key id {keyId}</p>

      {confirmed && (
        <p className="t-caption" role="status" style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <Icon.Check size={13} /> Copied. Close this dialog to clear the value from memory.
        </p>
      )}

      <p className="t-caption">
        Send it as <code className="t-code">Authorization: Bearer &lt;key&gt;</code> on
        <code className="t-code"> /v1/scrape</code>, <code className="t-code">/v1/jobs</code> and
        <code className="t-code"> /v1/usage</code>.
      </p>
    </Modal>
  );
}
