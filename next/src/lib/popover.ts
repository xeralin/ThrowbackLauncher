import { type RefObject, useEffect } from "react";

let openPopovers = 0;

export function hasOpenPopover(): boolean {
  return openPopovers > 0;
}

export function usePopoverDismiss(
  open: boolean,
  setOpen: (open: boolean) => void,
  ref: RefObject<HTMLElement | null>,
): void {
  useEffect(() => {
    if (!open) return;
    openPopovers += 1;
    function onOutside(event: Event) {
      if (ref.current && !ref.current.contains(event.target as Node))
        setOpen(false);
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setOpen(false);
    }
    window.addEventListener("mousedown", onOutside);
    window.addEventListener("focusin", onOutside);
    window.addEventListener("keydown", onKey);
    return () => {
      openPopovers -= 1;
      window.removeEventListener("mousedown", onOutside);
      window.removeEventListener("focusin", onOutside);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, setOpen, ref]);
}
