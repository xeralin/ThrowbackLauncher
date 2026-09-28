"use client";

import { CopyButton } from "@/components/CopyButton";
import { ListRow } from "@/components/ui";

export function CopyPathRow({ path }: { path: string }) {
  return (
    <ListRow label={path} title={path}>
      <span className="-mr-1 flex shrink-0 items-center">
        <CopyButton
          label="Copy path"
          copiedLabel="Path copied"
          onCopy={(copied) =>
            navigator.clipboard.writeText(path).then(copied, () => {})
          }
        />
      </span>
    </ListRow>
  );
}
