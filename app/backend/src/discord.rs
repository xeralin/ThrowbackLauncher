use serde::Deserialize;

use crate::github::json;

const INVITE: &str = "r6s-operation-throwback-1092820800203141130";

#[derive(Deserialize)]
struct Invite {
    approximate_member_count: u64,
}

pub fn member_count() -> Option<u64> {
    let url = format!("https://discord.com/api/v10/invites/{INVITE}?with_counts=true");
    let invite: Invite = json(&url).ok()?;
    Some(invite.approximate_member_count)
}
