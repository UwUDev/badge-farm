use crate::cli::Status;
use crate::stream::StreamFarm;
use discord_client_gateway::events::Event;
use discord_client_gateway::gateway::GatewayClient;
use std::time::Duration;
use tokio::time::{Instant, interval, sleep};

pub struct Ws {
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Ws {
    pub(crate) async fn close(mut self) {
        if let Some(tx) = self.shutdown.take() {
            tx.send(()).ok();
        }
        self.task.await.ok();
    }
}

pub async fn open_gateway(token: &str) -> Result<Ws, Box<dyn std::error::Error + Send + Sync>> {
    let mut gateway = tokio::time::timeout(
        Duration::from_secs(30),
        GatewayClient::builder(token.to_string())
            .desktop()
            .connect(),
    )
    .await
    .map_err(|_| "gateway connect timed out")??;

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if Instant::now() > deadline {
            return Err("gateway never reached READY".into());
        }
        match tokio::time::timeout(Duration::from_secs(10), gateway.next_event()).await {
            Ok(Ok(Event::Ready(_))) => break,
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => return Err(e),
            Err(_) => continue,
        }
    }

    let (shutdown, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                event = gateway.next_event() => {
                    if event.is_err() {
                        break;
                    }
                }
            }
        }
        gateway.graceful_shutdown().await.ok();
    });

    Ok(Ws {
        shutdown: Some(shutdown),
        task,
    })
}

pub async fn stream_only(
    token: String,
    status: Status,
    guild: Option<u64>,
    channel: u64,
    user_id: u64,
    game: Option<(String, u64)>,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let mut gateway = loop {
            let attempt = tokio::time::timeout(
                Duration::from_secs(45),
                GatewayClient::builder(token.clone()).desktop().connect(),
            )
            .await;
            match attempt {
                Ok(Ok(gateway)) => break gateway,
                Ok(Err(e)) => eprintln!("[stream {channel}] connect failed: {e}"),
                Err(_) => eprintln!("[stream {channel}] connect timed out"),
            }
            tokio::select! {
                _ = sleep(Duration::from_secs(5)) => {}
                _ = stop.changed() => return,
            }
        };
        gateway.status = status.into();
        if let Some((name, id)) = game.clone()
            && let Err(e) = gateway.set_game_activity(&name, id, None).await
        {
            eprintln!("[stream {channel}] set activity failed: {e}");
        }
        let mut stream = StreamFarm::new(guild, channel, user_id);
        let mut tick = interval(Duration::from_secs(1));

        let mut reconnect = false;
        loop {
            tokio::select! {
                event = gateway.next_event() => match event {
                    Ok(event) => {
                        stream.on_event(&event);
                        stream.step(&mut gateway).await;
                    }
                    Err(e) => {
                        eprintln!("[stream {channel}] {e}");
                        reconnect = true;
                        break;
                    }
                },
                _ = tick.tick() => {
                    if stream.missing_supplemental() {
                        eprintln!("[stream {channel}] no READY_SUPPLEMENTAL after 10s, reconnecting");
                        reconnect = true;
                        break;
                    }
                    stream.step(&mut gateway).await;
                },
                _ = stop.changed() => break,
            }
        }

        stream.stop(&mut gateway).await;
        gateway.clear_game_activity().await.ok();
        gateway.graceful_shutdown().await.ok();

        if !reconnect || *stop.borrow() {
            return;
        }
        sleep(Duration::from_secs(2)).await;
    }
}
