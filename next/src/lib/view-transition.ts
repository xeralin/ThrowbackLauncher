"use client";

import { flushSync } from "react-dom";

let switching = false;

export function isSwitching(): boolean {
  return switching;
}

export function applySwitch(apply: () => void): void {
  switching = true;
  flushSync(apply);
  queueMicrotask(() => {
    switching = false;
  });
}

export function withViewTransition(apply: () => void, type: string): void {
  const update = () => applySwitch(apply);
  if (typeof document.startViewTransition !== "function") {
    update();
    return;
  }
  document.startViewTransition({ update, types: [type] });
}
