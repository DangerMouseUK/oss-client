//! Open SubSpace game client.

mod app;
mod audio;
mod config;
mod game;
mod gfx;

use std::path::PathBuf;

use game::online::ServerOptions;

const USAGE: &str = "\
Usage: oss-client [options]                (login screen)
       oss-client --server <host:port> --name <name> [options]
       oss-client --sandbox [map.lvl] [options]

Play online:
  --server <host:port>      Zone to join
  --name <name>             Player name (default: from the settings file)
  --password <pw>           Password (default empty)
  --arena <name>            Arena to enter (default: any public arena)
  --new-user                Register the name instead of expecting it to exist.
                            Subgame forgets its accounts when the zone restarts
  --ship <1-8|spec>         Ship to enter in (default spec online, 1 offline)
  --freq <n>                Team to join (in game: type =n)
  --follow <name>           Spectate, following this player
  --attach <name>           Attach to this teammate as a turret once in play
  --record <file.ossr>      Record the visit to a replay file. The login's
                            password is replaced with a marker, so a recording
                            is safe to attach to a bug report

Watch a replay:
  --replay <file.ossr>      Play a recording back (spectating, following its player)

Play offline:
  --sandbox [map.lvl]       Fly around a map (default: a built-in arena) with stock
                            settings

Options:
  --assets-dir <dir>        Graphics/sounds that override the bundled ones
                            (also OSS_ASSETS_DIR); may hold only some files
  --continuum-dir <dir>     Take graphics/sounds from a Continuum install
                            (also OSS_CONTINUUM_DIR)
  --lvz <file.lvz>          Show an LVZ file, offline or in a zone (repeatable)
  --scale <n>               Integer pixel scale (default: from display DPI)
  --vsync                   Cap frame rate to the display refresh
  --fullscreen              Borderless fullscreen
  --mute                    No sound
  --settings                Open the settings screen at start (sandbox)
  --screen <menu|helpN>     Open the menu, or help page N, at start (testing)
  --press <secs>:<key>      Press this key <secs> in, by its settings-file name
                            (Enter, Tab, KeyQ, ArrowDown...; a key that types a
                            character types it, so this drives the menu, the chat
                            line and the form; repeatable; testing)
  --hold <secs>:<key>:<for> Hold that key for <for> seconds, repeating it the way
                            a keyboard does (thrust, guns; repeatable; testing)
  --click <secs>:<x>,<y>    Click at that point <secs> in, in UI pixels (screen
                            pixels over the pixel scale) from the top-left: the
                            start-up screen, or the settings screen while it is
                            up (repeatable; testing)
  --drag <secs>:<x>,<y>:<x>,<y>
                            Drag from the first point to the second, <secs> in,
                            in the same units -- what a slider answers to, where
                            a click is not enough (repeatable; testing)
  --directory <host:port>   A directory server for the start-up screen's zone list
                            (repeatable; adds to the settings file's)
  --zones                   Start with the zone list selected
  --security <mode>         auto (OpenSec, falling back to VIE), opensec or vie
  --config <file>           Settings file, which must already exist; only the
                            default is written for you (oss-client.ini in the
                            user's settings folder)
  --screenshot <file.png>   Save a frame after 2 seconds, then exit (testing)
  --screenshot-at <secs>    When to take that screenshot
  --quit-at <secs>          Leave after this many seconds (testing; with
                            OSS_PERF set, prints the frame-time distribution)
  --autopilot               Fly and shoot automatically (testing)
  --send <secs>:<line>      Type this line in game <secs> after the arena loads,
                            as if you had typed it (repeatable; testing)
  -h, --help                Show this help";

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=off,naga=warn"),
    )
    .init();
    match parse_args(std::env::args().skip(1)) {
        Ok(None) => println!("oss-client {}\n\n{USAGE}", env!("CARGO_PKG_VERSION")),
        Ok(Some(mut opts)) => {
            if let Err(e) = apply_config(&mut opts).and_then(|()| app::run(opts)) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<app::Options>, String> {
    let mut map = None;
    let mut server = None;
    let mut name = None;
    let mut password = String::new();
    let mut arena = None;
    let mut new_user = false;
    let mut ship: Option<u8> = None;
    let mut freq: Option<i16> = None;
    let mut continuum_dir = None;
    let mut assets_dir = None;
    let mut sandbox = false;
    let mut scale = None;
    let mut vsync = false;
    let mut fullscreen = false;
    let mut screenshot = None;
    let mut screenshot_at = 2.0f32;
    let mut quit_at: Option<f32> = None;
    let mut autopilot = false;
    let mut send: Vec<(f32, String)> = Vec::new();
    let mut mute = false;
    let mut config = None;
    let mut lvz = Vec::new();
    let mut follow = None;
    let mut attach = None;
    let mut record = None;
    let mut replay = None;
    let mut open_settings = false;
    let mut open_screen = None;
    let mut press: Vec<app::ScriptedKey> = Vec::new();
    let mut pointer: Vec<app::ScriptedPoint> = Vec::new();
    let mut directories = Vec::new();
    let mut open_zones = false;
    let mut security = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--sandbox" => {
                sandbox = true;
                if args.peek().is_some_and(|a| !a.starts_with("--")) {
                    map = args.next().map(PathBuf::from);
                }
            }
            "--assets-dir" => assets_dir = Some(PathBuf::from(value("--assets-dir")?)),
            "--server" => server = Some(value("--server")?),
            "--name" => name = Some(value("--name")?),
            "--password" => password = value("--password")?,
            "--arena" => arena = Some(value("--arena")?),
            "--new-user" => new_user = true,
            "--freq" => {
                freq = Some(
                    value("--freq")?
                        .parse::<i16>()
                        .ok()
                        .filter(|f| *f >= 0)
                        .ok_or("--freq must be 0 or more")?,
                )
            }
            "--continuum-dir" => continuum_dir = Some(PathBuf::from(value("--continuum-dir")?)),
            "--ship" => {
                let v = value("--ship")?;
                ship = Some(if v.eq_ignore_ascii_case("spec") {
                    oss_session::SPECTATOR
                } else {
                    v.parse::<u8>()
                        .ok()
                        .filter(|s| (1..=8).contains(s))
                        .ok_or("--ship must be 1-8 or spec")?
                        - 1
                })
            }
            "--scale" => {
                scale = Some(
                    value("--scale")?
                        .parse::<u32>()
                        .ok()
                        .filter(|s| (1..=8).contains(s))
                        .ok_or("--scale must be 1-8")?,
                )
            }
            "--vsync" => vsync = true,
            "--fullscreen" => fullscreen = true,
            "--screenshot" => screenshot = Some(PathBuf::from(value("--screenshot")?)),
            "--screenshot-at" => {
                screenshot_at = value("--screenshot-at")?
                    .parse::<f32>()
                    .ok()
                    .filter(|s| (0.0..=600.0).contains(s))
                    .ok_or("--screenshot-at must be 0-600 seconds")?
            }
            "--quit-at" => {
                quit_at = Some(
                    value("--quit-at")?
                        .parse::<f32>()
                        .ok()
                        .filter(|s| (0.0..=3600.0).contains(s))
                        .ok_or("--quit-at must be 0-3600 seconds")?,
                )
            }
            "--autopilot" => autopilot = true,
            "--send" => {
                let v = value("--send")?;
                let (secs, line) = v
                    .split_once(':')
                    .ok_or("--send takes <secs>:<line>, e.g. 5:?go twobase")?;
                let at: f32 = secs
                    .trim()
                    .parse()
                    .map_err(|_| format!("--send: {secs:?} isn't a number of seconds"))?;
                send.push((at, line.to_string()));
            }
            "--mute" => mute = true,
            "--config" => config = Some(PathBuf::from(value("--config")?)),
            "--lvz" => lvz.push(PathBuf::from(value("--lvz")?)),
            "--follow" => follow = Some(value("--follow")?),
            "--attach" => attach = Some(value("--attach")?),
            "--record" => record = Some(PathBuf::from(value("--record")?)),
            "--replay" => replay = Some(PathBuf::from(value("--replay")?)),
            "--settings" => open_settings = true,
            "--press" => {
                let arg = value("--press")?;
                let (at, key) = scripted_key("--press", &arg)?;
                // A tap: down and up in the same frame, which is what an
                // edge-triggered screen looks at.
                press.push(app::ScriptedKey {
                    at,
                    key,
                    what: app::ScriptedEdge::Down,
                });
                press.push(app::ScriptedKey {
                    at,
                    key,
                    what: app::ScriptedEdge::Up,
                });
            }
            "--hold" => {
                let arg = value("--hold")?;
                let (rest, secs) = arg
                    .rsplit_once(':')
                    .ok_or("--hold needs <seconds>:<key>:<seconds>, e.g. 3:ArrowUp:2")?;
                let secs: f32 = secs
                    .trim()
                    .parse()
                    .map_err(|_| "--hold needs <seconds>:<key>:<seconds>, e.g. 3:ArrowUp:2")?;
                // NaN named rather than reached by negating the comparison: a
                // hold of "nan" seconds would otherwise schedule a key that
                // never comes up.
                if secs.is_nan() || secs <= 0.0 {
                    return Err("--hold needs a duration above zero".into());
                }
                let (at, key) = scripted_key("--hold", rest)?;
                press.extend(app::hold_events(at, key, secs));
            }
            "--click" => {
                let arg = value("--click")?;
                let bad = || "--click needs <seconds>:<x>,<y>, e.g. 3:640,520".to_string();
                let (at, at_xy) = arg.split_once(':').ok_or_else(bad)?;
                let (cx, cy) = at_xy.split_once(',').ok_or_else(bad)?;
                let at: f32 = at.trim().parse().map_err(|_| bad())?;
                let cx: f32 = cx.trim().parse().map_err(|_| bad())?;
                let cy: f32 = cy.trim().parse().map_err(|_| bad())?;
                // A press and a release at the same instant: a tap, which is
                // what a click-sensed thing reads.
                for what in [app::PointerEdge::Press, app::PointerEdge::Release] {
                    pointer.push(app::ScriptedPoint {
                        at,
                        x: cx,
                        y: cy,
                        what,
                    });
                }
            }
            "--drag" => {
                let arg = value("--drag")?;
                let bad =
                    || "--drag needs <seconds>:<x>,<y>:<x>,<y>, e.g. 3:408,286:460,286".to_string();
                let point = |s: &str| -> Result<(f32, f32), String> {
                    let (x, y) = s.split_once(',').ok_or_else(bad)?;
                    let x: f32 = x.trim().parse().map_err(|_| bad())?;
                    let y: f32 = y.trim().parse().map_err(|_| bad())?;
                    Ok((x, y))
                };
                let mut parts = arg.split(':');
                let (at, from, to) = match (parts.next(), parts.next(), parts.next(), parts.next())
                {
                    (Some(at), Some(from), Some(to), None) => (at, from, to),
                    _ => return Err(bad()),
                };
                let at: f32 = at.trim().parse().map_err(|_| bad())?;
                let (x1, y1) = point(from)?;
                let (x2, y2) = point(to)?;
                // Three frames apart, which is what makes this a drag rather
                // than a click: a slider is drag-sensed, and reads the pointer
                // where it is while the button is down.
                pointer.push(app::ScriptedPoint {
                    at,
                    x: x1,
                    y: y1,
                    what: app::PointerEdge::Press,
                });
                pointer.push(app::ScriptedPoint {
                    at: at + app::DRAG_STEP,
                    x: x2,
                    y: y2,
                    what: app::PointerEdge::Move,
                });
                pointer.push(app::ScriptedPoint {
                    at: at + 2.0 * app::DRAG_STEP,
                    x: x2,
                    y: y2,
                    what: app::PointerEdge::Release,
                });
            }
            "--screen" => {
                let name = value("--screen")?;
                open_screen = Some(match name.as_str() {
                    "menu" => app::StartScreen::Menu,
                    "help" => app::StartScreen::Help(0),
                    other => match other.strip_prefix("help").map(str::parse::<usize>) {
                        // "help2" is the second page, so that a screenshot can
                        // reach the pages past the first.
                        Some(Ok(n)) if n >= 1 => app::StartScreen::Help(n - 1),
                        _ => return Err("--screen must be menu, help, or help2/help3".into()),
                    },
                });
            }
            "--directory" => directories.push(value("--directory")?),
            "--zones" => open_zones = true,
            "--security" => {
                security = Some(
                    oss_session::security::SecurityMode::from_name(&value("--security")?)
                        .ok_or("--security must be auto, opensec or vie")?,
                )
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    // Soonest last, so the game screen pops them off the end as the clock passes.
    send.sort_by(|a, b| b.0.total_cmp(&a.0));
    // Soonest first: the app walks this one forwards. A stable sort keeps a
    // tap's down ahead of its up when both land on the same instant.
    press.sort_by(|a, b| a.at.total_cmp(&b.at));
    pointer.sort_by(|a, b| a.at.total_cmp(&b.at));
    if let Some(path) = replay {
        if sandbox || server.is_some() {
            return Err("--replay can't be combined with --sandbox or --server".into());
        }
        return Ok(Some(app::Options {
            mode: app::Mode::Replay(path),
            assets_dir,
            continuum_dir,
            scale,
            vsync,
            fullscreen,
            screenshot,
            screenshot_at,
            quit_at,
            autopilot,
            press: press.clone(),
            pointer: pointer.clone(),
            send: send.clone(),
            mute,
            volume: 60,
            config,
            bindings: Default::default(),
            settings_path: None,
            open_settings,
            open_screen,
            macros: Vec::new(),
            palette: Default::default(),
            ui_scale: 0,
            directories: directories.clone(),
            security: security.unwrap_or_default(),
            security_from_cli: security.is_some(),
            identity: true,
            open_zones,
            pad: Default::default(),
            lvz,
            player_name: None,
            login_file: None,
        }));
    }
    let mode = match (sandbox, server) {
        (true, Some(_)) => return Err("use either --sandbox or --server, not both".into()),
        (true, None) => {
            let ship = ship.unwrap_or(0);
            if ship == oss_session::SPECTATOR {
                return Err("the sandbox needs a ship (1-8)".into());
            }
            app::Mode::Sandbox { map, ship }
        }
        // Built once, for both ways in. The start-up screen fills in who you are
        // and where you are going on top of this, so an option added here reaches
        // the zone whether or not that screen was used -- which is what went wrong
        // when the screen built its own and five options were dropped on the floor.
        (false, server) => {
            let opts = Box::new(ServerOptions {
                server: server.unwrap_or_default(),
                // Filled from the settings file if not given; checked in `apply_config`.
                name: name.unwrap_or_default(),
                password,
                arena,
                ship: ship.unwrap_or(oss_session::SPECTATOR),
                freq,
                follow,
                attach,
                record,
                security: security.unwrap_or_default(),
                known_servers: None,
                identities: None,
                new_user,
            });
            if opts.server.is_empty() {
                app::Mode::Login(opts)
            } else {
                app::Mode::Zone(opts)
            }
        }
    };
    Ok(Some(app::Options {
        mode,
        assets_dir,
        continuum_dir,
        scale,
        vsync,
        fullscreen,
        screenshot,
        screenshot_at,
        quit_at,
        autopilot,
        press,
        pointer,
        send,
        mute,
        volume: 60,
        config,
        bindings: Default::default(),
        settings_path: None,
        open_settings,
        open_screen,
        macros: Vec::new(),
        palette: Default::default(),
        ui_scale: 0,
        directories,
        security: security.unwrap_or_default(),
        security_from_cli: security.is_some(),
        identity: true,
        open_zones,
        pad: Default::default(),
        lvz,
        player_name: None,
        login_file: None,
    }))
}

/// Reads the settings file and fills in what the command line didn't set.
/// The `<seconds>:<key>` both `--press` and `--hold` start with.
fn scripted_key(flag: &str, arg: &str) -> Result<(f32, winit::keyboard::KeyCode), String> {
    let (at, name) = arg
        .split_once(':')
        .ok_or_else(|| format!("{flag} needs <seconds>:<key>, e.g. 3:Enter"))?;
    let at: f32 = at
        .trim()
        .parse()
        .map_err(|_| format!("{flag} needs <seconds>:<key>, e.g. 3:Enter"))?;
    let key = config::key_from_name(name.trim())
        .ok_or_else(|| format!("{flag}: no key called {}", name.trim()))?;
    Ok((at, key))
}

fn apply_config(opts: &mut app::Options) -> Result<(), String> {
    let path = match &opts.config {
        Some(p) => p.clone(),
        None => match config::default_dir() {
            Some(dir) => dir.join(config::FILE_NAME),
            None => {
                log::warn!("no settings folder found; using defaults");
                return finish_options(opts, config::Config::default());
            }
        },
    };
    opts.login_file = path.parent().map(|dir| dir.join(game::login::FILE_NAME));
    opts.settings_path = Some(path.clone());
    let (cfg, warnings) = config::Config::load(&path, opts.config.is_some())?;
    for w in warnings {
        log::warn!("{}: {w}", path.display());
    }
    log::info!("settings: {}", path.display());
    finish_options(opts, cfg)
}

fn finish_options(opts: &mut app::Options, cfg: config::Config) -> Result<(), String> {
    opts.scale = opts.scale.or(cfg.scale);
    opts.vsync |= cfg.vsync;
    opts.fullscreen |= cfg.fullscreen;
    opts.mute |= cfg.mute;
    opts.volume = cfg.volume;
    opts.bindings = cfg.bindings;
    opts.macros = cfg.macros;
    opts.palette = cfg.palette;
    opts.ui_scale = cfg.ui_scale;
    opts.pad = cfg.pad;
    opts.directories.extend(cfg.directories);
    // The command line wins over the settings file.
    if !opts.security_from_cli {
        opts.security = cfg.security;
    }
    opts.identity = cfg.identity;
    opts.player_name = cfg.name.clone();
    if let app::Mode::Zone(z) = &mut opts.mode
        && z.name.is_empty()
    {
        match cfg.name {
            Some(name) => z.name = name,
            // No name anywhere: ask on the start-up screen, carrying everything
            // else the command line asked for rather than starting again from
            // nothing.
            None => opts.mode = app::Mode::Login(z.clone()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Option<app::Options>, String> {
        parse_args(s.split_whitespace().map(String::from))
    }

    /// The start-up screen is a way into a zone like any other, so what the command
    /// line asked for has to reach it. Every one of these was dropped on the floor
    /// by the version that built the screen its own options: a player who typed
    /// `--record` and then used the form got no recording and no complaint.
    #[test]
    fn the_start_up_screen_is_given_what_the_command_line_asked_for() {
        let o = parse("--record visit.ossr --freq 3 --follow Ace --attach Mate --new-user")
            .unwrap()
            .unwrap();
        let app::Mode::Login(z) = &o.mode else {
            panic!("no --server: the start-up screen")
        };
        assert_eq!(
            z.record.as_deref(),
            Some(PathBuf::from("visit.ossr").as_path())
        );
        assert_eq!(z.freq, Some(3));
        assert_eq!(z.follow.as_deref(), Some("Ace"));
        assert_eq!(z.attach.as_deref(), Some("Mate"));
        assert!(z.new_user);

        // And when a zone was named but no name was found anywhere, the screen is
        // asked for the name -- not for everything else over again.
        let mut o = parse("--server x:1 --record visit.ossr --freq 3")
            .unwrap()
            .unwrap();
        finish_options(&mut o, config::Config::default()).unwrap();
        let app::Mode::Login(z) = &o.mode else {
            panic!("no name: ask for one")
        };
        assert_eq!(
            (z.server.as_str(), z.freq, z.record.is_some()),
            ("x:1", Some(3), true)
        );
    }

    /// `--press` exists so the start-up screen can be driven at all: `--server`
    /// goes straight round it and `--send` only types once you are in a game.
    #[test]
    fn scripted_key_presses_parse_and_sort() {
        use app::ScriptedEdge::{Down, Up};
        use winit::keyboard::KeyCode as K;
        let o = parse("--press 4:Enter --press 2:ArrowDown --press 3:Tab")
            .unwrap()
            .unwrap();
        let got: Vec<_> = o.press.iter().map(|k| (k.at, k.key, k.what)).collect();
        assert_eq!(
            got,
            vec![
                (2.0, K::ArrowDown, Down),
                (2.0, K::ArrowDown, Up),
                (3.0, K::Tab, Down),
                (3.0, K::Tab, Up),
                (4.0, K::Enter, Down),
                (4.0, K::Enter, Up),
            ],
            "soonest first, because the app walks the list forwards, and a tap's \
             down stays ahead of its up"
        );
        assert!(parse("--press Enter").is_err(), "no time");
        assert!(parse("--press soon:Enter").is_err(), "time is not a number");
        assert!(parse("--press 3:Nonesuch").is_err(), "no such key");
    }

    /// `--drag` is to `--click` what `--hold` is to `--press`. A slider is
    /// drag-sensed: it reads the pointer where it is while the button is down,
    /// so a press and a release at one instant -- a click -- moves it nowhere.
    /// That is what stopped the pixel scale being set from a scripted run.
    #[test]
    fn a_scripted_drag_presses_moves_and_lets_go() {
        use app::PointerEdge::{Move, Press, Release};
        let o = parse("--click 1:10,20 --drag 2:408,286:460,286")
            .unwrap()
            .unwrap();
        let got: Vec<_> = o.pointer.iter().map(|p| (p.at, p.x, p.y, p.what)).collect();
        assert_eq!(
            got,
            vec![
                // A click is a tap: both edges at once, in the one place.
                (1.0, 10.0, 20.0, Press),
                (1.0, 10.0, 20.0, Release),
                // A drag takes hold where it started and lets go where it ended.
                (2.0, 408.0, 286.0, Press),
                (2.0 + app::DRAG_STEP, 460.0, 286.0, Move),
                (2.0 + 2.0 * app::DRAG_STEP, 460.0, 286.0, Release),
            ],
            "soonest first, and a press stays ahead of the release it belongs to"
        );
        assert!(parse("--drag 2:408,286").is_err(), "only one point");
        assert!(
            parse("--drag 2:408:460,286").is_err(),
            "a point without a y"
        );
        assert!(
            parse("--drag 2:408,286:460,286:9,9").is_err(),
            "three points"
        );
        assert!(
            parse("--drag soon:1,2:3,4").is_err(),
            "time is not a number"
        );
    }

    /// `--hold` is the other half: `--press` taps, so nothing that has to be held
    /// -- thrust, the afterburner, the guns -- could be scripted at all.
    #[test]
    fn a_held_key_goes_down_repeats_and_comes_up() {
        use app::ScriptedEdge::{Down, Repeat, Up};
        use winit::keyboard::KeyCode as K;
        let o = parse("--hold 3:ArrowUp:1").unwrap().unwrap();
        assert_eq!(o.press.first().map(|k| (k.at, k.key, k.what)), {
            Some((3.0, K::ArrowUp, Down))
        });
        assert_eq!(o.press.last().map(|k| (k.at, k.key, k.what)), {
            Some((4.0, K::ArrowUp, Up))
        });
        let repeats = o.press.iter().filter(|k| k.what == Repeat).count();
        // Half a second of delay, then thirty a second for the other half.
        assert_eq!(repeats, 15, "a held key repeats the way a keyboard does");
        assert!(
            o.press.iter().all(|k| (3.0..=4.0).contains(&k.at)),
            "every event falls inside the hold"
        );
        assert!(parse("--hold 3:ArrowUp").is_err(), "no duration");
        assert!(parse("--hold 3:ArrowUp:0").is_err(), "a hold of nothing");
        assert!(parse("--hold 3:Nonesuch:1").is_err(), "no such key");
    }

    #[test]
    fn parses_sandbox_and_server() {
        let o = parse("--sandbox m.lvl --ship 3 --scale 2 --vsync")
            .unwrap()
            .unwrap();
        assert!(matches!(
            &o.mode,
            app::Mode::Sandbox { map: Some(map), ship: 2 } if map == &PathBuf::from("m.lvl")
        ));
        assert_eq!((o.scale, o.vsync), (Some(2), true));
        let o = parse("--sandbox --ship 2").unwrap().unwrap();
        assert!(matches!(&o.mode, app::Mode::Sandbox { map: None, ship: 1 }));
        let o = parse("--server 127.0.0.1:5000 --name Bob --ship 4")
            .unwrap()
            .unwrap();
        assert!(matches!(&o.mode, app::Mode::Zone(z) if z.name == "Bob" && z.ship == 3));
        let mut o = parse("--server x:1").unwrap().unwrap();
        finish_options(&mut o, config::Config::default()).unwrap();
        assert!(
            matches!(&o.mode, app::Mode::Login(z) if z.server == "x:1"),
            "no name: ask for one"
        );
        let cfg = config::Config {
            name: Some("Ace".into()),
            scale: Some(3),
            vsync: true,
            ..Default::default()
        };
        let mut o = parse("--server x:1 --scale 2").unwrap().unwrap();
        finish_options(&mut o, cfg).unwrap();
        assert!(matches!(&o.mode, app::Mode::Zone(z) if z.name == "Ace"));
        assert_eq!((o.scale, o.vsync), (Some(2), true), "flags win");
        assert!(parse("--ship 9 --sandbox m.lvl").is_err());
        assert!(parse("--sandbox m.lvl --ship spec").is_err());
        assert!(matches!(
            parse("").unwrap().unwrap().mode,
            app::Mode::Login(z) if z.server.is_empty()
        ));
        assert!(parse("--help").unwrap().is_none());
    }
}
