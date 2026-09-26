"use client";

import { useSyncExternalStore } from "react";

const subscribe = () => () => {};
const getSlot = () => document.getElementById("topbar-actions");
const getServerSlot = () => null;

export function useTopbarSlot(): HTMLElement | null {
  return useSyncExternalStore(subscribe, getSlot, getServerSlot);
}
