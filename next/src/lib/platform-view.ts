"use client";

import { createContext, useContext, useSyncExternalStore } from "react";
import { usePlatform, type ThrowbackOS } from "@/lib/bridge";
import { withViewTransition } from "@/lib/view-transition";

export const PlatformViewScope = createContext(false);

let override: ThrowbackOS | null = null;
const listeners = new Set<() => void>();

function getSnapshot(): ThrowbackOS | null {
  return override;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function setPlatformView(os: ThrowbackOS) {
  if (override === os) return;
  withViewTransition(() => {
    override = os;
    listeners.forEach((listener) => listener());
  });
}

export function resetPlatformView() {
  if (override === null) return;
  override = null;
  listeners.forEach((listener) => listener());
}

export function usePlatformView(): ThrowbackOS | null {
  const os = usePlatform();
  const scoped = useContext(PlatformViewScope);
  const view = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  return (scoped ? view : null) ?? os;
}
