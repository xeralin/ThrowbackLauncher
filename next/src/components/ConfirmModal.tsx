"use client";

import type { ReactNode } from "react";
import { Button } from "@/components/Button";
import { Modal } from "@/components/Modal";
import { Note } from "@/components/Note";

export function ConfirmModal({
  title,
  confirmLabel,
  busyLabel,
  busy = false,
  confirmOnEnter = true,
  note,
  onConfirm,
  onCancel,
  children,
}: {
  title: string;
  confirmLabel: string;
  busyLabel?: string;
  busy?: boolean;
  confirmOnEnter?: boolean;
  note?: ReactNode;
  onConfirm: () => void;
  onCancel: () => void;
  children?: ReactNode;
}) {
  return (
    <Modal
      title={title}
      onClose={busy ? undefined : onCancel}
      onConfirm={busy || !confirmOnEnter ? undefined : onConfirm}
      footer={
        <>
          {note && <Note className="mr-auto">{note}</Note>}
          <Button variant="secondary" disabled={busy} onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="primary" disabled={busy} onClick={onConfirm}>
            {busy && busyLabel ? busyLabel : confirmLabel}
          </Button>
        </>
      }
    >
      {children}
    </Modal>
  );
}
