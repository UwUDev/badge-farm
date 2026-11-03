use crate::fingerprint;
use discord_client_rest::rest::{RequestProperties, RestClient};

use discord_client_structs::structs::application::detectable::DetectableApplication;
use rand::Rng;
use rand::seq::SliceRandom;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

const MS_HOUR: f64 = 3_600_000.0;
const RUN_HOURS: f64 = 250.0;
const TARGET_HOURS_PER_DAY: f64 = 10.0;
const RUN_PAUSE_SECS: u64 = 90;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn win32_exe(app: &DetectableApplication) -> Option<&str> {
    app.executables
        .iter()
        .find(|e| e.os == "win32" && !e.is_launcher && !e.name.is_empty())
        .map(|e| e.name.as_str())
}

struct Ids {
    hb: String,
    sig: String,
    seq: u64,
    locale: String,
}

#[derive(Clone, Copy)]
struct Game<'a> {
    id: &'a str,
    name: &'a str,
    exe: &'a str,
    fp: &'a str,
}

struct Session {
    game: usize,
    start: i64,
    duration: i64,
}

impl Ids {
    fn new(locale: &str, heartbeat_session_id: String, launch_signature: String) -> Self {
        Self {
            hb: heartbeat_session_id,
            sig: launch_signature,
            seq: 0,
            locale: locale.to_string(),
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn heartbeat(
        &mut self,
        game: Game,
        session_id: &str,
        ts: i64,
        tracked: i64,
        initial: bool,
        last: bool,
    ) -> Value {
        json!({
            "type": "running_game_heartbeat",
            "properties": {
                "client_track_timestamp": ts,
                "client_heartbeat_session_id": self.hb,
                "event_sequence_number": self.next_seq(),
                "game_id": game.id,
                "game_name": game.name,
                "game_metadata": null,
                "game_executable": game.exe,
                "game_detection_enabled": true,
                "initial_heartbeat": initial,
                "final_heartbeat": last,
                "game_session_id": session_id,
                "duration_tracked_ms": tracked,
                "rtc_connection_id": null,
                "media_session_id": null,
                "launch_signature": self.sig,
                "client_app_state": "focused",
                "client_send_timestamp": ts,
            }
        })
    }

    fn launch(&mut self, game: Game, ts: i64) -> Value {
        json!({
            "type": "launch_game",
            "properties": {
                "client_track_timestamp": ts,
                "client_heartbeat_session_id": self.hb,
                "event_sequence_number": self.next_seq(),
                "game": game.name,
                "game_id": game.id,
                "verified": true,
                "elevated": false,
                "is_launcher": false,
                "game_platform": "desktop",
                "detection_method": "verified_game",
                "is_overlay_enabled": false,
                "is_overlay_game_enabled": true,
                "is_overlay_game_source": "OOP_DEFAULT_DATABASE",
                "fullscreen_type": "UNKNOWN",
                "hardware_display_count": 1,
                "overlay_method": "Disabled",
                "activity_status_enabled": true,
                "activity_status_shared_guilds": [],
                "current_user_status": "online",
                "game_detection_enabled": true,
                "executable_path": game.exe,
                "executable_fingerprint": game.fp,
                "voice_channel_id": null,
                "voice_channel_type": null,
                "voice_channel_bitrate": null,
                "voice_channel_guild_id": null,
                "hidden_by_distributor": false,
                "game_metadata": null,
                "client_performance_cpu": null,
                "client_performance_memory": null,
                "cpu_core_count": null,
                "accessibility_features": 0,
                "rendered_locale": self.locale,
                "launch_signature": self.sig,
                "client_rtc_state": null,
                "client_app_state": "focused",
                "client_send_timestamp": ts,
            }
        })
    }

    fn session_events(&mut self, game: Game, session: &Session, hb_minutes: f64) -> Vec<Value> {
        let session_id = uuid::Uuid::new_v4().to_string();
        let step = ((hb_minutes * 60_000.0) as i64).max(1);
        let mut events = Vec::new();
        events.push(self.heartbeat(game, &session_id, session.start, 0, true, false));
        events.push(self.launch(game, session.start));
        let mut tracked = 0i64;
        while tracked + step < session.duration {
            tracked += step;
            events.push(self.heartbeat(
                game,
                &session_id,
                session.start + tracked,
                step,
                false,
                false,
            ));
        }
        events.push(self.heartbeat(
            game,
            &session_id,
            session.start + session.duration,
            session.duration - tracked,
            false,
            true,
        ));
        events
    }
}

fn game_pool(apps: &[DetectableApplication]) -> Vec<(&DetectableApplication, &str)> {
    apps.iter()
        .filter_map(|a| win32_exe(a).map(|exe| (a, exe)))
        .collect()
}

fn library(games: usize, total_hours: f64) -> Vec<f64> {
    let heavy = ((games as f64 * 0.10).round() as usize).max(1).min(games);
    let mid = ((games as f64 * 0.25).round() as usize)
        .max(1)
        .min(games.saturating_sub(heavy));
    let rest = games.saturating_sub(heavy + mid);
    let mut weights = Vec::with_capacity(games + 1);
    weights.extend(std::iter::repeat_n(4.0, heavy));
    weights.extend(std::iter::repeat_n(1.5, mid));
    weights.extend(std::iter::repeat_n(1.0, rest));
    let sum: f64 = weights.iter().sum();
    weights
        .iter()
        .map(|w| total_hours * w / sum.max(1e-9))
        .collect()
}

fn schedule(
    hours: &[f64],
    session_max: f64,
    spread_days: usize,
    day_max: f64,
    back_days: usize,
) -> Result<(Vec<Session>, usize), String> {
    let mut rng = rand::rng();
    let mut chunks: Vec<(usize, f64)> = Vec::new();
    for (game, total) in hours.iter().enumerate() {
        let mut left = *total;
        while left > 0.05 {
            let take = left.min(session_max);
            chunks.push((game, take));
            left -= take;
        }
    }
    chunks.shuffle(&mut rng);
    let active_days = (spread_days as f64 * 0.85).max(1.0);
    let per_day = ((chunks.len() as f64 / active_days).ceil() as usize).max(1);

    let mut queue = chunks.into_iter().peekable();
    let mut plan = Vec::new();
    let mut day_end = now_ms() - (back_days as i64) * 86_400_000;
    let mut days_used = 0usize;

    for _ in 0..spread_days {
        if queue.peek().is_none() {
            break;
        }
        days_used += 1;
        if rng.random_bool(0.15) {
            day_end -= 86_400_000;
            continue;
        }
        let target = per_day + rng.random_range(0usize..=1);
        let mut placed = 0usize;
        let mut play = 0.0f64;
        let mut window = 0.0f64;
        let mut session_end = day_end - (rng.random_range(0.5..=4.0) * MS_HOUR) as i64;
        while placed < target {
            let Some(&(game, hours)) = queue.peek() else {
                break;
            };
            let gap = rng.random_range(0.1..=0.5);
            if play + hours > day_max || window + hours + gap > 21.0 {
                break;
            }
            queue.next();
            let duration = (hours * MS_HOUR) as i64;
            let start = session_end - duration;
            plan.push(Session {
                game,
                start,
                duration,
            });
            placed += 1;
            play += hours;
            window += hours + gap;
            session_end = start - (gap * MS_HOUR) as i64;
        }
        day_end -= 86_400_000;
    }

    if queue.peek().is_some() {
        return Err(format!(
            "{} sessions placed, games still queued - raise --claim-spread-days or --claim-day-max-h",
            plan.len()
        ));
    }
    Ok((plan, days_used))
}

fn schedule_auto(
    hours: &[f64],
    session_max: f64,
    spread_days: usize,
    day_max: f64,
    back_days: usize,
) -> Result<(Vec<Session>, usize), String> {
    if spread_days > 0 {
        return schedule(hours, session_max, spread_days, day_max, back_days);
    }
    let total: f64 = hours.iter().sum();
    let mut spread = ((total / TARGET_HOURS_PER_DAY).ceil() as usize).max(7);
    for _ in 0..16 {
        if let Ok(plan) = schedule(hours, session_max, spread, day_max, back_days) {
            return Ok(plan);
        }
        spread = (spread as f64 * 1.4) as usize;
    }
    Err("could not fit the claim into a backdated history".to_string())
}

async fn send(
    rest: &RestClient,
    analytics: &str,
    events: Vec<Value>,
    batch: usize,
    delay_ms: u64,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let batch = batch.max(1);
    let total = events.len();
    let requests = total.div_ceil(batch);
    let mut sent = 0usize;
    for (index, chunk) in events.chunks(batch).enumerate() {
        let body = json!({"token": analytics, "events": chunk});
        let _: Value = rest
            .post("science", Some(body), Some(RequestProperties::home()))
            .await?;
        sent += chunk.len();
        if (index + 1) % 25 == 0 || index + 1 == requests {
            println!(
                "[science] {sent}/{total} events in {}/{} requests",
                index + 1,
                requests
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }
    println!("[science] done: {sent} events in {requests} requests");
    Ok(sent)
}

pub struct PlanOptions {
    pub total_hours: f64,
    pub games: usize,
    pub session_max: f64,
    pub hb_minutes: f64,
    pub spread_days: usize,
    pub day_max: f64,
    pub batch: usize,
    pub delay_ms: u64,
    pub dry_run: bool,
}

pub async fn claim_plan(
    rest: &RestClient,
    opts: PlanOptions,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let PlanOptions {
        total_hours,
        games,
        session_max,
        hb_minutes,
        spread_days,
        day_max,
        batch,
        delay_ms,
        dry_run,
    } = opts;
    let analytics = rest.science().analytics_token().await?;
    let apps = rest.application().get_detectable_games().await?;
    let mut pool = game_pool(&apps);
    if pool.len() < games {
        return Err(format!("only {} detectable games with a win32 exe", pool.len()).into());
    }
    pool.shuffle(&mut rand::rng());
    pool.truncate(games);

    let mut ids = Ids::new(rest.locale(), rest.heartbeat_session_id(), rest.launch_signature());
    let machine = crate::fingerprint::machine_guid();
    let user = rest.user_id.to_string();
    let mut left = total_hours;
    let mut back_days = 0usize;
    let mut first_run = true;
    while left > 0.0 {
        let chunk = left.min(RUN_HOURS);
        let hours = library(games, chunk);
        let (mut plan, days_used) =
            schedule_auto(&hours, session_max, spread_days, day_max, back_days)?;
        plan.sort_by_key(|s| s.start);
        back_days += days_used.max(1);

        let mut events = Vec::new();
        for session in &plan {
            let (app, exe) = pool[session.game];
            let id = app.id.to_string();
            let fp = fingerprint::for_game(&id, exe, &machine, &user);
            let game = Game {
                id: &id,
                name: &app.name,
                exe,
                fp: &fp,
            };
            events.extend(ids.session_events(game, session, hb_minutes));
        }

        let planned: f64 = plan.iter().map(|s| s.duration as f64 / MS_HOUR).sum();
        let requests = events.len().div_ceil(batch.max(1));
        println!(
            "[science] plan: {} games, {:.0}h, {} sessions, {:.1}h/day over {} days ending {}d ago, {} events, {} requests",
            games,
            planned,
            plan.len(),
            planned / days_used.max(1) as f64,
            days_used,
            back_days - days_used.max(1),
            events.len(),
            requests
        );
        if events.is_empty() {
            break;
        }

        if dry_run {
            if first_run {
                let sample = events
                    .iter()
                    .find(|e| e["type"] == "launch_game")
                    .unwrap_or(&events[0]);
                println!(
                    "[science] sample event: {}",
                    serde_json::to_string_pretty(sample).unwrap_or_default()
                );
                first_run = false;
            }
        } else {
            send(rest, &analytics, events, batch, delay_ms).await?;
        }

        left -= chunk;
        if left > 0.0 && !dry_run {
            println!("[science] {left:.0}h left, pausing {RUN_PAUSE_SECS}s");
            tokio::time::sleep(std::time::Duration::from_secs(RUN_PAUSE_SECS)).await;
        }
    }
    Ok(())
}

pub async fn claim(
    rest: &RestClient,
    count: usize,
    hours: f64,
    hb_minutes: f64,
    batch: usize,
    delay_ms: u64,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let analytics = rest.science().analytics_token().await?;
    let apps = rest.application().get_detectable_games().await?;
    let mut pool = game_pool(&apps);
    if pool.is_empty() {
        return Err("no detectable games with a win32 executable".into());
    }
    pool.shuffle(&mut rand::rng());
    let picked: Vec<(&DetectableApplication, &str)> = pool.into_iter().take(count).collect();

    let duration = (hours * MS_HOUR) as i64;
    let start = now_ms() - duration;
    let mut ids = Ids::new(rest.locale(), rest.heartbeat_session_id(), rest.launch_signature());
    let machine = crate::fingerprint::machine_guid();
    let user = rest.user_id.to_string();
    let mut events = Vec::new();
    for (app, exe) in &picked {
        let session = Session {
            game: 0,
            start,
            duration,
        };
        let id = app.id.to_string();
        let fp = fingerprint::for_game(&id, exe, &machine, &user);
        let game = Game {
            id: &id,
            name: &app.name,
            exe,
            fp: &fp,
        };
        events.extend(ids.session_events(game, &session, hb_minutes));
    }

    println!(
        "[science] {} games x {}h = {:.0}h claimed ({} events, dry_run={})",
        picked.len(),
        hours,
        picked.len() as f64 * hours,
        events.len(),
        dry_run
    );
    if dry_run {
        let sample = events
            .iter()
            .find(|e| e["type"] == "launch_game")
            .unwrap_or(&events[0]);
        println!(
            "[science] sample event: {}",
            serde_json::to_string_pretty(sample).unwrap_or_default()
        );
        return Ok(());
    }
    send(rest, &analytics, events, batch, delay_ms).await?;
    Ok(())
}
