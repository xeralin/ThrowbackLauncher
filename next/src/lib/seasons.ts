const LOCK_YEAR = 8;
const LOCK_SEASON = 4;
const REWORK_YEAR = 9;
const REWORK_SEASON = 2;

function parseSeasonKey(key: string): { year: number; season: number } | null {
  const match = /^Y(\d+)S(\d+)/.exec(key);
  return match ? { year: Number(match[1]), season: Number(match[2]) } : null;
}

export function operatorsLocked(key: string): boolean {
  const parsed = parseSeasonKey(key);
  if (!parsed) return false;
  return (
    parsed.year > LOCK_YEAR ||
    (parsed.year === LOCK_YEAR && parsed.season >= LOCK_SEASON)
  );
}

export function soloOperators(key: string): string {
  const parsed = parseSeasonKey(key);
  if (!parsed) return "Recruit";
  const reworked =
    parsed.year > REWORK_YEAR ||
    (parsed.year === REWORK_YEAR && parsed.season >= REWORK_SEASON);
  return reworked ? "Striker and Sentry" : "Recruit";
}

export function seasonRank(key: string): number {
  const parsed = parseSeasonKey(key);
  return parsed ? parsed.year * 10 + parsed.season : 0;
}
