"use client";

import { createPortal } from "react-dom";
import { PlatformSwitch } from "@/components/PlatformSwitch";
import { PlatformViewScope } from "@/lib/platform-view";
import { useTopbarSlot } from "@/lib/topbar-slot";

export default function FaqLayout({ children }: { children: React.ReactNode }) {
  const slot = useTopbarSlot();

  return (
    <PlatformViewScope value={true}>
      {slot && createPortal(<PlatformSwitch />, slot)}
      {children}
    </PlatformViewScope>
  );
}
