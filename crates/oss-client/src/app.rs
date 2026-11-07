//! Window/event loop glue: owns the window, renderer, input devices and the sandbox.

use std::path::PathBuf;
use std::sync::Arc;

use oss_assets::ContinuumDir;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use crate::game::input::InputState;
use crate::game::sandbox::Sandbox;
use crate::gfx::{FrameError, Renderer};

pub struct SandboxOptions {
    pub map: PathBuf,
    pub continuum_dir: Option<PathBuf>,
    pub ship: u8,
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
}

struct Running {
    window: Arc<Window>,
    renderer: Renderer,
    sandbox: Sandbox,
    started: std::time::Instant,
    captured: bool,
}

struct App {
    options: SandboxOptions,
    dir: ContinuumDir,
    level: Vec<u8>,
    running: Option<Running>,
    input: InputState,
    gilrs: Option<gilrs::Gilrs>,
    error: Option<String>,
}

pub fn run_sandbox(options: SandboxOptions) -> Result<(), String> {
    let dir = ContinuumDir::locate(options.continuum_dir.as_deref()).map_err(|e| {
        format!(
            "{e}\nPoint --continuum-dir (or {}) at your Continuum folder.",
            oss_assets::ENV_VAR
        )
    })?;
    log::info!("Continuum assets: {}", dir.root().display());
    let level = oss_assets::locate::read_file(&options.map).map_err(|e| e.to_string())?;

    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let gilrs = match gilrs::Gilrs::new() {
        Ok(g) => Some(g),
        Err(e) => {
            log::warn!("gamepads unavailable: {e}");
            None
        }
    };
    let mut app = App {
        options,
        dir,
        level,
        running: None,
        input: InputState::default(),
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
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title("Open SubSpace — sandbox")
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
        let audio = if self.options.mute {
            None
        } else {
            crate::audio::Audio::new(&self.dir)
        };
        match Sandbox::new(
            &mut renderer,
            &self.dir,
            &self.level,
            self.options.ship,
            scale,
            audio,
        ) {
            Ok(sandbox) => {
                window.request_redraw();
                self.running = Some(Running {
                    window,
                    renderer,
                    sandbox,
                    started: std::time::Instant::now(),
                    captured: false,
                });
            }
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(run) = self.running.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => run.renderer.resize(size.width, size.height),
            WindowEvent::Focused(false) => self.input.clear(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.input.key(code, event.state == ElementState::Pressed);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(g) = self.gilrs.as_mut() {
                    while let Some(ev) = g.next_event() {
                        self.input.gamepad(ev.event);
                    }
                }
                if self.options.autopilot {
                    self.input.autopilot(run.started.elapsed().as_secs_f32());
                }
                run.sandbox.update(&mut self.input);
                if let Some(path) = &self.options.screenshot {
                    if run.captured {
                        event_loop.exit();
                        return;
                    }
                    if run.started.elapsed().as_secs_f32() >= self.options.screenshot_at {
                        run.renderer.capture = Some(path.clone());
                        run.captured = true;
                    }
                }
                if run.sandbox.quit {
                    event_loop.exit();
                    return;
                }
                let stars = run.sandbox.draw(&mut run.renderer);
                match run.renderer.render(Some(stars), 0.0) {
                    Ok(()) | Err(FrameError::Skip) => {}
                    Err(FrameError::Fatal(e)) => return self.fail(event_loop, e),
                }
                run.window.request_redraw();
            }
            _ => {}
        }
    }
}
