use clap::Parser;
use discord_client_gateway::events::Event;
use discord_client_gateway::gateway::GatewayClient;
use discord_client_rest::rest::RestClient;
use std::time::Duration;

mod cli;
mod fingerprint;
mod gateway;
mod science;
mod stream;
mod utils;

use crate::cli::Cli;
use crate::gateway::{open_gateway, stream_only};
use crate::utils::{pick_games, print_progress, resolve_guild};
use stream::StreamFarm;
use tokio::time::{Instant, interval, sleep, sleep_until};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let rotate = Duration::from_secs_f64(cli.rotate_minutes * 60.0);
    let token = std::fs::read_to_string(&cli.token_file)
        .expect("token file")
        .trim()
        .to_string();

    let ws = match open_gateway(&token).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("[check] gateway failed, nothing sent: {e}");
            return;
        }
    };
    println!("[check] gateway reached READY");

    let rest = RestClient::connect(token.clone(), None, None, None, None)
        .await
        .expect("rest connect");

    let total = if cli.hours > 0.0 {
        cli.hours
    } else {
        cli.claim_total_hours
    };
    if total > 0.0 {
        let games = if cli.claim_games > 0 {
            cli.claim_games
        } else if cli.games > 0 {
            cli.games
        } else {
            100
        };
        let opts = science::PlanOptions {
            total_hours: total,
            games,
            session_max: cli.claim_session_max,
            hb_minutes: cli.claim_heartbeat_min,
            batch: cli.claim_batch,
            delay_ms: cli.claim_batch_delay_ms,
            dry_run: cli.dry_run,
        };
        if let Err(e) = science::claim_plan(&rest, opts).await {
            eprintln!("[science] {e}");
        }
        ws.close().await;
        return;
    }

    if cli.claim_games > 0 {
        if let Err(e) = science::claim(
            &rest,
            cli.claim_games,
            cli.claim_hours,
            cli.claim_heartbeat_min,
            cli.claim_batch,
            cli.claim_batch_delay_ms,
            cli.dry_run,
        )
        .await
        {
            eprintln!("[science] {e}");
        }
        ws.close().await;
        return;
    }

    let games = if cli.no_games {
        Vec::new()
    } else {
        let games = pick_games(
            rest.application()
                .get_detectable_games()
                .await
                .expect("detectable games"),
            cli.games,
        );
        assert!(!games.is_empty(), "no detectable game found");
        println!("[farm] {} games in rotation", games.len());
        games
    };

    let mut leave_guild = None;
    let mut extras: Vec<(Option<u64>, u64)> = Vec::new();
    let mut stream = match cli.stream_channel.first().copied() {
        Some(channel) => {
            let guild = resolve_guild(&rest, cli.stream_guild, channel).await;
            println!("[stream] channel {channel} guild {guild:?}");
            leave_guild = guild;
            Some(StreamFarm::new(guild, channel, rest.user_id))
        }
        None => None,
    };

    for &channel in cli.stream_channel.iter().skip(1) {
        let guild = resolve_guild(&rest, cli.stream_guild, channel).await;
        println!("[stream] extra channel {channel} guild {guild:?}");
        extras.push((guild, channel));
    }

    let mut gateway = GatewayClient::builder(token.clone())
        .desktop()
        .connect()
        .await
        .expect("gateway connect");
    gateway.status = cli.status.into();

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let mut extra_tasks = Vec::new();
    for (guild, channel) in extras {
        let token = token.clone();
        let status = cli.status;
        let user_id = rest.user_id;
        let game = games.first().map(|g| (g.name.clone(), g.id));
        let stop_rx = stop_rx.clone();
        extra_tasks.push(tokio::spawn(async move {
            stream_only(token, status, guild, channel, user_id, game, stop_rx).await;
        }));
        sleep(Duration::from_millis(1200)).await;
    }

    if cli.leave {
        gateway
            .update_voice_state(leave_guild, None, false, false, None)
            .await
            .expect("leave voice");
        sleep(Duration::from_secs(3)).await;
        gateway.graceful_shutdown().await.ok();
        return;
    }

    print_progress(&rest).await;

    let mut poll = interval(Duration::from_secs_f64(cli.poll_minutes * 60.0));
    poll.tick().await;
    let mut tick = interval(Duration::from_secs(1));

    let mut index = 0;
    if !games.is_empty() {
        let game = &games[index];
        gateway
            .set_game_activity(&game.name, game.id, None)
            .await
            .expect("set activity");
        println!("[farm] playing {} ({})", game.name, game.id);
    }
    let mut switch_at = Instant::now() + rotate;

    let mut terminate = Box::pin(tokio::signal::ctrl_c());

    loop {
        tokio::select! {
            event = gateway.next_event() => match event {
                Ok(event) => {
                    match &event {
                        Event::Ready(_) => {
                            println!("[gateway] ready");
                            if !games.is_empty() {
                                let game = &games[index];
                                if let Err(e) = gateway.set_game_activity(&game.name, game.id, None).await {
                                    eprintln!("[farm] set activity failed: {e}");
                                }
                            }
                        }
                        Event::Resumed(_) => println!("[gateway] resumed"),
                        _ => {}
                    }
                    if let Some(stream) = stream.as_mut() {
                        stream.on_event(&event);
                        stream.step(&mut gateway).await;
                    }
                }
                Err(e) => {
                    eprintln!("[gateway] {e}");
                    sleep(Duration::from_secs(5)).await;
                }
            },
            _ = tick.tick() => {
                if let Some(stream) = stream.as_mut() {
                    stream.step(&mut gateway).await;
                }
            }
            _ = poll.tick() => print_progress(&rest).await,
            _ = sleep_until(switch_at), if !games.is_empty() => {
                if cli.once && index + 1 >= games.len() {
                    break;
                }
                print_progress(&rest).await;
                index = (index + 1) % games.len();
                let game = &games[index];
                gateway.clear_game_activity().await.ok();
                sleep(Duration::from_secs(2)).await;
                if let Err(e) = gateway.set_game_activity(&game.name, game.id, None).await {
                    eprintln!("[farm] set activity failed: {e}");
                }
                println!("[farm] playing {} ({})", game.name, game.id);
                switch_at = Instant::now() + rotate;
            }
            _ = &mut terminate => break,
        }
    }

    if let Some(stream) = stream.as_mut() {
        stream.stop(&mut gateway).await;
    }
    let _ = stop_tx.send(true);
    for task in extra_tasks {
        let _ = task.await;
    }
    gateway.clear_game_activity().await.ok();
    print_progress(&rest).await;
    gateway.graceful_shutdown().await.ok();
    ws.close().await;
}
