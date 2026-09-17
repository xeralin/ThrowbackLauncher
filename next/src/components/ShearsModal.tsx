"use client";

import { iconButton } from "@/components/Button";
import { Modal } from "@/components/Modal";
import { ListRow } from "@/components/ui";
import { RemoveIcon } from "@/components/icons";
import { useEffect, useRef } from "react";
import { formatBytes, type ShearsAction, type ShearsKind } from "@/lib/bridge";

export function ShearsModal({
  actions,
  busy,
  onCut,
  onClose,
}: {
  actions: ShearsAction[];
  busy: boolean;
  onCut: (kind: ShearsKind, level: number) => void;
  onClose: () => void;
}) {
  const firstCutRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    if (document.activeElement === document.body) firstCutRef.current?.focus();
  }, [actions]);
  return (
    <Modal title="Shears" onClose={busy ? undefined : onClose}>
      <div className="flex flex-col gap-2">
        {actions.map((action, index) => (
          <ListRow key={action.key} label={action.label}>
            <span className="-mr-1 flex shrink-0 items-center gap-2">
              <code className="chip">{formatBytes(action.size)}</code>
              <button
                type="button"
                ref={index === 0 ? firstCutRef : undefined}
                aria-label={`Remove ${action.label}`}
                aria-disabled={busy || undefined}
                onClick={
                  busy ? undefined : () => onCut(action.kind, action.level)
                }
                className={`${iconButton}${busy ? " pointer-events-none opacity-40" : ""}`}
              >
                <RemoveIcon />
              </button>
            </span>
          </ListRow>
        ))}
      </div>
    </Modal>
  );
}
