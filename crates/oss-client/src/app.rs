//! Window/event loop glue: owns the window, renderer, input devices and the current
//! screen (connecting to a zone, or in the game).

use std::path::PathBuf;
use std::sync::Arc;

use oss_assets::{AssetDirs, AssetSearch};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::audio::Audio;
use crate::game::art::{self, Art};
use crate::game::input::InputState;
use crate::game::online::{Online, ServerOptions};
use crate::game::sandbox::Sandbox;
use crate::game::view::{Canvas, color};
use crate::gfx::{FrameError, Renderer, TextureId};

/// What to play.
pub enum Mode {
    /// Offline, on a local map (or the built-in one).
    Sandbox { map: Option<PathBuf>, ship: u8 },
    /// Online, in a zone.
    Zone(ServerOptions),
}

pub struct Options {
    pub mode: Mode,
    /// Asset folder overriding the bundled assets (`--assets-dir`).
    pub assets_dir: Option<PathBuf>,
    /// A Continuum install to take assets from (`--continuum-dir`).
    pub continuum_dir: Option<PathBuf>,
    /// Integer pixel scale; `None` picks one from the display's DPI.
    pub scale: Option<u32>,
    pub vsync: bool,
    pub fullscreen: bool,
    /// Save a frame to this file after a short warm-up, then exit.
    pub screenshot: Option<PathBuf>,
    /// Seconds after start to take the screenshot.
    pub screenshot_at: f32,
    /// Drive the ship automatically (for screenshots and smoke tests).
    pub autopilot: bool,
    pub mute: bool,
    /// Master volume 0-100.
    pub volume: u32,
    /// `--config`: the settings file, if not the default one.
    pub config: Option<PathBuf>,
    pub bindings: crate::game::input::Bindings,
}

/// Waiting for the zone: shows progress until the arena is ready.
struct Connecting {
    net: Online,
    art: Art,
    atlas: TextureId,
}

enum Screen {
    Connecting(Box<Connecting>),
    Game(Box<Sandbox>),
    /// Placeholder while switching screens.
    Empty,
}

struct Running {
    window: Arc<Window>,
    renderer: Renderer,
    screen: Screen,
    scale: u32,
    started: std::time::Instant,
    captured: bool,
    /// The window can't be seen (minimised/covered); Windows then stops sending
    /// redraws, so the game is driven from `about_to_wait` without drawing.
    occluded: bool,
}

struct App {
    options: Options,
    dir: AssetDirs,
    level: Vec<u8>,
    running: Option<Running>,
    input: InputState,
    gilrs: Option<gilrs::Gilrs>,
    error: Option<String>,
}

pub fn run(options: Options) -> Result<(), String> {
    let search = AssetSearch {
        assets_dir: options.assets_dir.clone(),
        continuum_dir: options.continuum_dir.clone(),
        // Development builds also find the repository's bundled folder.
        dev_dir: cfg!(debug_assertions)
            .then(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/classic")),
        discover_install: true,
    };
    let dir = AssetDirs::locate(&search).map_err(|e| {
        format!(
            "{e}
Put the game graphics in assets/classic beside the program, or point --assets-dir ({}) or --continuum-dir at a folder that has them.",
            oss_assets::ASSETS_ENV_VAR
        )
    })?;
    let shown: Vec<String> = dir
        .roots()
        .iter()
        .map(|r| r.display().to_string())
        .collect();
    log::info!("assets: {}", shown.join(", "));
    let level = match &options.mode {
        Mode::Sandbox { map: Some(map), .. } => {
            oss_assets::locate::read_file(map).map_err(|e| e.to_string())?
        }
        Mode::Sandbox { map: None, .. } => crate::game::default_map::default_map(),
        Mode::Zone(_) => Vec::new(),
    };

    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let gilrs = match gilrs::Gilrs::new() {
        Ok(g) => Some(g),
        Err(e) => {
            log::warn!("gamepads unavailable: {e}");
            None
        }
    };
    let input = InputState::new(options.bindings.clone());
    let mut app = App {
        options,
        dir,
        level,
        running: None,
        input,
        gilrs,
        error: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, msg: String) {
        log::error!("{msg}");
        self.error = Some(msg);
        event_loop.exit();
    }

    fn audio(&self) -> Option<Audio> {
        if self.options.mute {
            None
        } else {
            Audio::new(&self.dir, self.options.volume)
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        let title = match &self.options.mode {
            Mode::Sandbox { .. } => "Open SubSpace — sandbox".to_string(),
            Mode::Zone(z) => format!("Open SubSpace — {}", z.server),
        };
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        if self.options.fullscreen {
            attrs = attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)));
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => return self.fail(event_loop, format!("window: {e}")),
        };
        let mut renderer = match Renderer::new(window.clone(), self.options.vsync) {
            Ok(r) => r,
            Err(e) => return self.fail(event_loop, format!("graphics: {e}")),
        };
        let scale = self
            .options
            .scale
            .unwrap_or_else(|| window.scale_factor().round().max(1.0) as u32);
        let screen = match &self.options.mode {
            Mode::Sandbox { ship, .. } => {
                match Sandbox::new(
                    &mut renderer,
                    &self.dir,
                    &self.level,
                    *ship,
                    scale,
                    self.audio(),
                ) {
                    Ok(mut s) => {
                        s.aim_bot = self.options.autopilot;
                        Screen::Game(Box::new(s))
                    }
                    Err(e) => return self.fail(event_loop, e),
                }
            }
            Mode::Zone(z) => {
                let size = window.inner_size();
                let res = ((size.width / scale) as u16, (size.height / scale) as u16);
                let net = match Online::connect(z, res) {
                    Ok(n) => n,
                    Err(e) => return self.fail(event_loop, e),
                };
                let (art, atlas_size, pixels) = match art::load(&self.dir, None) {
                    Ok(a) => a,
                    Err(e) => return self.fail(event_loop, e),
                };
                let atlas = renderer.create_texture(atlas_size, atlas_size, &pixels);
                Screen::Connecting(Box::new(Connecting { net, art, atlas }))
            }
        };
        window.request_redraw();
        self.running = Some(Running {
            window,
            renderer,
            screen,
            scale,
            started: std::time::Instant::now(),
            captured: false,
            occluded: false,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.running.is_none() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(run) = self.running.as_mut() {
                    run.renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => self.input.clear(),
            WindowEvent::Occluded(hidden) => {
                if let Some(run) = self.running.as_mut() {
                    run.occluded = hidden;
                    if !hidden {
                        run.window.request_redraw();
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.input.key(code, down);
                }
                if down && let Some(text) = &event.text {
                    self.input.typed.push_str(text);
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop, true),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(run) = self.running.as_ref() else {
            return;
        };
        if run.occluded {
            // Keep the simulation and connection alive, at a modest rate.
            self.frame(event_loop, false);
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                std::time::Instant::now() + std::time::Duration::from_millis(5),
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Poll);
        }
    }
}

impl App {
    /// One frame: input, network, simulation and (when `draw`) rendering.
    fn frame(&mut self, event_loop: &ActiveEventLoop, draw: bool) {
        if let Some(g) = self.gilrs.as_mut() {
            while let Some(ev) = g.next_event() {
                self.input.gamepad(ev.event);
            }
        }
        let audio_wanted = !self.options.mute;
        let Some(run) = self.running.as_mut() else {
            return;
        };
        if self.options.autopilot {
            self.input.autopilot(run.started.elapsed().as_secs_f32());
        }

        // Connecting: pump until the arena is ready, then switch to the game.
        if let Screen::Connecting(conn) = &mut run.screen {
            conn.net.pump();
            if self.input.take_pressed().contains(&KeyCode::Escape) {
                event_loop.exit();
                return;
            }
            self.input.take_edges();
            self.input.typed.clear();
            if let Some(err) = conn.net.error.clone() {
                return self.fail(event_loop, err);
            }
            if conn.net.ready() {
                let Screen::Connecting(conn) = std::mem::replace(&mut run.screen, Screen::Empty)
                else {
                    unreachable!()
                };
                let audio = if audio_wanted {
                    Audio::new(&self.dir, self.options.volume)
                } else {
                    None
                };
                match Sandbox::new_online(&mut run.renderer, &self.dir, conn.net, run.scale, audio)
                {
                    Ok(mut s) => {
                        s.aim_bot = self.options.autopilot;
                        run.screen = Screen::Game(Box::new(s));
                    }
                    Err(e) => return self.fail(event_loop, e),
                }
            }
        }

        let stars = match &mut run.screen {
            Screen::Game(game) => {
                game.update(&mut self.input);
                if game.quit {
                    event_loop.exit();
                    return;
                }
                draw.then(|| game.draw(&mut run.renderer))
            }
            Screen::Connecting(_) if !draw => None,
            Screen::Connecting(conn) => {
                let c = Canvas::new(
                    &conn.art,
                    conn.atlas,
                    run.renderer.size(),
                    run.scale,
                    (0.0, 0.0),
                );
                let lines = [conn.net.status.as_str(), "", "Esc to give up"];
                for (i, line) in lines.iter().enumerate() {
                    let x = ((c.view_w - line.len() as f32 * 8.0) / 2.0).floor();
                    let y = (c.view_h / 2.0 - 20.0 + i as f32 * 14.0).floor();
                    c.text(
                        &mut run.renderer,
                        x,
                        y,
                        if i == 0 { color::GREEN } else { color::WHITE },
                        line,
                    );
                }
                None
            }
            Screen::Empty => None,
        };

        if !draw {
            return;
        }
        let screenshot_due = self.options.screenshot.is_some()
            && run.started.elapsed().as_secs_f32() >= self.options.screenshot_at;
        if screenshot_due && !run.captured && run.renderer.capture.is_none() {
            run.renderer.capture = self.options.screenshot.clone();
            run.captured = true;
        }
        match run.renderer.render(stars, 0.0) {
            Ok(()) | Err(FrameError::Skip) => {}
            Err(FrameError::Fatal(e)) => return self.fail(event_loop, e),
        }
        // A skipped frame (e.g. occluded window) keeps the capture pending.
        if run.captured && run.renderer.capture.is_none() {
            event_loop.exit();
            return;
        }
        run.window.request_redraw();
    }
}
