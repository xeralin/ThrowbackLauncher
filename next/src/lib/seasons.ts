import { UNLOCK_ALL_SEASONS } from "@/config/liberator-builds";

function seasonRank(key: string): number {
  const match = /^Y(\d+)S(\d+)/.exec(key);
  return match ? Number(match[1]) * 10 + Number(match[2]) : 0;
}

export function operatorsLocked(key: string): boolean {
  return (
    seasonRank(key) >= seasonRank("Y8S4") &&
    !UNLOCK_ALL_SEASONS.some((row) => key.startsWith(`${row.season}_`))
  );
}
