use clap::{Parser, ValueEnum};
use discord_client_structs::structs::user::status::StatusType;

#[derive(Parser)]
#[command(about = "Farms Discord badge progress (Game Time, Game Variety, Streaming)")]
pub struct Cli {
    /// File containing the account token
    #[arg(short, long, default_value = "token.txt")]
    pub token_file: String,

    /// Number of games: rotation count, or how many games to claim across with --hours
    #[arg(short, long, default_value_t = 0)]
    pub games: usize,

    /// Minutes spent on each game before switching
    #[arg(short, long, default_value_t = 10.0)]
    pub rotate_minutes: f64,

    /// Minutes between badge progress checks
    #[arg(short, long, default_value_t = 5.0)]
    pub poll_minutes: f64,

    /// Stop after going through every game once
    #[arg(long)]
    pub once: bool,

    /// Do not play games, only stream
    #[arg(long, requires = "stream_channel")]
    pub no_games: bool,

    /// Voice channel to join and stream in
    #[arg(short = 'c', long)]
    pub stream_channel: Vec<u64>,

    /// Status shown while farming
    #[arg(short, long, value_enum, default_value_t = Status::Idle)]
    pub status: Status,

    /// Leave any voice channel and exit
    #[arg(long)]
    pub leave: bool,

    /// Guild of the voice channel (looked up from the channel when omitted)
    #[arg(long, requires = "stream_channel")]
    pub stream_guild: Option<u64>,

    /// Hours of playtime to claim across games, then exit
    #[arg(long, default_value_t = 0.0)]
    pub hours: f64,

    /// Executable to fingerprint for the launch events of a claim
    #[arg(long)]
    pub fingerprint_exe: Option<std::path::PathBuf>,

    /// Print the science plan without posting
    #[arg(long, visible_alias = "claim-dry-run")]
    pub dry_run: bool,

    /// Claim playtime on N detectable games via the science endpoint, then exit
    #[arg(long, default_value_t = 0, hide = true)]
    pub claim_games: usize,

    /// Hours to claim per game
    #[arg(long, default_value_t = 1.0, hide = true)]
    pub claim_hours: f64,

    /// Events per science request
    #[arg(long, default_value_t = 50, hide = true)]
    pub claim_batch: usize,

    /// Milliseconds to wait between science requests
    #[arg(long, default_value_t = 1000, hide = true)]
    pub claim_batch_delay_ms: u64,

    /// Plan mode: total hours to claim across a backdated history
    #[arg(long, default_value_t = 0.0, hide = true)]
    pub claim_total_hours: f64,

    /// Plan mode: maximum hours per session (sessions above ~15h get rejected)
    #[arg(long, default_value_t = 3.0, hide = true)]
    pub claim_session_max: f64,

    /// Plan mode: minutes between heartbeats inside a session
    #[arg(long, default_value_t = 180.0, hide = true)]
    pub claim_heartbeat_min: f64,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Status {
    Online,
    Idle,
    Dnd,
}

impl From<Status> for StatusType {
    fn from(status: Status) -> Self {
        match status {
            Status::Online => StatusType::Online,
            Status::Idle => StatusType::AFK,
            Status::Dnd => StatusType::DoNotDisturb,
        }
    }
}
