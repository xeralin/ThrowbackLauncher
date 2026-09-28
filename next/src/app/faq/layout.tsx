"use client";

import { createPortal } from "react-dom";
import { PlatformSwitch } from "@/components/PlatformSwitch";
import { useTopbarSlot } from "@/lib/topbar-slot";

export default function FaqLayout({ children }: { children: React.ReactNode }) {
  const slot = useTopbarSlot();

  return (
    <>
      {slot && createPortal(<PlatformSwitch />, slot)}
      {children}
    </>
  );
}
