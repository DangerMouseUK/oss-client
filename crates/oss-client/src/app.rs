//! Window/event loop glue: owns the window, renderer, input devices and the current
//! screen (connecting to a zone, or in the game).

use std::path::PathBuf;
use std::sync::Arc;

use oss_assets::{AssetDirs, AssetSearch};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::audio::Audio;
use crate::game::account::{Answer, Prompt};
use crate::game::art::{self, Art};
use crate::game::input::{InputState, Stroke, char_for, shifted_char};
use crate::game::login::{Choice, Login};
use crate::game::online::{Online, ServerOptions, Supplied};
use crate::game::sandbox::Sandbox;
use crate::game::settings_ui::{SettingsUi, UiAction};
use crate::game::start_spots::{Rect, Spot, Spots};
use crate::game::view::{Canvas, color, ui_color};
use crate::gfx::{FrameError, Renderer, TextureId};

/// An in-game screen `--screen` can put up at start, so that a screenshot can
/// catch one without a key being pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartScreen {
    Menu,
    /// A help page, counted from zero.
    Help(usize),
}

/// What to play.
pub enum Mode {
    /// Offline, on a local map (or the built-in one).
    Sandbox { map: Option<PathBuf>, ship: u8 },
    /// Online, in a zone.
    Zone(Box<ServerOptions>),
    /// The start-up screen, carrying everything the command line asked for so that
    /// going in through the form reaches the zone with the same options as going in
    /// straight from the command line.
    Login(Box<ServerOptions>),
    /// Playing a replay file back.
    Replay(PathBuf),
}

/// One scheduled keyboard event in a scripted run.
///
/// `--press` is a tap: the key goes down and comes up in the same frame, which
/// is all an edge-triggered screen looks at. `--hold` keeps it down, and repeats
/// it the way a real keyboard does, because a harness that holds a key without
/// repeating it is not holding a key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScriptedKey {
    /// Seconds after the window opened.
    pub at: f32,
    pub key: winit::keyboard::KeyCode,
    pub what: ScriptedEdge,
}

/// What a [`ScriptedKey`] does to the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptedEdge {
    Down,
    /// The OS repeating a key that is being held.
    Repeat,
    Up,
}

/// A held key does nothing for half a second, then about thirty a second.
///
/// Windows' own defaults, and the rate itself matters to nothing: what reads
/// repeats only asks whether one arrived. Expanded at parse time so the frame
/// loop stays a walk down one sorted list.
pub const REPEAT_DELAY: f32 = 0.5;
/// Repeats a second once they start. See [`REPEAT_DELAY`].
pub const REPEAT_RATE: u32 = 30;

/// One scripted pointer event, in UI pixels.
///
/// `--click` is a press and a release at the same instant, which is all a
/// click-sensed thing looks at. `--drag` puts a move between them and spreads
/// the three over separate frames, so a press in one place leaves the pointer
/// in another. Nothing the client draws is dragged today; the flag stays so
/// that a screen that grows something to drag can be tested without one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScriptedPoint {
    /// Seconds after the window opened.
    pub at: f32,
    pub x: f32,
    pub y: f32,
    pub what: PointerEdge,
}

/// What a [`ScriptedPoint`] does to the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEdge {
    Press,
    /// The pointer moving with the button down, which is what makes a drag one.
    Move,
    Release,
}

/// How far apart a scripted drag's press, move and release are put.
///
/// Anything over one frame will do; this is a handful of them, so a drag still
/// reads as one gesture where a run is being watched. Expanded at parse time so
/// the frame loop stays a walk down one sorted list.
pub const DRAG_STEP: f32 = 0.05;

/// How many lines of the settings window's lists one notch of the wheel moves.
const WHEEL_LINES: i32 = 3;

/// The events `--hold <at>:<key>:<secs>` arrives as: down, then repeats, then up.
///
/// The repeats are counted rather than stepped up to, so how many there are does
/// not turn on where a float lands.
pub fn hold_events(at: f32, key: winit::keyboard::KeyCode, secs: f32) -> Vec<ScriptedKey> {
    let mut v = vec![ScriptedKey {
        at,
        key,
        what: ScriptedEdge::Down,
    }];
    let repeats = ((secs - REPEAT_DELAY).max(0.0) * REPEAT_RATE as f32) as u32;
    for n in 0..repeats {
        v.push(ScriptedKey {
            at: at + REPEAT_DELAY + n as f32 / REPEAT_RATE as f32,
            key,
            what: ScriptedEdge::Repeat,
        });
    }
    v.push(ScriptedKey {
        at: at + secs,
        key,
        what: ScriptedEdge::Up,
    });
    v
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
    /// `--quit-at`: leave after this many seconds, so a timed measurement run
    /// ends by itself and the frame-time summary is printed.
    pub quit_at: Option<f32>,
    /// Drive the ship automatically (for screenshots and smoke tests).
    pub autopilot: bool,
    /// `--press` and `--hold`: scripted keyboard events, soonest first (testing).
    ///
    /// The start-up screen is the one screen no scripted run could reach past,
    /// because `--server` goes straight round it and `--send` only types in game.
    /// This presses its keys, so the path from the form to an arena can be driven
    /// and photographed rather than described.
    ///
    /// The settings window reads the same input, so these drive it too: its
    /// sections, its lines, a rebind and a save can all be pressed.
    pub press: Vec<ScriptedKey>,
    /// `--click` and `--drag`, expanded into pointer edges at the given times
    /// and places, soonest first (testing).
    ///
    /// The start-up screen is mouse-driven, so without this nothing could check
    /// that it is -- the same hole `--press` was, one screen along. Positions
    /// are in UI pixels (game pixels from the top-left), which is what every
    /// position in the drawing code is in, so a click can be aimed at what a
    /// screenshot shows. The settings window is drawn in the same units, so a
    /// click on it is aimed the same way.
    pub pointer: Vec<ScriptedPoint>,
    /// `--send`: lines to type in game at the given times, for scripted smoke tests
    /// (a live `?go`, say). Sorted soonest-last, so the game screen pops them off the
    /// end as its clock passes them.
    pub send: Vec<(f32, String)>,
    pub mute: bool,
    /// Master volume 0-100.
    pub volume: u32,
    /// `--config`: the settings file, if not the default one. It has to exist
    /// already; only the default one is written when it is missing.
    pub config: Option<PathBuf>,
    pub bindings: crate::game::input::Bindings,
    /// Where the settings file lives (for the settings window's Save).
    pub settings_path: Option<PathBuf>,
    /// `--settings`: open the settings window at start.
    pub open_settings: bool,
    /// An in-game screen to open as soon as the game starts (testing).
    pub open_screen: Option<StartScreen>,
    /// Macro keys from the settings file.
    pub macros: Vec<(crate::game::input::Binding, String)>,
    /// Directory servers for the zone browser.
    pub directories: Vec<String>,
    /// Connection encryption: the automatic order, or one mechanism by name.
    pub security: oss_session::security::SecurityMode,
    /// The mechanisms the client can speak, and the client the session presents
    /// itself as. The default is this repository's own: the built-in mechanisms,
    /// and [`oss_session::VieClient`].
    ///
    /// A build outside this repository registers its mechanisms in a
    /// [`Supplied`] and hands it to [`crate::options_with`], so that
    /// `--security` and the settings file know them; see the crate
    /// documentation.
    pub supplied: Supplied,
    /// `--security` was given (it wins over the settings file).
    pub security_from_cli: bool,
    /// Offer zones an OpenSec identity key.
    pub identity: bool,
    /// `--zones`: start on the zone browser.
    pub open_zones: bool,
    /// Accessibility settings.
    pub palette: crate::game::palette::PaletteKind,
    pub ui_scale: u32,
    /// Gamepad buttons per action.
    pub pad: crate::game::input::PadBindings,
    /// `--lvz`: LVZ files to show in the sandbox.
    pub lvz: Vec<PathBuf>,
    /// Player name from the settings file (the login screen's default).
    pub player_name: Option<String>,
    /// Where the login screen remembers the last zone and name.
    pub login_file: Option<PathBuf>,
    /// The settings file as it was read, before anything on the command line
    /// was laid over it.
    ///
    /// The settings screen starts from this, so its Save writes back what the
    /// file said plus what the player changed on the screen -- never what the
    /// command line asked for this once. It started from the merged options
    /// instead, and a test run's `--directory 127.0.0.1:4991` was saved into
    /// the lead's own settings, twice over, leaving the next ordinary start
    /// with a local directory and no zone list.
    pub saved: crate::config::Config,
}

/// Waiting for the zone: shows progress until the arena is ready.
struct Connecting {
    net: Online,
    art: Art,
    atlas: TextureId,
    /// The form to go back to if the connection fails.
    login: Option<Login>,
    /// A question the zone has asked, drawn over the progress lines while it
    /// waits for an answer.
    prompt: Option<Prompt>,
}

struct LoginScreen {
    login: Login,
    art: Art,
    atlas: TextureId,
    /// The zone list, which is part of this screen rather than a screen of its own.
    browser: crate::game::zones::ZoneBrowser,
    /// Where everything was drawn last frame, so the pointer can find it.
    /// Filled by [`draw_start_screen`] as it draws; see
    /// [`crate::game::start_spots`].
    spots: Spots,
    /// Fills in a saved password when the name and zone have one.
    recall: crate::game::remember::Recall,
}

enum Screen {
    Login(Box<LoginScreen>),
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
    /// The login form to come back to when the player leaves the game, as it was
    /// when they went in. `None` when the run never passed through it.
    front_end: Option<Login>,
}

struct App {
    options: Options,
    dir: AssetDirs,
    level: Vec<u8>,
    running: Option<Running>,
    input: InputState,
    gilrs: Option<gilrs::Gilrs>,
    /// The settings window, while it is open over whichever screen opened it.
    settings: Option<SettingsUi>,
    error: Option<String>,
    /// How far through [`Options::press`] the clock has got.
    pressed: usize,
    /// How far through [`Options::pointer`] the clock has got.
    pointed: usize,
    /// Where the pointer is, in UI pixels, while it is over the window.
    cursor: Option<(f32, f32)>,
    /// A left click arrived and the screen has not answered it yet. Held here
    /// rather than acted on in the event, so a click is read against the same
    /// regions, in the same place in the frame, as a key is.
    clicked: bool,
    /// This machine's password store, if it has one, for "Remember password".
    vault: crate::game::remember::Vault,
}

pub fn run(options: Options) -> Result<(), String> {
    let search = AssetSearch {
        assets_dir: options.assets_dir.clone(),
        continuum_dir: options.continuum_dir.clone(),
        // A binary built from the checkout also finds the repository's bundled
        // folder, in every profile. This used to be gated on `debug_assertions`,
        // which made `cargo run --release -p oss-client` -- the command the README
        // gives first -- fail on any machine without a Continuum install, because
        // nothing copies `assets/classic` next to the executable. It is still
        // ordered after the folder beside the executable, so a packaged build that
        // ships its own assets wins.
        //
        // The folder is found at run time, by walking up from the running
        // executable. It used to be `env!("CARGO_MANIFEST_DIR")`, which resolves
        // when the client is compiled and therefore wrote the building machine's
        // absolute path -- its user name included -- into every binary built from
        // it, shipped ones included. An installed copy finds nothing above itself
        // and the search simply moves on, as it did with a baked path that did not
        // exist on the player's machine.
        dev_dir: oss_assets::checkout_assets_dir(),
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
    for line in startup_lines(&options) {
        log::info!("{line}");
    }
    let level = match &options.mode {
        Mode::Sandbox { map: Some(map), .. } => {
            oss_assets::locate::read_file(map).map_err(|e| e.to_string())?
        }
        Mode::Sandbox { map: None, .. } => crate::game::default_map::default_map(),
        Mode::Zone(_) | Mode::Login(_) | Mode::Replay(_) => Vec::new(),
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
    let mut input = InputState::new(options.bindings.clone());
    input.set_pad_bindings(options.pad.clone());
    let mut app = App {
        options,
        dir,
        level,
        running: None,
        input,
        gilrs,
        settings: None,
        error: None,
        pressed: 0,
        pointed: 0,
        clicked: false,
        cursor: None,
        vault: crate::game::remember::Vault::probe(),
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Leaves the zone before the process ends.
///
/// `event_loop.exit()` only stops the loop, and nothing in the transport sends a
/// disconnect on drop, so every quit -- the Esc menu, the window close button, a
/// finished `--screenshot` run -- reached the server as a timeout and left it
/// holding the slot. Called on each exit that might have a live session.
fn leave_zone(screen: &mut Screen) {
    match screen {
        Screen::Game(game) => game.disconnect(),
        Screen::Connecting(conn) => conn.net.disconnect(),
        _ => {}
    }
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, msg: String) {
        log::error!("{msg}");
        self.error = Some(msg);
        event_loop.exit();
    }

    /// Leaves the game for the front end, which is where SubSpace and Continuum
    /// put you when you pick Quit on the menu: the program only ends when you quit
    /// the front end itself.
    ///
    /// The form comes back as it was on the way in. A run that started straight in
    /// a zone (`--server`) or in the sandbox never saw one, so one is made from the
    /// settings and from what that run was playing -- the originals always have a
    /// front end to come back to, and vanishing is the thing being fixed.
    fn back_to_front_end(&mut self) -> Result<(), String> {
        let Some(run) = self.running.as_mut() else {
            return Ok(());
        };
        let mut login = match run.front_end.take() {
            Some(login) => login,
            // This run never passed through the form, so one is made from the
            // settings and from what the run was playing -- which `Login::load`
            // lays over the file for us, the same way it does at start-up.
            None => {
                let base = match &self.options.mode {
                    Mode::Zone(z) | Mode::Login(z) => z.clone(),
                    _ => Box::new(Login::default().into_base()),
                };
                let mut l = Login::load(
                    self.options.login_file.as_deref(),
                    self.options.player_name.as_deref(),
                    base,
                );
                if let Mode::Zone(opts) = &self.options.mode {
                    l.ship = opts.ship;
                }
                l
            }
        };
        login.error = None;
        let (art, atlas) = load_art(&self.dir, &mut run.renderer)?;
        run.screen = Screen::Login(Box::new(LoginScreen {
            login,
            art,
            atlas,
            browser: crate::game::zones::ZoneBrowser::open(&self.options.directories),
            spots: Spots::default(),
            recall: Default::default(),
        }));
        run.window.set_title("Open SubSpace");
        self.settings = None;
        self.input.clear();
        run.window.request_redraw();
        Ok(())
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
            Mode::Login(_) => "Open SubSpace".to_string(),
            Mode::Replay(p) => format!("Open SubSpace — replay {}", p.display()),
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
        let scale = wanted_scale(self.options.scale, window.scale_factor());
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
                        s.scripted = self.options.send.clone();
                        s.open_settings = self.options.open_settings;
                        s.open_screen(self.options.open_screen);
                        for path in &self.options.lvz {
                            match oss_assets::locate::read_file(path) {
                                Ok(data) => s.load_lvz(&path.display().to_string(), &data),
                                Err(e) => log::warn!("{e}"),
                            }
                        }
                        s.macros = self.options.macros.clone();
                        s.palette = self.options.palette;
                        s.ui_scale_extra = self.options.ui_scale;
                        Screen::Game(Box::new(s))
                    }
                    Err(e) => return self.fail(event_loop, e),
                }
            }
            Mode::Zone(z) => {
                let mut z = (**z).clone();
                apply_settings(&self.options, &mut z);
                let z = &z;
                let size = window.inner_size();
                let res = ((size.width / scale) as u16, (size.height / scale) as u16);
                let net = match Online::connect(z, res, &self.options.supplied) {
                    Ok(n) => n,
                    Err(e) => return self.fail(event_loop, e),
                };
                let (art, atlas) = match load_art(&self.dir, &mut renderer) {
                    Ok(a) => a,
                    Err(e) => return self.fail(event_loop, e),
                };
                Screen::Connecting(Box::new(Connecting {
                    net,
                    art,
                    atlas,
                    login: None,
                    prompt: None,
                }))
            }
            Mode::Replay(path) => {
                let net = match Online::open_replay(path) {
                    Ok(n) => n,
                    Err(e) => return self.fail(event_loop, e),
                };
                let (art, atlas) = match load_art(&self.dir, &mut renderer) {
                    Ok(a) => a,
                    Err(e) => return self.fail(event_loop, e),
                };
                Screen::Connecting(Box::new(Connecting {
                    net,
                    art,
                    atlas,
                    login: None,
                    prompt: None,
                }))
            }
            Mode::Login(z) => {
                let (art, atlas) = match load_art(&self.dir, &mut renderer) {
                    Ok(a) => a,
                    Err(e) => return self.fail(event_loop, e),
                };
                let mut login = Login::load(
                    self.options.login_file.as_deref(),
                    self.options.player_name.as_deref(),
                    z.clone(),
                );
                if self.options.open_zones {
                    login.focus_zones();
                }
                Screen::Login(Box::new(LoginScreen {
                    login,
                    art,
                    atlas,
                    browser: crate::game::zones::ZoneBrowser::open(&self.options.directories),
                    spots: Spots::default(),
                    recall: Default::default(),
                }))
            }
        };
        window.request_redraw();
        self.running = Some(Running {
            window,
            renderer,
            screen,
            front_end: None,
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
            WindowEvent::CloseRequested => {
                if let Some(run) = self.running.as_mut() {
                    leave_zone(&mut run.screen);
                }
                event_loop.exit()
            }
            WindowEvent::Resized(size) => {
                log::debug!("resized to {}x{}", size.width, size.height);
                if let Some(run) = self.running.as_mut() {
                    run.renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => self.input.clear(),
            WindowEvent::Focused(true) => {
                if let Some(Running {
                    screen: Screen::Game(game),
                    ..
                }) = self.running.as_mut()
                {
                    game.refocused();
                }
            }
            WindowEvent::Occluded(hidden) => {
                if let Some(run) = self.running.as_mut() {
                    run.occluded = hidden;
                    if !hidden {
                        run.window.request_redraw();
                    }
                }
            }
            // Every key goes into the client's own input, the settings window's
            // included: it reads the keystrokes from there in `frame`, which is
            // what lets a scripted `--press` reach it as it reaches every other
            // screen.
            WindowEvent::KeyboardInput { event, .. } => {
                let code = match event.physical_key {
                    PhysicalKey::Code(code) => Some(code),
                    _ => None,
                };
                if event.state != ElementState::Pressed {
                    if let Some(code) = code {
                        self.input.release(code);
                    }
                    return;
                }
                // The key and the character it typed are one keystroke, with the
                // OS repeat flag on it. `event.repeat` is the flag that used to
                // be dropped on the floor, which is why a held Backspace cleared
                // one character and stopped.
                let mut chars = event.text.iter().flat_map(|t| t.chars());
                let first = chars.next();
                if code.is_some() || first.is_some() {
                    self.input.press(Stroke {
                        key: code,
                        text: first,
                        repeat: event.repeat,
                    });
                }
                // A key event can carry more than one character when a dead key
                // finishes a composition. The key belongs to the first of them.
                for c in chars {
                    self.input.press(Stroke::text(c).repeated(event.repeat));
                }
            }
            // The pointer, which the start-up screen is built around: Continuum's
            // own is a set of named hot spots, so a keyboard-only version of it
            // is a gap rather than a shortfall. Kept to that screen for now --
            // the game underneath has no pointer of its own yet.
            WindowEvent::CursorMoved { position, .. } => {
                let Some(run) = self.running.as_mut() else {
                    return;
                };
                let scale = run.scale.max(1) as f64;
                let at = ((position.x / scale) as f32, (position.y / scale) as f32);
                self.cursor = Some(at);
                if matches!(run.screen, Screen::Login(_)) || self.settings.is_some() {
                    run.window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            // A click is held for `frame`, which hands it to the settings window
            // when that is open and to the screen underneath when it is not.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.clicked = true,
            WindowEvent::MouseWheel { delta, .. } => {
                // `zoneup`/`zonedown` in the skin format are buttons beside the
                // list. The wheel over the list is where a pointer looks for
                // them now.
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => (p.y / 20.0) as f32,
                };
                // Not `signum`, which calls zero positive and would scroll on a
                // wheel that has not turned.
                let rows = if lines > 0.0 {
                    -1
                } else if lines < 0.0 {
                    1
                } else {
                    0
                };
                // The settings window's long lists (a line for every action)
                // scroll under the wheel wherever the pointer is.
                if let Some(ui) = self.settings.as_mut() {
                    ui.scroll(rows * WHEEL_LINES);
                    return;
                }
                let Some(run) = self.running.as_mut() else {
                    return;
                };
                let cursor = self.cursor;
                if let Screen::Login(ls) = &mut run.screen
                    && let Some((mx, my)) = cursor
                    && ls.spots.over_list(mx, my)
                {
                    ls.browser.scroll_by(rows);
                    ls.login.focus_zones();
                    if let Some(z) = ls.browser.zones.get(ls.browser.selected) {
                        ls.login.server.clone_from(&z.address);
                        ls.login.error = None;
                    }
                    run.window.request_redraw();
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
        let frame_at = std::time::Instant::now();
        if let Some(g) = self.gilrs.as_mut() {
            while let Some(ev) = g.next_event() {
                let pressed = self.input.gamepad(ev.event);
                if let (Some(b), Some(ui)) = (pressed, self.settings.as_mut()) {
                    ui.capture_pad(b);
                }
            }
        }
        let audio_wanted = !self.options.mute;
        let Some(since_start) = self.running.as_ref().map(|r| r.started.elapsed()) else {
            return;
        };
        let elapsed = since_start.as_secs_f32();
        // Scripted pointer events: put the pointer there, then press, which is
        // the order a real pointer does it in and the order the hover highlight
        // needs. Every screen, the settings window included, keeps the same
        // pointer in the same units, which has a place and a click and nothing
        // else yet: a release is theirs to ignore, and a drag is a press in one
        // place and a pointer that ends up in another.
        while let Some(p) = self.options.pointer.get(self.pointed).copied() {
            if p.at > elapsed {
                break;
            }
            log::info!("pointer: {:?} at {},{}", p.what, p.x, p.y);
            self.cursor = Some((p.x, p.y));
            self.clicked |= p.what == PointerEdge::Press;
            self.pointed += 1;
        }
        // Taken before the run is borrowed, because the screens underneath want
        // it and it lives on the app rather than on any one of them.
        let cursor = self.cursor;
        // Scripted keys, before anything reads the input. A key carries the
        // character it types, the way a window keyboard event does, or it drives
        // nothing that reads text -- which is the menu, the chat line, every
        // form field and a macro's line in the settings.
        while let Some(k) = self.options.press.get(self.pressed).copied() {
            if k.at > elapsed {
                break;
            }
            match k.what {
                ScriptedEdge::Down | ScriptedEdge::Repeat => {
                    // Shift is read from what is held, so `--hold` on a Shift
                    // key makes the keys under it type their capitals -- which
                    // is the only way a scripted run can type the colon in a
                    // zone address.
                    let typed = if self.input.shift() {
                        shifted_char(k.key)
                    } else {
                        char_for(k.key)
                    };
                    let repeat = k.what == ScriptedEdge::Repeat;
                    if !repeat {
                        log::info!("--press: {:?}", k.key);
                    }
                    let mut s = Stroke::press(k.key).repeated(repeat);
                    if let Some(c) = typed {
                        s = s.typing(c);
                    }
                    self.input.press(s);
                }
                ScriptedEdge::Up => self.input.release(k.key),
            }
            self.pressed += 1;
        }
        // The settings window, while it is open, has the keyboard and the
        // pointer: it takes this frame's keystrokes and click before the screen
        // underneath can, and the key table's actions are dropped so nothing
        // fires in the game when the window closes.
        if let Some(ui) = self.settings.as_mut() {
            let strokes = self.input.take_strokes();
            self.input.take_edges();
            let mut action = ui.input(&strokes, self.input.shift(), self.input.ctrl());
            if std::mem::take(&mut self.clicked)
                && action == UiAction::None
                && let Some((mx, my)) = cursor
            {
                action = ui.click(mx, my);
            }
            match action {
                UiAction::None => {}
                UiAction::Close => self.settings = None,
                UiAction::Save => self.save_settings(),
            }
        }
        let Some(run) = self.running.as_mut() else {
            return;
        };
        if self.options.autopilot {
            self.input.autopilot(run.started.elapsed().as_secs_f32());
        }

        // Login: wait for a choice.
        if let Screen::Login(ls) = &mut run.screen {
            ls.browser.pump();
            // The Remember line appears once the store has said it is there.
            self.vault.poll();
            ls.login.set_vault(self.vault.store().is_some());
            if let Some(store) = self.vault.store() {
                ls.recall
                    .tick(&mut ls.login, &store, std::time::Instant::now());
                if let Some(who) = ls.login.take_forget() {
                    crate::game::remember::forget(&store, who);
                }
            }
            // While the options are up they have the keyboard; the screen underneath
            // would otherwise take every character into whichever field has focus.
            let strokes = if self.settings.is_some() {
                self.input.drop_strokes();
                Vec::new()
            } else {
                self.input.take_strokes()
            };
            self.input.take_edges();
            // The list owns Up/Down and the rest of its keys while it has the focus.
            // Whichever row it settles on fills in the Zone field, so the highlighted
            // row and the address can never disagree; Enter and Esc belong to the
            // screen, and the form below answers them.
            if ls.login.focused() == crate::game::login::Field::Zones {
                let picked = ls.browser.input(&strokes);
                if let Some(crate::game::zones::BrowserChoice::Pick(address)) = picked {
                    ls.login.server = address;
                    ls.login.error = None;
                } else if strokes.iter().any(|s| s.key.is_some())
                    && let Some(z) = ls.browser.zones.get(ls.browser.selected)
                {
                    ls.login.server.clone_from(&z.address);
                    ls.login.error = None;
                }
            }
            // A click, against the regions the last frame drew. The keyboard is
            // untouched by this: clicking a field focuses it exactly as tabbing
            // to it does, so everything that worked before still does.
            let clicked = std::mem::take(&mut self.clicked);
            let mut pointed = None;
            if clicked && self.settings.is_none() {
                pointed = cursor.and_then(|(mx, my)| ls.spots.at(mx, my));
                match pointed {
                    Some(Spot::Field(field)) => {
                        ls.login.focus_on(field);
                        if field == crate::game::login::Field::Remember {
                            ls.login.toggle_remember();
                        }
                    }
                    // Picking a ship takes the focus with it, so that choosing
                    // the one you already have still shows something: the
                    // heading's name turns yellow and its line lights up.
                    // Without that, clicking Spectator while already
                    // spectating set spectator to spectator and the screen
                    // said nothing at all -- which is indistinguishable from a
                    // click that missed.
                    Some(Spot::Ship(kind)) => {
                        ls.login.ship = kind;
                        ls.login.focus_on(crate::game::login::Field::Ship);
                    }
                    Some(Spot::Spectate) => {
                        ls.login.ship = oss_session::SPECTATOR;
                        ls.login.focus_on(crate::game::login::Field::Ship);
                    }
                    Some(Spot::Zone(row)) => {
                        ls.browser.select(row);
                        ls.login.focus_zones();
                        if let Some(z) = ls.browser.zones.get(row) {
                            ls.login.server.clone_from(&z.address);
                            ls.login.error = None;
                        }
                    }
                    Some(Spot::RefreshZones)
                    | Some(Spot::Play)
                    | Some(Spot::Quit)
                    | Some(Spot::Options)
                    | Some(Spot::Sandbox)
                    | None => {}
                }
            }
            let choice = match pointed {
                // The four that are choices rather than changes go through the
                // same door the keys do, so neither can do something the other
                // cannot.
                Some(Spot::Play) => ls.login.play(),
                Some(Spot::Quit) => Some(Choice::Quit),
                Some(Spot::Options) => Some(Choice::Options),
                Some(Spot::Sandbox) => Some(Choice::Sandbox),
                Some(Spot::RefreshZones) => Some(Choice::RefreshZones),
                _ => ls.login.input(&strokes, self.input.shift()),
            };
            match choice {
                Some(Choice::Options) => {
                    self.settings = Some(SettingsUi::new(current_config(&self.options)));
                }
                Some(Choice::RefreshZones) => {
                    // Keeps the zones on screen while the directories are asked
                    // again; the line under the list says a refresh is running.
                    ls.browser.refresh(&self.options.directories);
                    // Whatever went wrong last time is not what is happening now,
                    // and an error on that line hides the refresh behind it.
                    ls.login.error = None;
                }
                None => {}
                Some(Choice::Quit) => {
                    event_loop.exit();
                    return;
                }
                Some(Choice::Sandbox) => {
                    let level = crate::game::default_map::default_map();
                    let audio = audio_wanted
                        .then(|| Audio::new(&self.dir, self.options.volume))
                        .flatten();
                    match Sandbox::new(&mut run.renderer, &self.dir, &level, 0, run.scale, audio) {
                        Ok(mut s) => {
                            s.macros = self.options.macros.clone();
                            s.palette = self.options.palette;
                            s.ui_scale_extra = self.options.ui_scale;
                            run.screen = Screen::Game(Box::new(s));
                        }
                        Err(e) => return self.fail(event_loop, e),
                    }
                }
                Some(Choice::Connect(mut opts)) => {
                    apply_settings(&self.options, &mut opts);
                    if let Some(path) = &self.options.login_file {
                        ls.login.save(path);
                    }
                    let size = run.window.inner_size();
                    let res = (
                        (size.width / run.scale) as u16,
                        (size.height / run.scale) as u16,
                    );
                    match Online::connect(&opts, res, &self.options.supplied) {
                        Ok(net) => {
                            run.window
                                .set_title(&format!("Open SubSpace — {}", opts.server));
                            let Screen::Login(ls) =
                                std::mem::replace(&mut run.screen, Screen::Empty)
                            else {
                                unreachable!()
                            };
                            let LoginScreen {
                                login, art, atlas, ..
                            } = *ls;
                            run.screen = Screen::Connecting(Box::new(Connecting {
                                net,
                                art,
                                atlas,
                                login: Some(login),
                                prompt: None,
                            }));
                        }
                        Err(e) => ls.login.error = Some(e),
                    }
                }
            }
        }

        // Connecting: pump until the arena is ready, then switch to the game.
        if let Screen::Connecting(conn) = &mut run.screen {
            conn.net.pump();
            // The zone offered to create the name: ask the player, over this
            // screen, with the connection held open behind the box.
            if conn.prompt.is_none()
                && let Some((_, said)) = &conn.net.new_account
            {
                conn.prompt = Some(Prompt::new_account(said.as_deref()));
            }
            // The zone let us in and asks for its registration form: the same
            // box, and the arena waits behind it.
            if conn.prompt.is_none() && conn.net.registration {
                conn.prompt = Some(Prompt::registration());
            }
            let strokes = self.input.take_strokes();
            self.input.take_edges();
            let clicked = std::mem::take(&mut self.clicked);
            // Why the player is going back, when they are.
            let mut give_up = None;
            if let Some(prompt) = conn.prompt.as_mut() {
                let mut answer = prompt.input(&strokes, self.input.shift());
                if answer.is_none()
                    && clicked
                    && let Some((mx, my)) = cursor
                {
                    answer = prompt.click(mx, my);
                }
                match answer {
                    Some(Answer::Create) => {
                        conn.prompt = None;
                        conn.net.create_account();
                    }
                    Some(Answer::Cancel) => give_up = Some("No account was created.".to_string()),
                    Some(Answer::Send(form)) => {
                        conn.prompt = None;
                        conn.net.send_registration(&form);
                    }
                    // Continuum's other way out of its form: leave the zone.
                    Some(Answer::Disconnect) => {
                        give_up = Some("Disconnected from the zone.".to_string());
                    }
                    None => {}
                }
            } else if strokes
                .iter()
                .any(|s| s.first_press() == Some(KeyCode::Escape))
            {
                give_up = Some("Cancelled".to_string());
            }
            let failed = conn.net.error.clone();
            if give_up.is_some() || failed.is_some() {
                // Say so to the zone. Dropping the session sends nothing, and a
                // zone holding a login open for an answer would otherwise wait
                // out the timeout for one that is not coming.
                if failed.is_none() {
                    conn.net.disconnect();
                }
                // Back to the form if there was one; otherwise this was the whole run.
                let Screen::Connecting(conn) = std::mem::replace(&mut run.screen, Screen::Empty)
                else {
                    unreachable!()
                };
                let Connecting {
                    art, atlas, login, ..
                } = *conn;
                match (login, failed) {
                    (Some(mut login), err) => {
                        login.error = err.or(give_up);
                        run.screen = Screen::Login(Box::new(LoginScreen {
                            login,
                            art,
                            atlas,
                            browser: crate::game::zones::ZoneBrowser::open(
                                &self.options.directories,
                            ),
                            spots: Spots::default(),
                            recall: Default::default(),
                        }));
                    }
                    (None, Some(err)) => return self.fail(event_loop, err),
                    (None, None) => {
                        if let Some(why) = give_up {
                            log::info!("{why}");
                        }
                        event_loop.exit();
                        return;
                    }
                }
            }
        }
        if let Screen::Connecting(conn) = &mut run.screen
            && conn.net.ready()
        {
            let Screen::Connecting(conn) = std::mem::replace(&mut run.screen, Screen::Empty) else {
                unreachable!()
            };
            // Kept for the way back: leaving the game returns to this form.
            let conn = *conn;
            // The zone has let us in, so the password was right: now, and only
            // now, it is remembered (or a remembered one forgotten).
            if let Some(login) = &conn.login
                && let Some(store) = self.vault.store()
            {
                crate::game::remember::settle(&store, login);
            }
            run.front_end = conn.login;
            let audio = if audio_wanted {
                Audio::new(&self.dir, self.options.volume)
            } else {
                None
            };
            match Sandbox::new_online(&mut run.renderer, &self.dir, conn.net, run.scale, audio) {
                Ok(mut s) => {
                    s.aim_bot = self.options.autopilot;
                    s.scripted = self.options.send.clone();
                    s.open_screen(self.options.open_screen);
                    // `--lvz` applies online too, so a zone's own file can be tried
                    // against a real arena (and so an arena change has something to
                    // tear down on a zone that serves none).
                    for path in &self.options.lvz {
                        match oss_assets::locate::read_file(path) {
                            Ok(data) => s.load_lvz(&path.display().to_string(), &data),
                            Err(e) => log::warn!("{e}"),
                        }
                    }
                    s.macros = self.options.macros.clone();
                    s.palette = self.options.palette;
                    s.ui_scale_extra = self.options.ui_scale;
                    run.screen = Screen::Game(Box::new(s));
                }
                Err(e) => return self.fail(event_loop, e),
            }
        }

        let stars = match &mut run.screen {
            Screen::Game(game) => {
                if self.settings.is_none() {
                    // A click on the Esc menu becomes the keystroke that row
                    // answers to, put on the queue in front of everything the
                    // keyboard sent, so the menu reads it as a key and never
                    // learns a pointer was involved.
                    if std::mem::take(&mut self.clicked)
                        && let Some((mx, my)) = cursor
                        && let Some(stroke) = game.click(mx, my)
                    {
                        self.input.press(stroke);
                        // A click is a press and a release. Without the release
                        // the key stays held as far as the key table knows, and
                        // the real key does nothing until the window loses focus.
                        if let Some(key) = stroke.key {
                            self.input.release(key);
                        }
                    }
                    game.update(&mut self.input);
                } else {
                    // The world keeps running; your ship idles while the screen is open.
                    let mut idle = InputState::new(Default::default());
                    game.update(&mut idle);
                }
                if game.leave {
                    game.disconnect();
                    if let Err(e) = self.back_to_front_end() {
                        return self.fail(event_loop, e);
                    }
                    return;
                }
                if std::mem::take(&mut game.open_settings) {
                    self.settings = Some(SettingsUi::new(current_config(&self.options)));
                }
                if std::mem::take(&mut game.screenshot_request) {
                    match screenshot_path(self.options.settings_path.as_deref()) {
                        Ok(path) => {
                            game.message(format!("Screenshot: {}", path.display()), color::GREEN);
                            run.renderer.capture = Some(path);
                        }
                        Err(e) => game.message(format!("Can't save a screenshot: {e}"), color::RED),
                    }
                }
                draw.then(|| {
                    let at = std::time::Instant::now();
                    let stars = game.draw(&mut run.renderer);
                    let took = at.elapsed();
                    run.renderer.perf().record(crate::gfx::Stage::Draw, took);
                    stars
                })
            }
            Screen::Connecting(_) | Screen::Login(_) if !draw => None,
            Screen::Login(ls) => {
                draw_start_screen(ls, &mut run.renderer, run.scale, cursor);
                None
            }
            Screen::Connecting(conn) => {
                let c = Canvas::new(
                    &conn.art,
                    conn.atlas,
                    run.renderer.size(),
                    run.scale,
                    (0.0, 0.0),
                );
                if let Some(prompt) = conn.prompt.as_mut() {
                    prompt.draw(&c, &mut run.renderer, cursor);
                } else {
                    draw_progress(&c, &mut run.renderer, &conn.net);
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
        if let Some(ui) = self.settings.as_mut() {
            draw_settings(ui, &run.screen, &mut run.renderer, run.scale, cursor);
        }
        match run.renderer.render(stars, 0.0) {
            Ok(()) | Err(FrameError::Skip) => {}
            Err(FrameError::Fatal(e)) => return self.fail(event_loop, e),
        }
        run.renderer.perf().frame_done(frame_at.elapsed());
        if let Some(at) = self.options.quit_at
            && run.started.elapsed().as_secs_f32() >= at
        {
            log::info!("--quit-at {at}: leaving");
            leave_zone(&mut run.screen);
            event_loop.exit();
            return;
        }
        // A skipped frame (e.g. occluded window) keeps the capture pending.
        if run.captured && run.renderer.capture.is_none() {
            leave_zone(&mut run.screen);
            event_loop.exit();
            return;
        }
        run.window.request_redraw();
    }
}

fn load_art(dir: &AssetDirs, renderer: &mut Renderer) -> Result<(Art, TextureId), String> {
    let (art, size, pixels) = art::load(dir, None)?;
    let atlas = renderer.create_texture(size, size, &pixels);
    Ok((art, atlas))
}

/// Draws the settings window over whichever screen opened it, with that
/// screen's artwork, at the pixel scale the pointer is measured in.
fn draw_settings(
    ui: &mut SettingsUi,
    screen: &Screen,
    r: &mut Renderer,
    scale: u32,
    cursor: Option<(f32, f32)>,
) {
    let (art, atlas) = match screen {
        Screen::Login(ls) => (&ls.art, ls.atlas),
        Screen::Connecting(conn) => (&conn.art, conn.atlas),
        Screen::Game(game) => game.artwork(),
        Screen::Empty => return,
    };
    let mut c = Canvas::new(art, atlas, r.size(), scale, (0.0, 0.0));
    // In the palette being chosen, so a player picking one for their eyes sees
    // it before saving it rather than after.
    c.tints = ui.cfg.palette.text_tints();
    ui.draw(&c, r, cursor);
}

/// The lines the client logs at start about its security: the mechanisms it
/// supports, weakest first, and the order `auto` tries them in. Both are read
/// from [`Options::supplied`]'s mechanisms.
pub fn startup_lines(options: &Options) -> [String; 2] {
    oss_session::security::startup_lines(&options.supplied.mechanisms)
}

/// What the settings window opens on: the settings file's own values, never the
/// command line's (see [`Options::saved`]).
fn current_config(o: &Options) -> crate::config::Config {
    o.saved.clone()
}

impl App {
    fn save_settings(&mut self) {
        let Some(ui) = self.settings.as_mut() else {
            return;
        };
        let cfg = ui.cfg.clone();
        self.options.saved = cfg.clone();
        let path = self.options.settings_path.clone();
        ui.status = Some(match &path {
            Some(p) => match std::fs::write(p, cfg.to_text(&self.options.supplied.mechanisms)) {
                Ok(()) => {
                    ui.mark_saved();
                    format!("Saved to {}", p.display())
                }
                Err(e) => format!("Couldn't save: {e}"),
            },
            None => {
                ui.mark_saved();
                "No settings folder; applied for this session only".into()
            }
        });
        // Apply what can change now.
        let o = &mut self.options;
        o.scale = cfg.scale;
        o.vsync = cfg.vsync;
        o.mute = cfg.mute;
        o.volume = cfg.volume;
        o.player_name = cfg.name.clone();
        o.bindings = cfg.bindings.clone();
        o.macros = cfg.macros.clone();
        o.palette = cfg.palette;
        o.ui_scale = cfg.ui_scale;
        o.pad = cfg.pad.clone();
        self.input.set_bindings(cfg.bindings.clone());
        self.input.set_pad_bindings(cfg.pad.clone());
        if let Some(run) = self.running.as_mut() {
            if cfg.fullscreen != o.fullscreen {
                run.window.set_fullscreen(
                    cfg.fullscreen
                        .then_some(winit::window::Fullscreen::Borderless(None)),
                );
            }
            // "From display" means take it from the display, which is a value
            // to work out now, not a setting to skip. Skipping it is what made
            // choosing 0 look as though it did nothing: the old scale stayed
            // until the next start. The same function the first frame used, so
            // the two cannot answer differently.
            let wanted = wanted_scale(cfg.scale, run.window.scale_factor());
            run.scale = wanted;
            if let Screen::Game(game) = &mut run.screen {
                game.apply_settings(
                    Some(wanted),
                    cfg.volume,
                    cfg.macros.clone(),
                    cfg.palette,
                    cfg.ui_scale,
                );
            }
        }
        self.options.fullscreen = cfg.fullscreen;
    }
}

/// The pixel scale to run at: the one that was chosen, or the display's own
/// where nothing was.
///
/// `None` is the settings window's "from display" (0 in the file). It is a
/// value to resolve, not a setting to leave alone -- treating it as the latter
/// is what made choosing 0 appear to do nothing, because the old scale simply
/// stayed.
/// Used both for the first frame and when the settings are saved, so the two
/// cannot disagree.
fn wanted_scale(chosen: Option<u32>, display_factor: f64) -> u32 {
    chosen
        .unwrap_or_else(|| display_factor.round().max(1.0) as u32)
        .clamp(1, 8)
}

/// A new screenshot file: `screenshots/oss-YYYY-MM-DD-HHMMSS.png` beside the
/// settings file (or in the current folder).
fn screenshot_path(settings: Option<&std::path::Path>) -> std::io::Result<PathBuf> {
    let dir = settings
        .and_then(std::path::Path::parent)
        .map_or_else(|| PathBuf::from("screenshots"), |d| d.join("screenshots"));
    std::fs::create_dir_all(&dir)?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stamp = utc_stamp(secs);
    for n in 0..100 {
        let name = if n == 0 {
            format!("oss-{stamp}.png")
        } else {
            format!("oss-{stamp}-{n}.png")
        };
        let path = dir.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(std::io::Error::other("too many screenshots this second"))
}

/// `YYYY-MM-DD-HHMMSS` (UTC) for seconds since 1970.
fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Days to civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}-{:02}{:02}{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// How wide the start-up screen is, in characters of the 8x12 font.
const START_COLS: f32 = 72.0;
/// The profile and ships take the left of it; the zone list has the rest.
const START_LEFT_COLS: f32 = 30.0;

/// Colour for a zone's round trip, banded good / usable / poor, as the originals
/// coloured them. A zone that has not answered is dimmed rather than coloured: no
/// answer is not the same as a bad answer.
///
/// The exact thresholds the original used are not recorded here, so these are picked
/// to be useful against what live zones actually return — the ones measured today sat
/// between 72 and 115 ms. The fidelity point is that the column is coloured at all.
fn ping_color(ping_ms: Option<u32>) -> u32 {
    match ping_ms {
        None => color::DARK_RED,
        Some(ms) if ms <= 100 => color::GREEN,
        Some(ms) if ms <= 250 => color::YELLOW,
        Some(_) => color::RED,
    }
}

/// The start-up screen: profile, ship, zone list and the way through to the
/// options, on one screen.
///
/// Continuum's start-up screen is a single skinned window carrying all of that --
/// its skin format names `profile`, `ship1`..`ship8`, `spectate`, `zones`, `trace`,
/// `play`, `quit`, `macros`, `keydefs` and `options` as hot spots on one surface --
/// where this client had three screens and made the player walk between them. The
/// art of that window is Continuum's own and not ours to copy, so this is its
/// arrangement drawn in the box the game draws for itself.
fn draw_start_screen(
    ls: &mut LoginScreen,
    r: &mut Renderer,
    scale: u32,
    cursor: Option<(f32, f32)>,
) {
    let c = Canvas::new(&ls.art, ls.atlas, r.size(), scale, (0.0, 0.0));
    let (login, b) = (&ls.login, &ls.browser);
    let focus = login.focused();
    // The regions are collected here, as each thing is drawn, so what a pointer
    // can click is the same rectangle a player can see. Last frame's are thrown
    // away first.
    let spots = &mut ls.spots;
    spots.clear();
    let hovered = |rect: Rect| cursor.is_some_and(|(mx, my)| rect.holds(mx, my));

    let w = START_COLS * 8.0;
    let left_w = START_LEFT_COLS * 8.0;
    let list_x_off = left_w + 16.0;
    let list_w = w - list_x_off;

    // Tall enough for whichever column needs more, and for a list worth reading.
    let rows = b.zones.len().clamp(8, 14);
    let list_h = 14.0 + rows as f32 * 12.0;
    // Heading, the profile lines (four, or five with Remember), the gap, the
    // ship heading, and the ship grid: three rows of forty, the third of them
    // spectator's own cell.
    let profile_lines = login
        .fields()
        .iter()
        .filter(|f| profile_label(**f).is_some())
        .count() as f32;
    let left_h = 14.0 + profile_lines * 14.0 + 10.0 + 14.0 + 3.0 * 40.0;
    let body_h = list_h.max(left_h);
    // The foot carries the zone's blurb, two lines for anything that went wrong,
    // and the two key lines.
    let h = 20.0 + body_h + 68.0;
    let x = ((c.view_w - w) / 2.0).floor();
    let top = ((c.view_h - h) / 2.0).floor().max(4.0);

    c.panel(r, x, top, w, h);
    c.text_center(
        r,
        x + w / 2.0,
        top + 1.0,
        color::GREEN,
        "-= Open SubSpace =-",
    );
    c.separator(r, x, top + 13.0, w);

    // --- Profile -------------------------------------------------------------------
    let mut y = top + 20.0;
    c.text(r, x + 4.0, y, color::DARK_RED, "Profile");
    y += 14.0;
    let value_x = x + 4.0 + 10.0 * 8.0;
    for &field in login.fields() {
        let Some(label) = profile_label(field) else {
            continue;
        };
        let row = Rect::new(x + 2.0, y - 1.0, left_w - 4.0, 13.0);
        spots.add(row, Spot::Field(field));
        if focus == field || hovered(row) {
            c.color_rect(r, ui_color::HIGHLIGHT, row.x, row.y, row.w, row.h);
        }
        c.text(r, x + 4.0, y, color::GREEN, label);
        let mut value = login.value(field);
        if focus == field && field != crate::game::login::Field::Remember {
            value.push('_');
        }
        let max = ((left_w - 4.0 - (value_x - x)) / 8.0).max(1.0) as usize;
        let shown: String = if value.chars().count() > max {
            value.chars().skip(value.chars().count() - max).collect()
        } else {
            value
        };
        c.text(r, value_x, y, color::WHITE, &shown);
        y += 14.0;
    }

    // --- Ships, as the originals show them: the ships themselves ------------------
    y += 10.0;
    // The heading names the chosen ship beside it, and that word -- "Spectator",
    // when you are spectating -- is the only place on this screen the choice
    // appears in writing, so it is the first thing a pointer goes for. It was
    // not a region at all: the click landed on nothing, changed nothing and said
    // nothing, which is what "you can't directly click Spectator" was. It takes
    // the focus now, exactly as clicking a line of the profile does.
    let ship_row = Rect::new(x + 2.0, y - 1.0, left_w - 4.0, 13.0);
    spots.add(ship_row, Spot::Field(crate::game::login::Field::Ship));
    if focus == crate::game::login::Field::Ship || hovered(ship_row) {
        c.color_rect(
            r,
            ui_color::HIGHLIGHT,
            ship_row.x,
            ship_row.y,
            ship_row.w,
            ship_row.h,
        );
    }
    c.text(r, x + 4.0, y, color::DARK_RED, "Ship");
    c.text(
        r,
        x + 4.0 + 6.0 * 8.0,
        y,
        if focus == crate::game::login::Field::Ship {
            color::YELLOW
        } else {
            color::WHITE
        },
        ship_title(login.ship),
    );
    y += 14.0;
    let pitch = 56.0;
    let cols = Login::SHIP_COLUMNS;
    // The eight ships and spectator, as one grid of nine cells.
    //
    // Spectator used to be a line of text under the grid reading `S = Spectator`:
    // the same choice the eight beside it offer as a picture you click, offered
    // instead as a keystroke to type. Two kinds of thing for one action, and the
    // one that looks like a label is the one that is not. Worse, the key it named
    // was never implemented here -- nothing on this screen answered to S -- so the
    // one instruction it did give was false.
    for slot in 0..=cols * 2 {
        let (row, col) = ((slot / cols) as f32, (slot % cols) as f32);
        let (sx, sy) = (x + 6.0 + col * pitch, y + row * 40.0);
        // `ship1`..`ship8` and `spectate`: the skin's own hot spots, and the
        // reason this screen was never meant to be driven from the keyboard.
        let spectate = slot == cols * 2;
        let kind = if spectate {
            oss_session::SPECTATOR
        } else {
            slot
        };
        let box_ = Rect::new(sx - 2.0, sy - 2.0, 40.0, 40.0);
        spots.add(
            box_,
            if spectate {
                Spot::Spectate
            } else {
                Spot::Ship(slot)
            },
        );
        if login.ship == kind || hovered(box_) {
            c.color_rect(r, ui_color::HIGHLIGHT, box_.x, box_.y, box_.w, box_.h);
        }
        if spectate {
            // There is no spectator sprite to draw -- the whole point is that
            // there is no ship -- so the cell carries the word instead, in the
            // same box, the same size, with the same highlight.
            c.text_center(r, box_.x + box_.w / 2.0, sy + 12.0, color::WHITE, "SPEC");
        } else {
            c.ui(r, ls.art.ship(slot, 0), sx, sy, None, [1.0; 4]);
        }
    }

    // --- Zones ---------------------------------------------------------------------
    let lx = x + list_x_off;
    let mut ly = top + 20.0;
    // `zones`: the whole list, so the wheel can stand in for `zoneup`/`zonedown`.
    spots.set_list(Rect::new(lx - 2.0, ly - 1.0, list_w + 2.0, body_h));
    let head = Rect::new(lx - 2.0, ly - 1.0, list_w + 2.0, 13.0);
    spots.add(head, Spot::Field(crate::game::login::Field::Zones));
    if focus == crate::game::login::Field::Zones || hovered(head) {
        c.color_rect(r, ui_color::HIGHLIGHT, head.x, head.y, head.w, head.h);
    }
    c.text(r, lx, ly, color::DARK_RED, "Zone");
    c.text_right(r, x + w - 4.0, ly, color::DARK_RED, "Players  Ping");
    ly += 14.0;
    let first = b.selected.saturating_sub(rows - 1);
    let name_chars = ((list_w - 18.0 * 8.0) / 8.0).max(4.0) as usize;
    for (i, z) in b.zones.iter().enumerate().skip(first).take(rows) {
        let row = Rect::new(lx - 2.0, ly - 1.0, list_w + 2.0, 12.0);
        spots.add(row, Spot::Zone(i));
        if i == b.selected || hovered(row) {
            c.color_rect(r, ui_color::HIGHLIGHT, row.x, row.y, row.w, 13.0);
        }
        let name: String = z.name.chars().take(name_chars).collect();
        c.text(r, lx, ly, color::WHITE, &name);
        // Players and ping are drawn separately so the ping can carry its own
        // colour, as SubSpace and Continuum did.
        c.text_right(
            r,
            x + w - 4.0 - 6.0 * 8.0,
            ly,
            color::WHITE,
            &format!("{:>7}", z.players),
        );
        let ping = z.ping_ms.map_or_else(|| "-".to_string(), |p| p.to_string());
        c.text_right(
            r,
            x + w - 4.0,
            ly,
            ping_color(z.ping_ms),
            &format!("{ping:>4}"),
        );
        ly += 12.0;
    }

    // --- The foot: what the selected zone is, then what to press -------------------
    let foot = top + 20.0 + body_h;
    c.separator(r, x, foot, w);
    let width = (w / 8.0 - 1.0) as usize;
    let described: String = b
        .zones
        .get(b.selected)
        .map(|z| z.description.as_str())
        .unwrap_or_default()
        .chars()
        .take(width)
        .collect();
    c.text(r, x + 4.0, foot + 6.0, color::GREEN, &described);

    // The key lines are buttons as well as reminders. Continuum's `play`, `quit`
    // and `options` are visible buttons on its own start-up screen; ours were a
    // line of footer text, which is why the options could be in the client for a
    // week without the person who asked for them finding them.
    let lines: [&[(&str, Option<Spot>)]; 2] = [
        &[
            ("Enter = Play", Some(Spot::Play)),
            ("Tab/arrows = Move", None),
            ("F2 = Options", Some(Spot::Options)),
        ],
        &[
            ("F3 = Zone Refresh", Some(Spot::RefreshZones)),
            ("F5 = Sandbox", Some(Spot::Sandbox)),
            ("Esc = Quit", Some(Spot::Quit)),
        ],
    ];
    for (n, items) in lines.iter().enumerate() {
        let ky = foot + 44.0 + n as f32 * 12.0;
        // Four spaces between items, which is the spacing these two lines were
        // written with when they were one string each.
        let total: f32 = items
            .iter()
            .map(|(t, _)| t.chars().count() as f32 * 8.0)
            .sum::<f32>()
            + 32.0 * (items.len() - 1) as f32;
        let mut kx = (x + w / 2.0 - total / 2.0).floor();
        for (text, spot) in items.iter() {
            let tw = text.chars().count() as f32 * 8.0;
            if let Some(spot) = *spot {
                let rect = Rect::new(kx - 2.0, ky - 1.0, tw + 4.0, 13.0);
                spots.add(rect, spot);
                if hovered(rect) {
                    c.color_rect(r, ui_color::HIGHLIGHT, rect.x, rect.y, rect.w, rect.h);
                }
            }
            c.text(r, kx, ky, color::YELLOW, text);
            kx += tw + 32.0;
        }
    }

    // The second line is whichever matters most: what went wrong, what the
    // pointer is over, or how the zone list is getting on. The middle one is the
    // job Continuum's `infobox` does, in the place it does it. Drawn after the
    // key lines so that their regions are in before this asks what is under the
    // pointer.
    let hint = cursor.and_then(|(mx, my)| spots.at(mx, my)).map(Spot::hint);
    let (second, colour) = match (&login.error, hint) {
        (Some(err), _) => (err.as_str(), color::RED),
        // While a directory is still being waited on, how the list is getting on
        // beats what the pointer is over. Clicking "F3 = Zone Refresh" leaves the
        // pointer on the button, so the button's own description would otherwise
        // sit there for the whole fetch and the refresh would have no answer at
        // all -- which is the half of this the wording never fixed.
        (None, Some(hint)) if !b.busy() => (hint, color::YELLOW),
        (None, _) => (b.status.as_str(), color::WHITE),
    };
    // Wrapped, not cut. A refused connection explains itself in a sentence and a
    // half -- "the zone didn't prove the key on record; remove its line from
    // known_servers.txt" -- and truncating that at the panel edge throws away the
    // half that says what to do about it.
    for (i, line) in crate::game::chat::wrap(second, width, true)
        .iter()
        .take(2)
        .enumerate()
    {
        c.text(r, x + 4.0, foot + 18.0 + i as f32 * 12.0, colour, line);
    }
}

/// The connecting screen while nothing is being asked: where the connection has
/// got to, how to give up, and a bar for the arena's downloads.
fn draw_progress(c: &Canvas, r: &mut Renderer, net: &Online) {
    let lines = [net.status.as_str(), "", "Esc to give up"];
    for (i, line) in lines.iter().enumerate() {
        let x = ((c.view_w - line.len() as f32 * 8.0) / 2.0).floor();
        let y = (c.view_h / 2.0 - 20.0 + i as f32 * 14.0).floor();
        c.text(
            r,
            x,
            y,
            if i == 0 { color::GREEN } else { color::WHITE },
            line,
        );
    }
    // The bar, in the gap under the status line, filled by how much of the
    // arena's content has arrived (counted in bytes).
    if let Some(done) = net.progress {
        let w = (c.view_w * 0.5).clamp(80.0, 320.0).floor();
        let x = ((c.view_w - w) / 2.0).floor();
        let y = (c.view_h / 2.0 - 20.0 + 14.0 + 3.0).floor();
        c.rect(r, x, y, w, 6.0, [0.15, 0.15, 0.2, 1.0]);
        c.rect(
            r,
            x,
            y,
            (w * done.clamp(0.0, 1.0)).floor(),
            6.0,
            [0.25, 0.8, 0.35, 1.0],
        );
    }
}

/// The label for a field of the profile block, or `None` for the fields that are
/// drawn as something other than a line of text.
fn profile_label(field: crate::game::login::Field) -> Option<&'static str> {
    use crate::game::login::Field;
    match field {
        Field::Name | Field::Password | Field::Remember | Field::Arena | Field::Server => {
            Some(field.label())
        }
        Field::Ship | Field::Zones => None,
    }
}

/// The selected ship's name, for the line under the ships.
fn ship_title(ship: u8) -> &'static str {
    const NAMES: [&str; 8] = [
        "Warbird",
        "Javelin",
        "Spider",
        "Leviathan",
        "Terrier",
        "Weasel",
        "Lancaster",
        "Shark",
    ];
    NAMES.get(ship as usize).copied().unwrap_or("Spectator")
}

/// The parts of a connection that come from the settings file rather than from the
/// command line or the form. Both ways into a zone go through here, so neither can
/// be given a different set by accident.
fn apply_settings(o: &Options, opts: &mut ServerOptions) {
    opts.security = o.security.clone();
    opts.known_servers = known_servers_path(o);
    opts.identities = identities_path(o);
}

/// `identities.txt` beside the settings file for your OpenSec identity keys, unless
/// identities are off.
fn identities_path(o: &Options) -> Option<PathBuf> {
    let dir = o.settings_path.as_deref()?.parent()?;
    o.identity.then(|| dir.join("identities.txt"))
}

/// `known_servers.txt` beside the settings file, for OpenSec keys.
fn known_servers_path(o: &Options) -> Option<PathBuf> {
    o.settings_path
        .as_deref()
        .and_then(std::path::Path::parent)
        .map(|d| d.join("known_servers.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test run's `--directory 127.0.0.1:4991` reached the lead's own
    /// settings file, twice, and the next ordinary start showed a local
    /// directory and no zones. The command line is for this run; the settings
    /// screen, and so its Save, starts from what the file said.
    ///
    /// This fails on the version before the fix, which built the screen's
    /// starting values from the merged options.
    #[test]
    fn the_command_line_is_never_saved_into_the_settings_file() {
        let dir = std::env::temp_dir().join(format!("oss-client-b108-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp folder");
        let file = dir.join("settings.ini");
        std::fs::write(
            &file,
            "[directory]
servers = dir.example:4990
[video]
scale = 3
",
        )
        .expect("written");
        let args = [
            "--config",
            file.to_str().expect("a path"),
            "--directory",
            "127.0.0.1:4991",
            "--scale",
            "1",
            "--mute",
        ];
        let o = crate::cli::options(args.iter().map(|s| s.to_string()))
            .expect("parsed")
            .expect("not --help");
        std::fs::remove_dir_all(&dir).ok();

        // This run does what the command line asked.
        assert!(o.directories.iter().any(|d| d == "127.0.0.1:4991"));
        assert_eq!(o.scale, Some(1));
        assert!(o.mute);
        // What a Save would write is the file's own, untouched.
        let saved = current_config(&o).to_text(&o.supplied.mechanisms);
        assert!(!saved.contains("127.0.0.1"), "{saved}");
        assert!(saved.contains("servers = dir.example:4990"), "{saved}");
        assert!(saved.contains("scale = 3"), "{saved}");
        assert!(saved.contains("mute = false"), "{saved}");
    }

    /// A settings file naming a mechanism this build does not have keeps the
    /// word across a Save. It used to be read against a fixed list, dropped as
    /// unknown, and written back as `auto`, so saving any setting -- the
    /// volume, say -- quietly undid the player's choice. A build that does have
    /// the mechanism reads the same file and uses it.
    #[test]
    fn a_mechanism_named_in_the_settings_file_survives_a_save() {
        use oss_session::security::{Era, SecurityMode};

        let dir = std::env::temp_dir().join(format!("oss-client-keepword-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp folder");
        let file = dir.join("settings.ini");
        std::fs::write(
            &file,
            "[network]\nsecurity = dummy@example.org\n[audio]\nvolume = 40\n",
        )
        .expect("written");
        let args = || ["--config", file.to_str().expect("a path")].map(String::from);

        // A build without it: auto for this run, and the word kept.
        let o = crate::cli::options(args().into_iter())
            .expect("parsed")
            .expect("not --help");
        assert_eq!(o.security, SecurityMode::Auto);
        let mut cfg = current_config(&o);
        cfg.volume = 70;
        let saved = cfg.to_text(&o.supplied.mechanisms);
        assert!(saved.contains("security = dummy@example.org"), "{saved}");
        assert!(saved.contains("volume = 70"), "{saved}");
        std::fs::write(&file, &saved).expect("saved");

        // A build that registers it, reading the saved file: it is used.
        let mut supplied = Supplied::default();
        supplied
            .mechanisms
            .register(oss_session::test_zone::dummy_mechanism(
                "dummy@example.org",
                "Dummy",
                Era::Modern,
                2000,
            ))
            .unwrap();
        let o = crate::cli::options_with(args().into_iter(), supplied)
            .expect("parsed")
            .expect("not --help");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(o.security.name(), "dummy@example.org");
        let saved = current_config(&o).to_text(&o.supplied.mechanisms);
        assert!(saved.contains("security = dummy@example.org"), "{saved}");
        assert!(
            saved.contains("; Or name one to use it alone: none, vie, dummy@example.org, opensec."),
            "{saved}"
        );
    }

    /// The client's start-up lines, exactly, for this repository's own build,
    /// and for one that registers a mechanism of middle strength.
    #[test]
    fn the_start_up_lines_are_exact() {
        let o = crate::cli::options(["--sandbox"].map(String::from).into_iter())
            .expect("parsed")
            .expect("not --help");
        assert_eq!(
            startup_lines(&o),
            [
                "Mechanisms supported: None, Subspace, OpenSecurity",
                "Automatic order: OpenSecurity, then Subspace",
            ]
        );

        let mut supplied = Supplied::default();
        supplied
            .mechanisms
            .register(oss_session::test_zone::dummy_mechanism(
                "dummy@example.org",
                "Dummy",
                oss_session::security::Era::Modern,
                2000,
            ))
            .unwrap();
        let o = crate::cli::options_with(["--sandbox"].map(String::from).into_iter(), supplied)
            .expect("parsed")
            .expect("not --help");
        assert_eq!(
            startup_lines(&o),
            [
                "Mechanisms supported: None, Subspace, Dummy, OpenSecurity",
                "Automatic order: OpenSecurity, then Dummy, then Subspace",
            ]
        );
    }

    /// "The scale number doesn't seem to stick or even take effect" -- the
    /// second half of that was choosing 0, which resolved to nothing at all.
    #[test]
    fn zero_means_the_display_rather_than_no_change() {
        // "From display" reaches here as `None`, and must come back as a real
        // scale. Returning the old one is what made the setting look dead.
        assert_eq!(wanted_scale(None, 1.0), 1, "an ordinary display");
        assert_eq!(wanted_scale(None, 2.0), 2, "a high-DPI one");
        assert_eq!(wanted_scale(None, 1.5), 2, "rounded, not truncated");
        assert_eq!(wanted_scale(None, 0.0), 1, "never zero, whatever is asked");
        // A chosen scale is kept, and kept inside what the renderer accepts.
        assert_eq!(wanted_scale(Some(3), 1.0), 3);
        assert_eq!(wanted_scale(Some(99), 1.0), 8);
        assert_eq!(wanted_scale(Some(0), 2.0), 1, "0 as a value, not as None");
    }

    #[test]
    fn utc_stamps() {
        assert_eq!(utc_stamp(0), "1970-01-01-000000");
        assert_eq!(utc_stamp(951_782_400), "2000-02-29-000000");
        assert_eq!(utc_stamp(1_798_765_432), "2027-01-01-010352");
    }

    #[test]
    fn ping_is_banded_by_quality() {
        use crate::game::view::color;
        // Unanswered is dimmed, not red: it is not a bad ping, it is no ping.
        assert_eq!(super::ping_color(None), color::DARK_RED);
        assert_eq!(super::ping_color(Some(0)), color::GREEN);
        // The zones measured against live servers sit around here.
        assert_eq!(super::ping_color(Some(72)), color::GREEN);
        assert_eq!(super::ping_color(Some(115)), color::YELLOW);
        // Band edges land on the better side.
        assert_eq!(super::ping_color(Some(100)), color::GREEN);
        assert_eq!(super::ping_color(Some(101)), color::YELLOW);
        assert_eq!(super::ping_color(Some(250)), color::YELLOW);
        assert_eq!(super::ping_color(Some(251)), color::RED);
    }
}
