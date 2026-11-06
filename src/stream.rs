use discord_client_gateway::events::Event;
use discord_client_gateway::gateway::GatewayClient;
use std::time::{Duration, Instant};

const STEP_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_DELAY: Duration = Duration::from_secs(30);
const SUPPLEMENTAL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Joining,
    Joined,
    Creating,
    Streaming,
}

pub struct StreamFarm {
    guild_id: Option<u64>,
    channel_id: u64,
    user_id: u64,
    voice_known: bool,
    voice_channel: Option<u64>,
    phase: Phase,
    since: Instant,
    retry_at: Instant,
    stream_key: Option<String>,
    waiting_logged: bool,
    ready_at: Option<Instant>,
    supplemental_seen: bool,
}

impl StreamFarm {
    pub fn new(guild_id: Option<u64>, channel_id: u64, user_id: u64) -> Self {
        Self {
            guild_id,
            channel_id,
            user_id,
            voice_known: true,
            voice_channel: None,
            phase: Phase::Idle,
            since: Instant::now(),
            retry_at: Instant::now(),
            stream_key: None,
            waiting_logged: false,
            ready_at: None,
            supplemental_seen: false,
        }
    }

    fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.since = Instant::now();
    }

    fn reset(&mut self) {
        self.voice_channel = None;
        self.stream_key = None;
        self.waiting_logged = false;
        self.set_phase(Phase::Idle);
    }

    fn owns_stream(&self, key: &str) -> bool {
        key.ends_with(&format!(":{}", self.user_id))
    }

    pub fn missing_supplemental(&self) -> bool {
        self.ready_at
            .is_some_and(|at| !self.supplemental_seen && at.elapsed() > SUPPLEMENTAL_TIMEOUT)
    }

    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::Ready(_) => {
                self.reset();
                self.voice_known = true;
                self.ready_at = Some(Instant::now());
                self.supplemental_seen = false;
            }
            Event::ReadySupplemental(supplemental) => {
                self.voice_known = true;
                self.supplemental_seen = true;
                self.voice_channel = supplemental
                    .guilds
                    .iter()
                    .filter(|g| self.guild_id.is_none_or(|id| g.id == id))
                    .filter_map(|g| g.voice_states.as_ref())
                    .flatten()
                    .find(|v| v.user_id == self.user_id)
                    .and_then(|v| v.channel_id);
            }
            Event::VoiceStateUpdate(update) if update.voice_state.user_id == self.user_id => {
                let channel = update.voice_state.channel_id;
                if channel == Some(self.channel_id) {
                    self.voice_channel = channel;
                    if self.phase == Phase::Joining {
                        println!("[stream] joined voice channel {}", self.channel_id);
                        self.set_phase(Phase::Joined);
                    }
                } else if channel.is_none() {
                    self.voice_channel = None;
                    if self.phase != Phase::Idle && self.phase != Phase::Joining {
                        println!("[stream] left the voice channel, restarting");
                        self.stream_key = None;
                        self.set_phase(Phase::Idle);
                    }
                }
            }
            Event::StreamCreate(created) if self.owns_stream(&created.stream.stream_key) => {
                println!("[stream] started {}", created.stream.stream_key);
                self.stream_key = Some(created.stream.stream_key.clone());
                self.set_phase(Phase::Streaming);
            }
            Event::StreamDelete(deleted) if self.owns_stream(&deleted.stream_key) => {
                println!("[stream] stopped by server: {}", deleted.reason);
                self.stream_key = None;
                self.retry_at = Instant::now() + RETRY_DELAY;
                if self.voice_channel == Some(self.channel_id) {
                    self.set_phase(Phase::Joined);
                } else {
                    self.set_phase(Phase::Idle);
                }
            }
            _ => {}
        }
    }

    pub async fn step(&mut self, gateway: &mut GatewayClient) {
        if !self.voice_known {
            return;
        }
        let result = match self.phase {
            Phase::Idle => self.step_idle(gateway).await,
            Phase::Joining if self.since.elapsed() > STEP_TIMEOUT => {
                println!("[stream] join timed out");
                self.set_phase(Phase::Idle);
                Ok(())
            }
            Phase::Joined if Instant::now() >= self.retry_at => {
                self.set_phase(Phase::Creating);
                gateway
                    .create_stream(
                        if self.guild_id.is_some() {
                            "guild"
                        } else {
                            "call"
                        },
                        self.guild_id,
                        self.channel_id,
                        None,
                    )
                    .await
            }
            Phase::Creating if self.since.elapsed() > STEP_TIMEOUT => {
                println!("[stream] stream creation timed out");
                self.retry_at = Instant::now() + RETRY_DELAY;
                self.set_phase(Phase::Joined);
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            eprintln!("[stream] {e}");
        }
    }

    async fn step_idle(
        &mut self,
        gateway: &mut GatewayClient,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(channel) = self.voice_channel {
            if !self.waiting_logged {
                println!(
                    "[stream] account already in voice channel {channel}, waiting for it to leave"
                );
                self.waiting_logged = true;
            }
            return Ok(());
        }
        self.waiting_logged = false;
        if Instant::now() < self.retry_at {
            return Ok(());
        }
        println!("[stream] joining voice channel {}", self.channel_id);
        self.set_phase(Phase::Joining);
        gateway
            .update_voice_state(self.guild_id, Some(self.channel_id), true, false, None)
            .await
    }

    pub async fn stop(&mut self, gateway: &mut GatewayClient) {
        if let Some(key) = self.stream_key.take() {
            gateway.delete_stream(&key).await.ok();
        }
        if matches!(
            self.phase,
            Phase::Joining | Phase::Joined | Phase::Creating | Phase::Streaming
        ) {
            gateway
                .update_voice_state(self.guild_id, None, false, false, None)
                .await
                .ok();
        }
        self.set_phase(Phase::Idle);
    }
}
