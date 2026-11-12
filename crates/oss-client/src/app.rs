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
use crate::game::login::{Choice, Login};
use crate::game::online::{Online, ServerOptions};
use crate::game::sandbox::Sandbox;
use crate::game::settings_ui::{SettingsUi, UiAction};
use crate::game::view::{Canvas, color, ui_color};
use crate::gfx::{FrameError, Renderer, TextureId, UiFrame};

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
    /// `--press`: keys to press at the given times, soonest first (testing).
    ///
    /// The start-up screen is the one screen no scripted run could reach past,
    /// because `--server` goes straight round it and `--send` only types in game.
    /// This presses its keys, so the path from the form to an arena can be driven
    /// and photographed rather than described.
    pub press: Vec<(f32, winit::keyboard::KeyCode)>,
    /// `--send`: lines to type in game at the given times, for scripted smoke tests
    /// (a live `?go`, say). Sorted soonest-last, so the game screen pops them off the
    /// end as its clock passes them.
    pub send: Vec<(f32, String)>,
    pub mute: bool,
    /// Master volume 0-100.
    pub volume: u32,
    /// `--config`: the settings file, if not the default one.
    pub config: Option<PathBuf>,
    pub bindings: crate::game::input::Bindings,
    /// Where the settings file lives (for the settings screen's Save).
    pub settings_path: Option<PathBuf>,
    /// `--settings`: open the settings screen at start.
    pub open_settings: bool,
    /// An in-game screen to open as soon as the game starts (testing).
    pub open_screen: Option<StartScreen>,
    /// Macro keys from the settings file.
    pub macros: Vec<(crate::game::input::Binding, String)>,
    /// Directory servers for the zone browser.
    pub directories: Vec<String>,
    /// Connection encryption.
    pub security: oss_session::security::SecurityMode,
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
}

/// Waiting for the zone: shows progress until the arena is ready.
struct Connecting {
    net: Online,
    art: Art,
    atlas: TextureId,
    /// The form to go back to if the connection fails.
    login: Option<Login>,
}

struct LoginScreen {
    login: Login,
    art: Art,
    atlas: TextureId,
    /// The zone list, which is part of this screen rather than a screen of its own.
    browser: crate::game::zones::ZoneBrowser,
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
    /// egui (the settings screen).
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    settings: Option<SettingsUi>,
    error: Option<String>,
    /// How far through [`Options::press`] the clock has got.
    pressed: usize,
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
        // ordered after that folder, so a packaged build that ships its own assets
        // wins; in one that does not, the baked path simply does not exist and the
        // search moves on.
        dev_dir: Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/classic")),
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
        egui_ctx: egui::Context::default(),
        egui_state: None,
        settings: None,
        error: None,
        pressed: 0,
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
                let net = match Online::connect(z, res) {
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
            WindowEvent::KeyboardInput { event, .. } if self.settings.is_some() => {
                // The settings screen has the keyboard (and may be capturing a key).
                let down = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(code) = event.physical_key {
                    if down
                        && let Some(ui) = self.settings.as_mut()
                        && ui.capturing()
                    {
                        ui.capture(code, self.input.shift(), self.input.ctrl());
                    } else if down && code == KeyCode::Escape {
                        self.settings = None;
                    }
                    // Track modifiers for captures.
                    if matches!(
                        code,
                        KeyCode::ShiftLeft
                            | KeyCode::ShiftRight
                            | KeyCode::ControlLeft
                            | KeyCode::ControlRight
                    ) {
                        self.input.key(code, down);
                    }
                }
                self.egui_event(&WindowEvent::KeyboardInput {
                    device_id: winit::event::DeviceId::dummy(),
                    event,
                    is_synthetic: false,
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                let code = match event.physical_key {
                    PhysicalKey::Code(code) => Some(code),
                    _ => None,
                };
                if let Some(code) = code {
                    self.input.key(code, down);
                }
                if down && let Some(text) = &event.text {
                    self.input.typed.extend(text.chars().map(|c| (code, c)));
                }
            }
            WindowEvent::RedrawRequested => self.frame(event_loop, true),
            other => {
                if self.settings.is_some() {
                    self.egui_event(&other);
                }
            }
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
                let pressed = self.input.gamepad(ev.event);
                if let (Some(b), Some(ui)) = (pressed, self.settings.as_mut()) {
                    ui.capture_pad(b);
                }
            }
        }
        let audio_wanted = !self.options.mute;
        let Some(run) = self.running.as_mut() else {
            return;
        };
        // Scripted keys, before anything reads the input: pressed and released in
        // one frame, which is all an edge-triggered screen looks at.
        let elapsed = run.started.elapsed().as_secs_f32();
        while let Some((at, key)) = self.options.press.get(self.pressed).copied() {
            if at > elapsed {
                break;
            }
            log::info!("--press: {key:?}");
            self.input.key(key, true);
            self.input.key(key, false);
            self.pressed += 1;
        }
        if self.options.autopilot {
            self.input.autopilot(run.started.elapsed().as_secs_f32());
        }

        // Login: wait for a choice.
        if let Screen::Login(ls) = &mut run.screen {
            ls.browser.pump();
            // While the options are up they have the keyboard; the screen underneath
            // would otherwise take every character into whichever field has focus.
            let pressed = if self.settings.is_some() {
                self.input.take_pressed();
                self.input.take_edges();
                self.input.typed.clear();
                Vec::new()
            } else {
                self.input.take_pressed()
            };
            let typed: String = std::mem::take(&mut self.input.typed)
                .into_iter()
                .map(|(_, c)| c)
                .collect();
            self.input.take_edges();
            // The list owns Up/Down and the rest of its keys while it has the focus.
            // Whichever row it settles on fills in the Zone field, so the highlighted
            // row and the address can never disagree; Enter and Esc belong to the
            // screen, and the form below answers them.
            if ls.login.focused() == crate::game::login::Field::Zones {
                let picked = ls.browser.input(&pressed);
                if let Some(crate::game::zones::BrowserChoice::Pick(address)) = picked {
                    ls.login.server = address;
                    ls.login.error = None;
                } else if !pressed.is_empty()
                    && let Some(z) = ls.browser.zones.get(ls.browser.selected)
                {
                    ls.login.server.clone_from(&z.address);
                    ls.login.error = None;
                }
            }
            let choice = ls.login.input(&pressed, &typed, self.input.shift());
            match choice {
                Some(Choice::Options) => {
                    self.settings = Some(SettingsUi::new(current_config(&self.options)));
                }
                Some(Choice::RefreshZones) => {
                    ls.browser = crate::game::zones::ZoneBrowser::open(&self.options.directories);
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
                    match Online::connect(&opts, res) {
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
            let give_up = self.input.take_pressed().contains(&KeyCode::Escape);
            self.input.take_edges();
            self.input.typed.clear();
            let failed = conn.net.error.clone();
            if give_up || failed.is_some() {
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
                        login.error = Some(err.unwrap_or_else(|| "Cancelled".into()));
                        run.screen = Screen::Login(Box::new(LoginScreen {
                            login,
                            art,
                            atlas,
                            browser: crate::game::zones::ZoneBrowser::open(
                                &self.options.directories,
                            ),
                        }));
                    }
                    (None, Some(err)) => return self.fail(event_loop, err),
                    (None, None) => {
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
                draw.then(|| game.draw(&mut run.renderer))
            }
            Screen::Connecting(_) | Screen::Login(_) if !draw => None,
            Screen::Login(ls) => {
                draw_start_screen(ls, &mut run.renderer, run.scale);
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
        let ui_frame = self.settings_frame();
        let Some(run) = self.running.as_mut() else {
            return;
        };
        match run.renderer.render(stars, 0.0, ui_frame) {
            Ok(()) | Err(FrameError::Skip) => {}
            Err(FrameError::Fatal(e)) => return self.fail(event_loop, e),
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

impl App {
    fn egui_event(&mut self, event: &WindowEvent) {
        if let (Some(state), Some(run)) = (self.egui_state.as_mut(), self.running.as_ref()) {
            let _ = state.on_window_event(&run.window, event);
        }
    }
}

/// The settings as they stand now, for the settings screen.
fn current_config(o: &Options) -> crate::config::Config {
    {
        crate::config::Config {
            scale: o.scale,
            vsync: o.vsync,
            fullscreen: o.fullscreen,
            mute: o.mute,
            volume: o.volume,
            name: o.player_name.clone(),
            bindings: o.bindings.clone(),
            macros: o.macros.clone(),
            palette: o.palette,
            ui_scale: o.ui_scale,
            pad: o.pad.clone(),
            directories: o.directories.clone(),
            security: o.security,
            identity: o.identity,
        }
    }
}

impl App {
    /// Runs the settings screen for this frame, applies a save, and returns what to
    /// draw.
    fn settings_frame(&mut self) -> Option<UiFrame> {
        let run = self.running.as_ref()?;
        let ui = self.settings.as_mut()?;
        let state = self.egui_state.get_or_insert_with(|| {
            egui_winit::State::new(
                self.egui_ctx.clone(),
                egui::ViewportId::ROOT,
                &run.window,
                Some(run.window.scale_factor() as f32),
                None,
                Some(run.renderer.max_texture_side()),
            )
        });
        let raw = state.take_egui_input(&run.window);
        let mut action = UiAction::None;
        let out = self.egui_ctx.run_ui(raw, |ui_ctx| action = ui.show(ui_ctx));
        state.handle_platform_output(&run.window, out.platform_output);
        let pixels_per_point = out.pixels_per_point;
        let primitives = self.egui_ctx.tessellate(out.shapes, pixels_per_point);
        let frame = UiFrame {
            primitives,
            textures: out.textures_delta,
            pixels_per_point,
        };
        match action {
            UiAction::None => {}
            UiAction::Close => self.settings = None,
            UiAction::Save => self.save_settings(),
        }
        Some(frame)
    }

    fn save_settings(&mut self) {
        let Some(ui) = self.settings.as_mut() else {
            return;
        };
        let cfg = ui.cfg.clone();
        let path = self.options.settings_path.clone();
        ui.status = Some(match &path {
            Some(p) => match std::fs::write(p, cfg.to_text()) {
                Ok(()) => format!("Saved to {}", p.display()),
                Err(e) => format!("Couldn't save: {e}"),
            },
            None => "No settings folder; applied for this session only".into(),
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
            if let Some(s) = cfg.scale {
                run.scale = s;
            }
            if let Screen::Game(game) = &mut run.screen {
                game.apply_settings(
                    cfg.scale,
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
fn draw_start_screen(ls: &LoginScreen, r: &mut Renderer, scale: u32) {
    let c = Canvas::new(&ls.art, ls.atlas, r.size(), scale, (0.0, 0.0));
    let (login, b) = (&ls.login, &ls.browser);
    let focus = login.focused();

    let w = START_COLS * 8.0;
    let left_w = START_LEFT_COLS * 8.0;
    let list_x_off = left_w + 16.0;
    let list_w = w - list_x_off;

    // Tall enough for whichever column needs more, and for a list worth reading.
    let rows = b.zones.len().clamp(8, 14);
    let list_h = 14.0 + rows as f32 * 12.0;
    let left_h = 14.0 + 4.0 * 14.0 + 10.0 + 14.0 + 2.0 * 40.0 + 14.0;
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
    for &field in Login::fields() {
        let Some(label) = profile_label(field) else {
            continue;
        };
        if focus == field {
            c.color_rect(r, ui_color::HIGHLIGHT, x + 2.0, y - 1.0, left_w - 4.0, 13.0);
        }
        c.text(r, x + 4.0, y, color::GREEN, label);
        let mut value = login.value(field);
        if focus == field {
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
    c.text(r, x + 4.0, y, color::DARK_RED, "Ship");
    // The chosen ship is named beside the heading, so the row below is only the
    // one thing it offers.
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
    for kind in 0u8..8 {
        let (row, col) = ((kind / 4) as f32, (kind % 4) as f32);
        let (sx, sy) = (x + 6.0 + col * pitch, y + row * 40.0);
        if login.ship == kind {
            c.color_rect(r, ui_color::HIGHLIGHT, sx - 2.0, sy - 2.0, 40.0, 40.0);
        }
        c.ui(r, ls.art.ship(kind, 0), sx, sy, None, [1.0; 4]);
    }
    y += 2.0 * 40.0;
    let spectating = login.ship == oss_session::SPECTATOR;
    if spectating {
        c.color_rect(r, ui_color::HIGHLIGHT, x + 2.0, y - 1.0, left_w - 4.0, 13.0);
    }
    c.text(r, x + 6.0, y, color::WHITE, "S = Spectator");

    // --- Zones ---------------------------------------------------------------------
    let lx = x + list_x_off;
    let mut ly = top + 20.0;
    if focus == crate::game::login::Field::Zones {
        c.color_rect(
            r,
            ui_color::HIGHLIGHT,
            lx - 2.0,
            ly - 1.0,
            list_w + 2.0,
            13.0,
        );
    }
    c.text(r, lx, ly, color::DARK_RED, "Zone");
    c.text_right(r, x + w - 4.0, ly, color::DARK_RED, "Players  Ping");
    ly += 14.0;
    let first = b.selected.saturating_sub(rows - 1);
    let name_chars = ((list_w - 18.0 * 8.0) / 8.0).max(4.0) as usize;
    for (i, z) in b.zones.iter().enumerate().skip(first).take(rows) {
        if i == b.selected {
            c.color_rect(
                r,
                ui_color::HIGHLIGHT,
                lx - 2.0,
                ly - 1.0,
                list_w + 2.0,
                13.0,
            );
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
    // The second line is whichever matters more: what went wrong, or how the zone
    // list is getting on.
    let (second, colour) = match &login.error {
        Some(err) => (err.as_str(), color::RED),
        None => (b.status.as_str(), color::WHITE),
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
    c.text_center(
        r,
        x + w / 2.0,
        foot + 44.0,
        color::YELLOW,
        "Enter = Play    Tab/arrows = Move    F2 = Options",
    );
    c.text_center(
        r,
        x + w / 2.0,
        foot + 56.0,
        color::YELLOW,
        "F3 = Zones again    F5 = Sandbox    Esc = Quit",
    );
}

/// The label for a field of the profile block, or `None` for the fields that are
/// drawn as something other than a line of text.
fn profile_label(field: crate::game::login::Field) -> Option<&'static str> {
    use crate::game::login::Field;
    match field {
        Field::Name | Field::Password | Field::Arena | Field::Server => Some(field.label()),
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
    opts.security = o.security;
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
