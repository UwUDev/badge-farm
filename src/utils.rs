use discord_client_rest::rest::{RequestProperties, RestClient};
use discord_client_structs::structs::application::detectable::DetectableApplication;
use discord_client_structs::structs::channel::Channel;
use indicatif::{ProgressBar, ProgressStyle};
use rand::seq::SliceRandom;

const GAME_TIME_BADGE: u64 = 20;
const GAME_VARIETY_BADGE: u64 = 21;
const STREAMING_BADGE: u64 = 19;

pub fn pick_games(all: Vec<DetectableApplication>, count: usize) -> Vec<DetectableApplication> {
    let mut candidates: Vec<_> = all
        .into_iter()
        .filter(|g| {
            g.executables
                .iter()
                .any(|e| e.os == "win32" && !e.is_launcher)
        })
        .collect();
    candidates.shuffle(&mut rand::rng());
    if count > 0 {
        candidates.truncate(count);
    }
    candidates
}

pub async fn print_progress(rest: &RestClient) {
    let badges = match rest.user().get_badges().await {
        Ok(badges) => badges,
        Err(e) => {
            eprintln!("[badge] fetch failed: {e}");
            return;
        }
    };
    let style =
        ProgressStyle::with_template("[badge] {prefix:22} {bar:30.cyan/blue} {pos}/{len} {msg}")
            .expect("badge progress template")
            .progress_chars("=>-");
    for badge in badges
        .badges
        .iter()
        .filter(|b| [GAME_TIME_BADGE, GAME_VARIETY_BADGE, STREAMING_BADGE].contains(&b.badge_id))
    {
        let tiers = format!("tier={:?} next={:?}", badge.current_tier, badge.next_tier);
        let Some(p) = badge.progress.as_ref().and_then(|p| p.first()) else {
            println!("[badge] {} {tiers} no progress", badge.name);
            continue;
        };
        let current = p.current.unwrap_or(0);
        let threshold = p.threshold.unwrap_or(0);
        let unit = p.unit.as_deref().unwrap_or("");
        let helper = p.progress_helper_text.as_deref().unwrap_or("");
        let bar = ProgressBar::new(threshold)
            .with_style(style.clone())
            .with_prefix(badge.name.clone())
            .with_message(format!("{unit} {tiers} ({helper})"));
        if bar.is_hidden() {
            println!(
                "[badge] {} {tiers} {current}/{threshold} {unit} ({helper})",
                badge.name
            );
            continue;
        }
        bar.set_position(current);
        bar.abandon();
    }
}

pub async fn resolve_guild(rest: &RestClient, guild: Option<u64>, channel: u64) -> Option<u64> {
    if guild.is_some() {
        return guild;
    }
    rest.get::<Channel>(
        &format!("channels/{channel}"),
        None,
        Some(RequestProperties::home()),
    )
    .await
    .expect("stream channel lookup")
    .guild_id
}
