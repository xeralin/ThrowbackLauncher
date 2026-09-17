"use client";

import { useEffect, useId, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { usePathname } from "next/navigation";
import { hasOpenInfoHint } from "@/components/InfoHint";

const stack: { id: symbol; el: HTMLElement | null }[] = [];

export function hasOpenModal(): boolean {
  return stack.length > 0;
}

function syncOverlays() {
  stack.forEach((entry, index) => {
    if (entry.el) entry.el.inert = index < stack.length - 1;
  });
}

function setBackgroundInert(inert: boolean) {
  for (const element of document.body.children) {
    if (element instanceof HTMLElement && !element.hasAttribute("data-overlay"))
      element.inert = inert;
  }
}

export function Modal({
  title,
  children,
  footer,
  onClose,
  onConfirm,
}: {
  title: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  onClose?: () => void;
  onConfirm?: () => void;
}) {
  const id = useRef(Symbol("modal"));
  const titleId = useId();
  const overlayRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const pathname = usePathname();
  const openedAt = useRef(pathname);

  useEffect(() => {
    if (pathname !== openedAt.current) onClose?.();
  }, [pathname, onClose]);

  useEffect(() => {
    const entry = { id: id.current, el: overlayRef.current };
    const previous = document.activeElement;
    stack.push(entry);
    if (stack.length === 1) {
      document.documentElement.style.overflow = "hidden";
      setBackgroundInert(true);
    }
    syncOverlays();
    if (!panelRef.current?.contains(document.activeElement))
      panelRef.current?.focus();
    return () => {
      stack.splice(stack.indexOf(entry), 1);
      if (stack.length === 0) {
        document.documentElement.style.overflow = "";
        setBackgroundInert(false);
      }
      syncOverlays();
      if (previous instanceof HTMLElement && previous.isConnected)
        previous.focus();
      else document.querySelector<HTMLElement>("main")?.focus();
    };
  }, []);

  useEffect(() => {
    if (!onClose && !onConfirm) return;
    function onKey(event: KeyboardEvent) {
      if (stack.at(-1)?.id !== id.current) return;
      if (event.key === "Escape") {
        if (!hasOpenInfoHint()) onClose?.();
        return;
      }
      if (event.key !== "Enter" || event.isComposing) return;
      const active = document.activeElement;
      if (
        active instanceof HTMLButtonElement ||
        active instanceof HTMLAnchorElement
      )
        return;
      event.preventDefault();
      onConfirm?.();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, onConfirm]);

  return createPortal(
    <div
      ref={overlayRef}
      data-overlay
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      onMouseDown={(event) => {
        if (event.target !== event.currentTarget) return;
        if (document.activeElement instanceof HTMLElement)
          document.activeElement.blur();
        onClose?.();
      }}
      className="fixed inset-0 z-(--z-modal) flex items-center justify-center bg-black/60 animate-fade-in"
    >
      <div className="animate-fade-up">
        <div
          ref={panelRef}
          tabIndex={-1}
          className="flex max-h-[calc(100dvh-3rem)] w-[min(440px,calc(100vw-2rem))] flex-col rounded-lg border border-border bg-surface p-6 outline-none"
        >
          <h2
            id={titleId}
            className="mb-3 shrink-0 font-display text-[1.2rem] font-bold text-text"
          >
            {title}
          </h2>
          {children && (
            <div className="scroll-fade -mx-6 -my-5 max-h-[25.5rem] min-h-0 overflow-y-auto px-6 py-5">
              {children}
            </div>
          )}
          {footer && (
            <div className="mt-6 flex shrink-0 items-center justify-end gap-2">
              {footer}
            </div>
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}
