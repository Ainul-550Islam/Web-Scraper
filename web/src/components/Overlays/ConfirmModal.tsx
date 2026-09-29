// ConfirmModal — split from Overlays.tsx; barrel re-exports keep every public path & name

import { useState, type ReactNode } from "react";
import { Button } from "../ui";
import { Modal } from "./Modal";


export function ConfirmModal({
  title,
  body,
  confirmLabel,
  danger,
  onConfirm,
  onClose,
}: {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void | Promise<void>;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <Modal
      title={title}
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button
            variant={danger ? "danger" : "primary"}
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await onConfirm();
              } finally {
                setBusy(false);
                onClose();
              }
            }}
          >
            {busy ? "Working…" : confirmLabel}
          </Button>
        </>
      }
    >
      <div style={{ color: "var(--text-2)", fontSize: 13 }}>{body}</div>
    </Modal>
  );
}
