export async function fetchMemberCount(invite: string): Promise<number | null> {
  try {
    const res = await fetch(
      `https://discord.com/api/v10/invites/${invite}?with_counts=true`,
    );
    if (!res.ok) return null;
    const data = await res.json();
    return typeof data.approximate_member_count === "number"
      ? data.approximate_member_count
      : null;
  } catch {
    return null;
  }
}
