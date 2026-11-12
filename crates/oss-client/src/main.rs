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
  --record <file.ossr>      Record the visit to a replay file

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
  --screen <menu|help>      Open the menu or the help pages at start (testing)
  --directory <host:port>   A directory server for the zone browser (F3 on the
                            login screen; repeatable; adds to the settings file's)
  --zones                   Start on the zone browser
  --security <mode>         auto (OpenSec, falling back to VIE), opensec or vie
  --config <file>           Settings file (default: oss-client.ini in the user's
                            settings folder, created on first run)
  --screenshot <file.png>   Save a frame after 2 seconds, then exit (testing)
  --screenshot-at <secs>    When to take that screenshot
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
            "--screen" => {
                let name = value("--screen")?;
                open_screen = Some(match name.as_str() {
                    "menu" => app::StartScreen::Menu,
                    "help" => app::StartScreen::Help,
                    _ => return Err("--screen must be menu or help".into()),
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
            autopilot,
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
        (false, Some(server)) => app::Mode::Zone(Box::new(ServerOptions {
            server,
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
        })),
        // No --server or --sandbox: the login screen.
        (false, None) => app::Mode::Login { server: None },
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
        autopilot,
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
            // No name anywhere: ask on the login screen.
            None => {
                opts.mode = app::Mode::Login {
                    server: Some(z.server.clone()),
                }
            }
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
            matches!(&o.mode, app::Mode::Login { server: Some(s) } if s == "x:1"),
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
            app::Mode::Login { server: None }
        ));
        assert!(parse("--help").unwrap().is_none());
    }
}
