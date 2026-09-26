"use client";

import { useEffect, useState } from "react";
import { dismissToast } from "@/lib/toast";

type Entry = {
  id: number;
  text: string;
  leaving: boolean;
  key?: string;
};

let nextId = 1;

const MAX_VISIBLE = 3;
const VISIBLE_MS = 4000;
const LEAVE_MS = 200;

export function Toasts() {
  const [toasts, setToasts] = useState<Entry[]>([]);

  useEffect(() => {
    let list: Entry[] = [];

    function apply(update: (prev: Entry[]) => Entry[]) {
      list = update(list);
      setToasts(list);
    }

    function dismiss(id: number) {
      if (!list.some((toast) => toast.id === id && !toast.leaving)) return;
      apply((prev) =>
        prev.map((toast) =>
          toast.id === id ? { ...toast, leaving: true } : toast,
        ),
      );
      window.setTimeout(() => {
        apply((prev) => prev.filter((toast) => toast.id !== id));
      }, LEAVE_MS);
    }

    function onToast(raw: Event) {
      const detail = (raw as CustomEvent).detail as {
        text: string;
        key?: string;
      };
      const id = nextId++;
      if (detail.key) {
        apply((prev) => prev.filter((toast) => toast.key !== detail.key));
      }
      const active = list.filter((toast) => !toast.leaving);
      if (active.length >= MAX_VISIBLE) dismiss(active[0].id);
      apply((prev) => [
        ...prev,
        {
          id,
          text: detail.text,
          key: detail.key,
          leaving: false,
        },
      ]);
      window.setTimeout(() => dismiss(id), VISIBLE_MS);
    }

    function onDismiss(raw: Event) {
      const detail = (raw as CustomEvent).detail as {
        key?: string;
        id?: number;
      };
      const match = list.find(
        (toast) =>
          !toast.leaving &&
          (detail.id !== undefined
            ? toast.id === detail.id
            : toast.key === detail.key),
      );
      if (match) dismiss(match.id);
    }

    window.addEventListener("throwback:toast", onToast);
    window.addEventListener("throwback:toast-dismiss", onDismiss);
    return () => {
      window.removeEventListener("throwback:toast", onToast);
      window.removeEventListener("throwback:toast-dismiss", onDismiss);
    };
  }, []);

  return (
    <div
      data-overlay
      role="status"
      className="pointer-events-none fixed bottom-5 right-5 z-(--z-toast) flex max-w-[360px] flex-col items-end"
    >
      {toasts.map((toast) => (
        <div
          key={toast.id}
          className={`grid overflow-hidden transition-[grid-template-rows] duration-200 ease-out-cubic ${
            toast.leaving
              ? "[grid-template-rows:0fr]"
              : "animate-toast-in [grid-template-rows:1fr]"
          }`}
        >
          <div className="min-h-0 self-start">
            <button
              type="button"
              onClick={() => {
                navigator.clipboard.writeText(toast.text).catch(() => {});
                dismissToast({ id: toast.id });
              }}
              className={`pointer-events-auto mt-2 cursor-pointer rounded-md border border-border bg-surface-2 p-2.5 text-left font-mono text-ui text-text ${
                toast.leaving ? "animate-toast-fade-out" : "animate-toast-fade"
              }`}
            >
              {toast.text}
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}
