//! Open SubSpace game client.

mod app;
mod game;
mod gfx;

use std::path::PathBuf;

const USAGE: &str = "\
Usage: oss-client --sandbox <map.lvl> [options]

Options:
  --sandbox <map.lvl>       Fly around a map offline with stock settings
  --continuum-dir <dir>     Continuum install to read graphics/sounds from
                            (default: OSS_CONTINUUM_DIR, then Steam/standard paths)
  --ship <1-8>              Starting ship (default 1, Warbird)
  --scale <n>               Integer pixel scale (default: from display DPI)
  --vsync                   Cap frame rate to the display refresh
  --fullscreen              Borderless fullscreen
  --screenshot <file.png>   Save a frame after 2 seconds, then exit (testing)
  --autopilot               Fly and shoot automatically (testing)
  -h, --help                Show this help";

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=off,naga=warn"),
    )
    .init();
    match parse_args(std::env::args().skip(1)) {
        Ok(Command::Help) => println!("oss-client {}\n\n{USAGE}", env!("CARGO_PKG_VERSION")),
        Ok(Command::Sandbox(opts)) => {
            if let Err(e) = app::run_sandbox(opts) {
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

enum Command {
    Help,
    Sandbox(app::SandboxOptions),
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut map = None;
    let mut continuum_dir = None;
    let mut ship = 1u8;
    let mut scale = None;
    let mut vsync = false;
    let mut fullscreen = false;
    let mut screenshot = None;
    let mut autopilot = false;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--sandbox" => map = Some(PathBuf::from(value("--sandbox")?)),
            "--continuum-dir" => continuum_dir = Some(PathBuf::from(value("--continuum-dir")?)),
            "--ship" => {
                ship = value("--ship")?
                    .parse::<u8>()
                    .ok()
                    .filter(|s| (1..=8).contains(s))
                    .ok_or("--ship must be 1-8")?
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
            "--autopilot" => autopilot = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let map =
        map.ok_or("nothing to do: pass --sandbox <map.lvl> (networked play is not ready yet)")?;
    Ok(Command::Sandbox(app::SandboxOptions {
        map,
        continuum_dir,
        ship: ship - 1,
        scale,
        vsync,
        fullscreen,
        screenshot,
        autopilot,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Command, String> {
        parse_args(s.split_whitespace().map(String::from))
    }

    #[test]
    fn parses_sandbox() {
        let Ok(Command::Sandbox(o)) = parse("--sandbox m.lvl --ship 3 --scale 2 --vsync") else {
            panic!()
        };
        assert_eq!(o.map, PathBuf::from("m.lvl"));
        assert_eq!((o.ship, o.scale, o.vsync), (2, Some(2), true));
        assert!(parse("--ship 9 --sandbox m.lvl").is_err());
        assert!(parse("").is_err());
        assert!(matches!(parse("--help"), Ok(Command::Help)));
    }
}
