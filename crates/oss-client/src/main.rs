//! Open SubSpace game client.

mod app;
mod audio;
mod game;
mod gfx;

use std::path::PathBuf;

use game::online::ServerOptions;

const USAGE: &str = "\
Usage: oss-client --server <host:port> --name <name> [options]
       oss-client --sandbox <map.lvl> [options]

Play online:
  --server <host:port>      Zone to join
  --name <name>             Player name
  --password <pw>           Password (default empty)
  --arena <name>            Arena to enter (default: any public arena)
  --ship <1-8|spec>         Ship to enter in (default spec online, 1 offline)
  --freq <n>                Team to join (in game: type =n)

Play offline:
  --sandbox <map.lvl>       Fly around a map with stock settings

Options:
  --continuum-dir <dir>     Continuum install to read graphics/sounds from
                            (default: OSS_CONTINUUM_DIR, then Steam/standard paths)
  --scale <n>               Integer pixel scale (default: from display DPI)
  --vsync                   Cap frame rate to the display refresh
  --fullscreen              Borderless fullscreen
  --mute                    No sound
  --screenshot <file.png>   Save a frame after 2 seconds, then exit (testing)
  --screenshot-at <secs>    When to take that screenshot
  --autopilot               Fly and shoot automatically (testing)
  -h, --help                Show this help";

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=off,naga=warn"),
    )
    .init();
    match parse_args(std::env::args().skip(1)) {
        Ok(None) => println!("oss-client {}\n\n{USAGE}", env!("CARGO_PKG_VERSION")),
        Ok(Some(opts)) => {
            if let Err(e) = app::run(opts) {
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
    let mut ship: Option<u8> = None;
    let mut freq: Option<i16> = None;
    let mut continuum_dir = None;
    let mut scale = None;
    let mut vsync = false;
    let mut fullscreen = false;
    let mut screenshot = None;
    let mut screenshot_at = 2.0f32;
    let mut autopilot = false;
    let mut mute = false;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--sandbox" => map = Some(PathBuf::from(value("--sandbox")?)),
            "--server" => server = Some(value("--server")?),
            "--name" => name = Some(value("--name")?),
            "--password" => password = value("--password")?,
            "--arena" => arena = Some(value("--arena")?),
            "--freq" => {
                freq = Some(value("--freq")?.parse::<i16>().ok().filter(|f| *f >= 0).ok_or("--freq must be 0 or more")?)
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
            "--mute" => mute = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let mode = match (map, server) {
        (Some(_), Some(_)) => return Err("use either --sandbox or --server, not both".into()),
        (Some(map), None) => {
            let ship = ship.unwrap_or(0);
            if ship == oss_session::SPECTATOR {
                return Err("the sandbox needs a ship (1-8)".into());
            }
            app::Mode::Sandbox { map, ship }
        }
        (None, Some(server)) => app::Mode::Zone(ServerOptions {
            server,
            name: name.ok_or("--name is required online")?,
            password,
            arena,
            ship: ship.unwrap_or(oss_session::SPECTATOR),
            freq,
        }),
        (None, None) => {
            return Err("nothing to do: pass --server <host:port> or --sandbox <map.lvl>".into());
        }
    };
    Ok(Some(app::Options {
        mode,
        continuum_dir,
        scale,
        vsync,
        fullscreen,
        screenshot,
        screenshot_at,
        autopilot,
        mute,
    }))
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
        assert!(
            matches!(&o.mode, app::Mode::Sandbox { map, ship: 2 } if map == &PathBuf::from("m.lvl"))
        );
        assert_eq!((o.scale, o.vsync), (Some(2), true));
        let o = parse("--server 127.0.0.1:5000 --name Bob --ship 4")
            .unwrap()
            .unwrap();
        assert!(matches!(&o.mode, app::Mode::Zone(z) if z.name == "Bob" && z.ship == 3));
        assert!(parse("--server x:1").is_err(), "name required");
        assert!(parse("--ship 9 --sandbox m.lvl").is_err());
        assert!(parse("--sandbox m.lvl --ship spec").is_err());
        assert!(parse("").is_err());
        assert!(parse("--help").unwrap().is_none());
    }
}
