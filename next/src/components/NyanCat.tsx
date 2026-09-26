"use client";

import { useEffect, useSyncExternalStore } from "react";
import { ExternalLink } from "@/components/ExternalLink";
import { useSettings } from "@/lib/bridge";

const NYAN_GIF = "https://www.nyan.cat/cats/original.gif";

let status: "idle" | "loading" | "ready" | "failed" = "idle";
const listeners = new Set<() => void>();

function setStatus(next: typeof status) {
  status = next;
  listeners.forEach((listener) => listener());
}

function load() {
  if (status === "loading" || status === "ready") return;
  setStatus("loading");
  const image = new Image();
  image.onload = () => setStatus("ready");
  image.onerror = () => setStatus("failed");
  image.src = NYAN_GIF;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useNyan(enabled: boolean): boolean {
  const current = useSyncExternalStore(
    subscribe,
    () => status,
    () => "idle",
  );

  useEffect(() => {
    if (enabled) load();
  }, [enabled]);

  return enabled && current === "ready";
}

export function NyanCat() {
  if (!useNyan(!!useSettings()?.bar_nyan)) return null;
  return (
    <ExternalLink
      href="https://www.nyan.cat/"
      aria-label="Nyan Cat"
      className="nyan-cat"
    >
      <img src={NYAN_GIF} alt="" />
    </ExternalLink>
  );
}
