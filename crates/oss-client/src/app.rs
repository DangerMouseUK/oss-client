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
use crate::game::view::{Canvas, color};
use crate::gfx::{FrameError, Renderer, TextureId, UiFrame};

/// What to play.
pub enum Mode {
    /// Offline, on a local map (or the built-in one).
    Sandbox { map: Option<PathBuf>, ship: u8 },
    /// Online, in a zone.
    Zone(ServerOptions),
    /// The login screen, optionally with the zone filled in.
    Login { server: Option<String> },
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
    /// Macro keys from the settings file.
    pub macros: Vec<(crate::game::input::Binding, String)>,
    /// Directory servers for the zone browser.
    pub directories: Vec<String>,
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
    /// The zone browser, when open (F3).
    browser: Option<crate::game::zones::ZoneBrowser>,
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
        Mode::Zone(_) | Mode::Login { .. } | Mode::Replay(_) => Vec::new(),
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
            Mode::Login { .. } => "Open SubSpace".to_string(),
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
                        s.open_settings = self.options.open_settings;
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
            Mode::Login { server } => {
                let (art, atlas) = match load_art(&self.dir, &mut renderer) {
                    Ok(a) => a,
                    Err(e) => return self.fail(event_loop, e),
                };
                let mut login = Login::load(
                    self.options.login_file.as_deref(),
                    self.options.player_name.as_deref(),
                );
                if let Some(s) = server {
                    login.server = s.clone();
                }
                let browser = self
                    .options
                    .open_zones
                    .then(|| crate::game::zones::ZoneBrowser::open(&self.options.directories));
                Screen::Login(Box::new(LoginScreen {
                    login,
                    art,
                    atlas,
                    browser,
                }))
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
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.input.key(code, down);
                }
                if down && let Some(text) = &event.text {
                    self.input.typed.push_str(text);
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
        if self.options.autopilot {
            self.input.autopilot(run.started.elapsed().as_secs_f32());
        }

        // Login: wait for a choice.
        if let Screen::Login(ls) = &mut run.screen {
            let pressed = self.input.take_pressed();
            let typed = std::mem::take(&mut self.input.typed);
            self.input.take_edges();
            let choice = if let Some(b) = ls.browser.as_mut() {
                b.pump();
                match b.input(&pressed) {
                    Some(crate::game::zones::BrowserChoice::Pick(address)) => {
                        ls.login.server = address;
                        ls.login.error = None;
                        ls.browser = None;
                    }
                    Some(crate::game::zones::BrowserChoice::Back) => ls.browser = None,
                    None => {}
                }
                None
            } else {
                ls.login.input(&pressed, &typed, self.input.shift())
            };
            match choice {
                Some(Choice::Browse) => {
                    ls.browser = Some(crate::game::zones::ZoneBrowser::open(
                        &self.options.directories,
                    ));
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
                Some(Choice::Connect(opts)) => {
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
                            browser: None,
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
            let audio = if audio_wanted {
                Audio::new(&self.dir, self.options.volume)
            } else {
                None
            };
            match Sandbox::new_online(&mut run.renderer, &self.dir, conn.net, run.scale, audio) {
                Ok(mut s) => {
                    s.aim_bot = self.options.autopilot;
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
                if game.quit {
                    event_loop.exit();
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
                match &ls.browser {
                    Some(b) => draw_browser(b, &ls.art, ls.atlas, &mut run.renderer, run.scale),
                    None => draw_login(&ls.login, &ls.art, ls.atlas, &mut run.renderer, run.scale),
                }
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

/// The login form, centred.
fn draw_login(login: &Login, art: &Art, atlas: TextureId, r: &mut Renderer, scale: u32) {
    let c = Canvas::new(art, atlas, r.size(), scale, (0.0, 0.0));
    let w = 52.0 * 8.0;
    let rows = Login::fields().len() as f32;
    let h = 30.0 + rows * 16.0 + 70.0;
    let x = ((c.view_w - w) / 2.0).floor();
    let mut y = ((c.view_h - h) / 2.0).floor();
    c.rect(
        r,
        x - 10.0,
        y - 10.0,
        w + 20.0,
        h + 20.0,
        [0.0, 0.0, 0.0, 0.85],
    );
    c.text(r, x, y, color::YELLOW, "Open SubSpace");
    y += 26.0;
    for (i, &field) in Login::fields().iter().enumerate() {
        let focused = i == login.focus % Login::fields().len();
        let label = format!("{:>9}", field.label());
        c.text(r, x, y, color::GREEN, &label);
        let mut value = login.value(field);
        if focused {
            value = match field {
                crate::game::login::Field::Ship => format!("< {value} >"),
                _ => format!("{value}_"),
            };
            c.rect(r, x + 84.0, y - 2.0, w - 84.0, 14.0, [0.2, 0.25, 0.45, 0.8]);
        }
        let max = ((w - 92.0) / 8.0) as usize;
        let shown: String = if value.chars().count() > max {
            value.chars().skip(value.chars().count() - max).collect()
        } else {
            value
        };
        c.text(r, x + 88.0, y, color::WHITE, &shown);
        y += 16.0;
    }
    y += 10.0;
    if let Some(err) = &login.error {
        c.text(r, x, y, color::RED, err);
    }
    y += 20.0;
    c.text(
        r,
        x,
        y,
        color::WHITE,
        "Enter connect   Tab/arrows move   F3 zone list",
    );
    c.text(r, x, y + 14.0, color::WHITE, "F5 sandbox   Esc quit");
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

/// The zone browser: name, players and ping per zone, the selected one highlighted.
fn draw_browser(
    b: &crate::game::zones::ZoneBrowser,
    art: &Art,
    atlas: TextureId,
    r: &mut Renderer,
    scale: u32,
) {
    let c = Canvas::new(art, atlas, r.size(), scale, (0.0, 0.0));
    let w = (c.view_w - 40.0).min(72.0 * 8.0);
    let x = ((c.view_w - w) / 2.0).floor();
    let rows = (((c.view_h - 120.0) / 14.0).floor() as usize).max(3);
    let top = b.selected.saturating_sub(rows - 1);
    let mut y = 30.0;
    c.rect(
        r,
        x - 10.0,
        y - 10.0,
        w + 20.0,
        c.view_h - 40.0,
        [0.0, 0.0, 0.0, 0.85],
    );
    c.text(r, x, y, color::YELLOW, "Zones");
    c.text_right(r, x + w, y, color::GREEN, "Players  Ping");
    y += 22.0;
    let name_chars = ((w - 14.0 * 8.0) / 8.0) as usize;
    for (i, z) in b.zones.iter().enumerate().skip(top).take(rows) {
        if i == b.selected {
            c.rect(r, x - 4.0, y - 2.0, w + 8.0, 14.0, [0.2, 0.25, 0.45, 0.8]);
        }
        let name: String = z.name.chars().take(name_chars).collect();
        c.text(r, x, y, color::WHITE, &name);
        let ping = z.ping_ms.map_or_else(|| "-".to_string(), |p| p.to_string());
        c.text_right(
            r,
            x + w,
            y,
            color::WHITE,
            &format!("{:>7}  {ping:>4}", z.players),
        );
        y += 14.0;
    }
    let bottom = c.view_h - 46.0;
    if let Some(z) = b.zones.get(b.selected) {
        let desc: String = z.description.chars().take((w / 8.0) as usize).collect();
        c.text(r, x, bottom - 14.0, color::GREEN, &desc);
    }
    c.text(r, x, bottom, color::WHITE, &b.status);
    c.text(
        r,
        x,
        bottom + 14.0,
        color::WHITE,
        "Up/Down choose   Enter use   Esc back",
    );
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
}
