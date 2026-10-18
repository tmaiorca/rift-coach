//! `rc-cli mock-live`: serves a scripted game on the Live Client Data API
//! path, so the app's live view can be tried without launching League.

use std::time::{Duration, Instant};

use anyhow::Context;
use rc_live::mock::MockGame;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub struct Options {
    pub port: u16,
    pub speed: f64,
    pub minutes: f64,
    pub champion: String,
    pub riot_id: String,
    pub mode: String,
    pub lose: bool,
    pub loading_s: f64,
    pub hold_s: f64,
}

struct Script {
    game: MockGame,
    started: Instant,
    loading_s: f64,
    speed: f64,
}

impl Respond for Script {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let real = self.started.elapsed().as_secs_f64();
        if real < self.loading_s {
            return ResponseTemplate::new(404).set_body_string(r#"{"errorCode":"RESOURCE_NOT_FOUND"}"#);
        }
        let t = (real - self.loading_s) * self.speed;
        ResponseTemplate::new(200).set_body_json(self.game.all_game_data(t))
    }
}

pub async fn run(o: Options) -> anyhow::Result<()> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", o.port))
        .with_context(|| format!("port {} is busy (is another mock or a game running?)", o.port))?;
    let server = MockServer::builder().listener(listener).start().await;
    let game = MockGame {
        riot_id: o.riot_id.clone(),
        champion: (o.champion.clone(), o.champion.clone()),
        game_mode: o.mode.to_uppercase(),
        duration_s: o.minutes * 60.0,
        win: !o.lose,
        ..Default::default()
    };
    let speed = o.speed.max(0.1);
    Mock::given(path("/liveclientdata/allgamedata"))
        .respond_with(Script { game, started: Instant::now(), loading_s: o.loading_s, speed })
        .mount(&server)
        .await;

    let real_game_s = o.minutes * 60.0 / speed;
    println!("Mock game on {} ({} as {}, {}).", server.uri(), o.champion, o.riot_id, o.mode);
    println!("Start the app pointed at it (PowerShell):");
    println!("  $env:RIFT_COACH_LIVE_URL = \"{}\"; npm run tauri dev", server.uri());
    println!(
        "Loading screen for {:.0}s, then {:.0} game minutes at {speed}x ({:.0}s), end screen for {:.0}s.",
        o.loading_s, o.minutes, real_game_s, o.hold_s
    );
    if o.mode.eq_ignore_ascii_case("PRACTICETOOL") {
        println!("Practice tool games aren't looked up in Match-V5 afterwards (use --mode CLASSIC to try that).");
    }
    tokio::time::sleep(Duration::from_secs_f64(o.loading_s + real_game_s + o.hold_s)).await;
    println!("Game over; mock stopped.");
    Ok(())
}
