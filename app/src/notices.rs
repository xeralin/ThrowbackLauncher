use gpui::{App, Entity, EventEmitter, SharedString, Subscription};

use crate::model::Launcher;
use crate::model::cheatengine::CheatEngineEvent;
use crate::model::download::{DownloadEvent, Outcome};
use crate::model::launch::LaunchEvent;
use crate::model::liberator::LiberatorEvent;
use crate::model::rvpn::RvpnEvent;
use crate::model::settings::SettingsEvent;
use crate::model::update::UpdateEvent;
use crate::ui::toast;

pub const RATE_LIMIT: &str = "rate-limit";
const LIBERATOR: &str = "liberator";

type Notice = (SharedString, Option<&'static str>);

fn plain(text: impl Into<SharedString>) -> Option<Notice> {
    Some((text.into(), None))
}

fn rejected(event: &SettingsEvent) -> Option<Notice> {
    match event {
        SettingsEvent::UsernameRejected(message) | SettingsEvent::Rejected(message) => {
            plain(message.clone())
        }
        SettingsEvent::CacheCleared => plain("Cache cleared"),
        _ => None,
    }
}

fn attached(event: &LiberatorEvent) -> Option<Notice> {
    match event {
        LiberatorEvent::Error(message) => Some((message.clone(), Some(LIBERATOR))),
        LiberatorEvent::Exclusion => None,
    }
}

fn bridged(RvpnEvent::Error(message): &RvpnEvent) -> Option<Notice> {
    plain(message.clone())
}

fn trained(event: &CheatEngineEvent) -> Option<Notice> {
    match event {
        CheatEngineEvent::Error(message) => plain(message.clone()),
        CheatEngineEvent::Done => None,
    }
}

fn downloaded(event: &DownloadEvent) -> Option<Notice> {
    match event {
        DownloadEvent::Done { key, outcome } => {
            let code = key.split('_').next().unwrap_or(key);
            match outcome {
                Outcome::Done => plain(format!("{code} installed")),
                Outcome::Updated => plain(format!("{code} updated")),
                Outcome::Verified => plain(format!("{code} verified")),
                Outcome::SwitchedHm => plain(format!("{code} switched to Heated Metal")),
                Outcome::SwitchedTb => plain(format!("{code} switched to Throwback")),
                Outcome::NoSpace => plain("Not enough disk space"),
                Outcome::Cancelled | Outcome::Failed => None,
            }
        }
        DownloadEvent::PartialDeleted {
            error: Some(message),
            ..
        }
        | DownloadEvent::Error(message) => plain(message.clone()),
        DownloadEvent::RateLimited(message) => Some((message.clone(), Some(RATE_LIMIT))),
        _ => None,
    }
}

fn launched(event: &LaunchEvent) -> Option<Notice> {
    match event {
        LaunchEvent::Error(message) => plain(message.clone()),
        LaunchEvent::FilesMissing { .. } => None,
    }
}

fn updated(event: &UpdateEvent) -> Option<Notice> {
    match event {
        UpdateEvent::Done(component, Ok(())) => plain(format!("{} updated", component.name())),
        UpdateEvent::Done(_, Err(message)) => plain(message.clone()),
        UpdateEvent::Error(message) => plain(*message),
    }
}

fn announce<E: 'static>(
    launcher: &Entity<Launcher>,
    notice: fn(&E) -> Option<Notice>,
    cx: &mut App,
) -> Subscription
where
    Launcher: EventEmitter<E>,
{
    cx.subscribe(launcher, move |_, event: &E, cx| {
        if let Some((text, key)) = notice(event) {
            toast::show(text, key, cx);
        }
    })
}

pub fn watch(launcher: &Entity<Launcher>, cx: &mut App) -> [Subscription; 7] {
    [
        announce(launcher, attached, cx),
        announce(launcher, bridged, cx),
        announce(launcher, trained, cx),
        announce(launcher, rejected, cx),
        announce(launcher, downloaded, cx),
        announce(launcher, launched, cx),
        announce(launcher, updated, cx),
    ]
}

#[cfg(test)]
mod tests {
    use backend::update::Component;

    use super::*;
    use crate::model::Edition;
    use crate::model::download::LoginKind;

    fn shown(notice: Option<Notice>) -> Option<(String, Option<&'static str>)> {
        notice.map(|(text, key)| (text.to_string(), key))
    }

    fn text(content: &str) -> Option<(String, Option<&'static str>)> {
        Some((content.to_owned(), None))
    }

    #[test]
    fn download_outcomes_name_the_season_code() {
        let done = |outcome| {
            shown(downloaded(&DownloadEvent::Done {
                key: "Y5S3_ShadowLegacy".into(),
                outcome,
            }))
        };
        assert_eq!(done(Outcome::Done), text("Y5S3 installed"));
        assert_eq!(done(Outcome::Updated), text("Y5S3 updated"));
        assert_eq!(done(Outcome::Verified), text("Y5S3 verified"));
        assert_eq!(
            done(Outcome::SwitchedHm),
            text("Y5S3 switched to Heated Metal")
        );
        assert_eq!(
            done(Outcome::SwitchedTb),
            text("Y5S3 switched to Throwback")
        );
        assert_eq!(done(Outcome::NoSpace), text("Not enough disk space"));
        assert_eq!(done(Outcome::Cancelled), None);
        assert_eq!(done(Outcome::Failed), None);
    }

    #[test]
    fn download_errors_pass_through_and_only_the_rate_limit_has_a_key() {
        let removed = |error: Option<&str>| DownloadEvent::PartialDeleted {
            key: "Y1S0_Vanilla".into(),
            error: error.map(|message| message.to_owned().into()),
        };
        assert_eq!(
            shown(downloaded(&DownloadEvent::Error(
                "Steam login failed".into()
            ))),
            text("Steam login failed")
        );
        assert_eq!(
            shown(downloaded(&removed(Some("Nothing to remove")))),
            text("Nothing to remove")
        );
        assert_eq!(shown(downloaded(&removed(None))), None);
        let limit = "GitHub rate limit reached, try again in 2 minutes";
        assert_eq!(
            shown(downloaded(&DownloadEvent::RateLimited(limit.into()))),
            Some((limit.to_owned(), Some("rate-limit")))
        );
        let silent = [
            DownloadEvent::LoginRequired(Some(LoginKind::Account)),
            DownloadEvent::LoginRequired(None),
            DownloadEvent::DiskSpaceRequired(1.5),
            DownloadEvent::HmArchivePicked(true),
            DownloadEvent::FilesMissing {
                key: "Y9S2_NewBlood".into(),
                library: "/games".into(),
            },
        ];
        for event in silent {
            assert_eq!(shown(downloaded(&event)), None, "{event:?}");
        }
    }

    #[test]
    fn launch_update_and_settings_events_show_their_message() {
        assert_eq!(
            shown(launched(&LaunchEvent::Error("R6S is launching".into()))),
            text("R6S is launching")
        );
        let missing = LaunchEvent::FilesMissing {
            edition: Edition {
                key: "Y5S3_ShadowLegacy".into(),
                hm: true,
            },
            library: "/games".into(),
        };
        assert_eq!(shown(launched(&missing)), None);
        for (component, name) in [
            (Component::Launcher, "Throwback Launcher"),
            (Component::DepotDownloader, "DepotDownloader"),
            (Component::SevenZip, "7z"),
            (Component::ThrowbackLoader, "ThrowbackLoader"),
            (Component::HeatedMetal, "Heated Metal"),
        ] {
            assert_eq!(
                shown(updated(&UpdateEvent::Done(component, Ok(())))),
                text(&format!("{name} updated"))
            );
        }
        let failure = Err("7z update failed".into());
        assert_eq!(
            shown(updated(&UpdateEvent::Done(Component::SevenZip, failure))),
            text("7z update failed")
        );
        assert_eq!(
            shown(updated(&UpdateEvent::Error("A download is running"))),
            text("A download is running")
        );
        let invalid = SettingsEvent::UsernameRejected("Username cannot be empty".into());
        assert_eq!(shown(rejected(&invalid)), text("Username cannot be empty"));
        assert_eq!(shown(rejected(&SettingsEvent::UsernameSaved)), None);
    }
}
