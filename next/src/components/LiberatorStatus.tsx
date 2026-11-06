import { StrokeIcon, UnlockIcon, WarningIcon } from "@/components/icons";
import type { LiberatorState } from "@/lib/bridge";

export function LiberatorStatus({
  lib,
  enabled,
  loadingOnly = false,
  size = "size-4.5",
}: {
  lib: LiberatorState;
  enabled: boolean;
  loadingOnly?: boolean;
  size?: string;
}) {
  if (!enabled || (lib.available && !lib.attached)) return null;
  const problem = !lib.available || lib.unsupported;
  if (loadingOnly && (problem || lib.unlocked)) return null;

  return (
    <span className="flex">
      {problem ? (
        <WarningIcon className={`${size} text-notice-edge`} />
      ) : lib.unlocked ? (
        <UnlockIcon className={`${size} text-success`} />
      ) : (
        <StrokeIcon className={`${size} animate-spin text-text`}>
          <circle
            cx="12"
            cy="12"
            r="9"
            strokeWidth={3}
            className="stroke-border"
          />
          <path d="M21 12a9 9 0 0 0-9-9" strokeWidth={3} />
        </StrokeIcon>
      )}
    </span>
  );
}
