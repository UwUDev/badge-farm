# badge-farm

Keeps a Discord account playing detectable games and streaming in a voice channel to progress the Game Time, Game Variety and Streaming badges.

Built on [discord-client-rs](https://github.com/UwUDev/discord-client-rs).

## Usage

Put your token in a file (`token.txt` by default, git-ignored), then:

    cargo run --release -- -g 5 -r 15 -c <voice channel id>

Ctrl+C stops it cleanly: the stream is closed, the voice channel is left and the presence is cleared.

## Options

    -t, --token-file <file>        default token.txt
    -g, --games <n>                games to rotate through, or games to claim across with --hours
    -r, --rotate-minutes <min>     time spent on each game (default 10)
    -p, --poll-minutes <min>       how often badge progress is printed (default 5)
        --once                     stop after going through every game once
    -c, --stream-channel <id>      voice channel to stream in
        --stream-guild <id>        guild of the channel, looked up automatically if omitted
        --no-games                 don't play anything, only stream
        --leave                    leave the voice channel and exit
        --hours <hours>            claim that much playtime across games, then exit
        --dry-run                  print the claim plan without posting

Badge progress is also printed before every game switch.

## Claiming playtime

    cargo run --release -- --hours 5000 --games 100

Claims that many hours of play across that many detected games and exits. The run is split
into 250 h chunks with a pause between them, three-hour sessions, one delta per session.
`--dry-run` prints the plan for every chunk without posting.

## Streaming

If the account is already in a voice channel somewhere else, nothing is sent and the program waits for it to leave. After a reconnection it checks again before joining.

No audio or video is sent, only the voice state and the stream creation. It is not known whether Discord counts that towards the badge.

## Notes

- Don't restart it in a loop: every start does REST requests and ends up rate limited (429).
- To keep it running in the background: `nohup cargo run --release -- ... > farm.log 2>&1 &`, then `pkill -TERM badge-farm` to stop it cleanly.
- Badge counters move in hours and games, not minutes.
