//! The settings screen (egui): video, audio, player name, key bindings and macros.
//!
//! Opened from the Esc menu (O). Edits a copy of the settings; Save writes
//! `oss-client.ini` and applies what can change while playing (bindings, macros,
//! volume, pixel scale, fullscreen). Rebinding waits for the next key press, which the
//! app captures with its modifiers (see [`SettingsUi::capture`]).

use crate::config::Config;
use crate::game::input::{Action, Binding, Mods};

/// What the screen asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    None,
    Save,
    Close,
}

/// A key being captured: replacing an action's keys, adding one, or a new macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Capture {
    Replace(Action),
    Add(Action),
    Macro,
}

pub struct SettingsUi {
    pub cfg: Config,
    capture: Option<Capture>,
    /// Shown at the bottom (saved, failed...).
    pub status: Option<String>,
    name: String,
}

pub fn binding_label(b: &Binding) -> String {
    let prefix = match b.mods {
        Mods::Any => "",
        Mods::None => "(no Shift) ",
        Mods::Shift => "Shift+",
        Mods::Ctrl => "Ctrl+",
    };
    format!("{prefix}{:?}", b.key)
}

impl SettingsUi {
    pub fn new(cfg: Config) -> Self {
        let name = cfg.name.clone().unwrap_or_default();
        Self {
            cfg,
            capture: None,
            status: None,
            name,
        }
    }

    /// Waiting for a key press to bind.
    pub fn capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// The pressed key (with Shift/Ctrl as held) for the binding being captured.
    /// Escape cancels.
    pub fn capture(&mut self, key: winit::keyboard::KeyCode, shift: bool, ctrl: bool) {
        let Some(what) = self.capture.take() else {
            return;
        };
        if key == winit::keyboard::KeyCode::Escape {
            self.status = Some("Cancelled".into());
            return;
        }
        let is_modifier = matches!(
            key,
            winit::keyboard::KeyCode::ShiftLeft
                | winit::keyboard::KeyCode::ShiftRight
                | winit::keyboard::KeyCode::ControlLeft
                | winit::keyboard::KeyCode::ControlRight
        );
        let mods = match (ctrl && !is_modifier, shift && !is_modifier) {
            (true, _) => Mods::Ctrl,
            (_, true) => Mods::Shift,
            _ => Mods::Any,
        };
        let b = Binding { key, mods };
        match what {
            Capture::Replace(a) => self.cfg.bindings.set(a, vec![b]),
            Capture::Add(a) => self.cfg.bindings.add(a, b),
            Capture::Macro => self.cfg.macros.push((b, String::new())),
        }
        self.status = Some(format!("Bound {}", binding_label(&b)));
    }

    /// Draws the window; returns what the player chose.
    pub fn show(&mut self, ctx: &egui::Context) -> UiAction {
        let mut action = UiAction::None;
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable(true)
            .default_size([560.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.7)
                    .show(ui, |ui| self.sections(ui));
                ui.separator();
                if let Some(c) = self.capture {
                    let what = match c {
                        Capture::Replace(a) | Capture::Add(a) => a.name().to_string(),
                        Capture::Macro => "a new macro".into(),
                    };
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!("Press a key for {what} (Esc cancels)"),
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.cfg.name =
                            Some(self.name.trim().to_string()).filter(|n| !n.is_empty());
                        action = UiAction::Save;
                    }
                    if ui.button("Close").clicked() {
                        action = UiAction::Close;
                    }
                    if let Some(s) = &self.status {
                        ui.label(s);
                    }
                });
            });
        action
    }

    fn sections(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Video")
            .default_open(true)
            .show(ui, |ui| {
                let mut scale = self.cfg.scale.unwrap_or(0);
                ui.add(
                    egui::Slider::new(&mut scale, 0..=8).text("pixel scale (0 = from the display)"),
                );
                self.cfg.scale = (scale > 0).then_some(scale);
                ui.checkbox(&mut self.cfg.fullscreen, "Fullscreen");
                ui.checkbox(&mut self.cfg.vsync, "Vsync (applies next start)");
            });
        egui::CollapsingHeader::new("Audio")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut self.cfg.mute, "Mute (applies next start)");
                ui.add(egui::Slider::new(&mut self.cfg.volume, 0..=100).text("volume"));
            });
        egui::CollapsingHeader::new("Accessibility")
            .default_open(true)
            .show(ui, |ui| {
                egui::ComboBox::from_label("colour palette")
                    .selected_text(self.cfg.palette.name())
                    .show_ui(ui, |ui| {
                        for p in crate::game::palette::PaletteKind::ALL {
                            ui.selectable_value(&mut self.cfg.palette, p, p.name());
                        }
                    });
                ui.add(
                    egui::Slider::new(&mut self.cfg.ui_scale, 0..=4)
                        .text("larger interface (text, HUD)"),
                );
            });
        egui::CollapsingHeader::new("Player")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.name);
                });
            });
        egui::CollapsingHeader::new("Keys").show(ui, |ui| {
            egui::Grid::new("keys").striped(true).show(ui, |ui| {
                for a in Action::ALL {
                    ui.label(a.name());
                    let keys: Vec<String> =
                        self.cfg.bindings.get(a).iter().map(binding_label).collect();
                    ui.label(if keys.is_empty() {
                        "(none)".to_string()
                    } else {
                        keys.join(", ")
                    });
                    if ui.small_button("Change").clicked() {
                        self.capture = Some(Capture::Replace(a));
                    }
                    if ui.small_button("Add").clicked() {
                        self.capture = Some(Capture::Add(a));
                    }
                    if ui.small_button("Clear").clicked() {
                        self.cfg.bindings.set(a, Vec::new());
                    }
                    ui.end_row();
                }
            });
            if ui.button("Restore Continuum defaults").clicked() {
                self.cfg.bindings = Default::default();
            }
        });
        egui::CollapsingHeader::new("Macros").show(ui, |ui| {
            ui.label("A key sends its line as chat (same prefixes as the chat box).");
            let mut remove = None;
            egui::Grid::new("macros").striped(true).show(ui, |ui| {
                for (i, (b, text)) in self.cfg.macros.iter_mut().enumerate() {
                    ui.label(binding_label(b));
                    ui.add(egui::TextEdit::singleline(text).desired_width(300.0));
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
            if let Some(i) = remove {
                self.cfg.macros.remove(i);
            }
            if ui.button("Add a macro key").clicked() {
                self.capture = Some(Capture::Macro);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    #[test]
    fn capturing_keys() {
        let mut ui = SettingsUi::new(Config::default());
        ui.capture = Some(Capture::Replace(Action::Gun));
        ui.capture(KeyCode::Space, false, true);
        assert_eq!(
            ui.cfg.bindings.get(Action::Gun),
            &[Binding {
                key: KeyCode::Space,
                mods: Mods::Ctrl
            }]
        );
        assert!(!ui.capturing());
        // A modifier key on its own doesn't count as its own modifier.
        ui.capture = Some(Capture::Add(Action::Gun));
        ui.capture(KeyCode::ControlRight, false, true);
        assert_eq!(ui.cfg.bindings.get(Action::Gun).len(), 2);
        assert_eq!(ui.cfg.bindings.get(Action::Gun)[1].mods, Mods::Any);
        // Escape cancels; macros get an empty line to fill in.
        ui.capture = Some(Capture::Macro);
        ui.capture(KeyCode::Escape, false, false);
        assert!(ui.cfg.macros.is_empty());
        ui.capture = Some(Capture::Macro);
        ui.capture(KeyCode::F9, true, false);
        assert_eq!(ui.cfg.macros.len(), 1);
        assert_eq!(ui.cfg.macros[0].0.mods, Mods::Shift);
    }
}
