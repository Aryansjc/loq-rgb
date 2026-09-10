//! egui desktop application.
//!
//! Thin view layer over the library: the GUI owns a [`crate::controller::Controller`]
//! (or shows why none could be opened), a [`crate::config::ConfigStore`] and an
//! in-editing [`LightingConfig`] draft. All policy (which effect may use zone
//! colours, what the hardware can and cannot do) lives in the library and is
//! only *rendered* here — the UI never guesses a capability.

use std::time::Duration;

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::config::{ConfigStore, config_dir};
use crate::controller::Controller;
use crate::effects::{self, UnsupportedReason};
use crate::error::Error;
use crate::model::{DeviceState, Effect, LightingConfig, LiveEffect, Rgb, ZONE_COUNT};

/// Entry point used by `bin/loq-rgb.rs`.
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 640.0])
            .with_min_inner_size([760.0, 540.0])
            .with_title("LOQ RGB — keyboard lighting controller"),
        ..Default::default()
    };
    eframe::run_native(
        "LOQ RGB",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::new()))
        }),
    )
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct App {
    controller: Option<Controller>,
    open_error: Option<Error>,
    store: ConfigStore,
    store_error: Option<String>,
    /// Informational (non-error) message shown in the status bar.
    notice: Option<String>,
    /// Profile awaiting delete confirmation (modal).
    pending_delete: Option<String>,
    /// Profile awaiting rename (modal) and its new name buffer.
    pending_rename: Option<String>,
    rename_new_name: String,
    /// Config being edited (always normalized before apply).
    draft: LightingConfig,
    dirty: bool,
    last_edit_time: f64,
    start_applied: bool,
    selected_zone: usize,
    /// Hex text buffer for the selected zone's colour field.
    hex_buf: String,
    last_apply_time: Option<String>,
    readback: Option<Result<DeviceState, Error>>,
    save_as_name: String,
    /// Gradient offset for the host-rendered colour flow.
    flow_phase: f32,
    /// When the last flow frame was written (egui time).
    last_flow_frame: Option<f64>,
    /// Set when the background daemon owns animation (single-writer rule).
    daemon_notice: Option<String>,
    /// Whether we already tried to start the background animator for this
    /// host-rendered session (prevents a spawn retry loop if it fails).
    animator_spawn_attempted: bool,
}

impl App {
    fn new() -> Self {
        let store = match ConfigStore::load() {
            Ok(s) => s,
            Err(e) => {
                // Corrupt config: store was recreated with defaults; surface
                // the message so nothing happens silently.
                let mut s = ConfigStore::load_from(&config_dir().join("config.json"))
                    .unwrap_or_else(|_| ConfigStore {
                        path: config_dir().join("config.json"),
                        cfg: crate::config::AppConfig::default(),
                    });
                // Recreate defaults if the second load also failed.
                let msg = e.to_string();
                if s.cfg.profiles.is_empty() {
                    s.cfg = crate::config::AppConfig::default();
                }
                let _ = s.save();
                let _ = (msg.clone(), &mut s);
                return Self::from_store(s, Some(msg));
            }
        };
        Self::from_store(store, None)
    }

    fn from_store(store: ConfigStore, store_error: Option<String>) -> Self {
        let mut store = store;
        // The Fn+Space cycle ends with an Off step; expose it in the profile
        // list so the GUI matches the cycle order.
        if crate::hotkey::ensure_off_profile(&mut store.cfg) {
            let _ = store.save();
        }
        let draft = store.cfg.active_config();
        let hex_buf = draft.zones[0].to_hex();
        Self {
            controller: None,
            open_error: None,
            store,
            store_error,
            notice: None,
            pending_delete: None,
            pending_rename: None,
            rename_new_name: String::new(),
            draft,
            dirty: false,
            last_edit_time: 0.0,
            start_applied: false,
            selected_zone: 0,
            hex_buf,
            last_apply_time: None,
            readback: None,
            save_as_name: String::new(),
            flow_phase: 0.0,
            last_flow_frame: None,
            daemon_notice: None,
            animator_spawn_attempted: false,
        }
    }

    fn try_open(&mut self) {
        if self.controller.is_some() {
            return;
        }
        match Controller::open_hardware() {
            Ok(c) => {
                self.controller = Some(c);
                self.open_error = None;
                self.start_applied = false; // apply draft once on (re)connect
            }
            Err(e) => self.open_error = Some(e),
        }
    }

    fn colours_editable(&self) -> bool {
        self.draft.effect.uses_zone_colors()
    }

    /// Mutate the draft; records the edit time so the frame loop can debounce
    /// hardware writes during continuous drags.
    fn edit(&mut self, f: impl FnOnce(&mut LightingConfig), now: f64) {
        f(&mut self.draft);
        self.draft = self.draft.normalized();
        self.dirty = true;
        self.last_edit_time = now;
        self.hex_buf = self.draft.zones[self.selected_zone].to_hex();
    }

    /// Debounced writer: called once per frame; sends only when the user has
    /// paused editing for 150 ms.
    fn flush_pending(&mut self, now: f64) {
        if !self.dirty {
            return;
        }
        if now - self.last_edit_time < 0.150 {
            return;
        }
        self.dirty = false;
        self.apply_draft(now);
    }

    fn apply_draft(&mut self, now: f64) {
        let Some(controller) = self.controller.as_mut() else {
            return;
        };
        match controller.apply(self.draft) {
            Ok(()) => {
                self.last_apply_time = Some(format_time(now));
                self.start_applied = true;
            }
            Err(_) => {
                // controller.last_error() carries the detail for the banner
            }
        }
        // Persist the active profile so the CLI, the background animator and
        // the next start all see exactly what is on the keyboard. Wave frames
        // are written by the animation loop, not here, so this never runs at
        // frame rate.
        let name = self.store.cfg.active_profile.clone();
        if self.store.cfg.profiles.get(&name) != Some(&self.draft)
            && let Err(e) = self.store.upsert_profile(&name, self.draft)
        {
            self.store_error = Some(e.to_string());
        }
    }
}

fn format_time(t: f64) -> String {
    let secs = t as u64;
    format!(
        "{:02}:{:02}:{:02}",
        (secs / 3600) % 24,
        (secs / 60) % 60,
        secs % 60
    )
}

// ---------------------------------------------------------------------------
// eframe::App
// ---------------------------------------------------------------------------

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);

        if self.controller.is_none() {
            self.try_open();
        } else if !self.start_applied && self.open_error.is_none() {
            // Apply the active profile once on connect, like the plan's
            // "apply active profile on start" behaviour.
            self.dirty = false;
            self.apply_draft(now);
        }

        self.flush_pending(now);

        // Host-rendered colour flow: keep writing frames while this effect is
        // active and the controller is connected.
        if self.controller.is_some() {
            self.animate_host_flow(&ctx, now);
        }

        // Periodic retry when the controller is missing.
        if self.controller.is_none() {
            ctx.request_repaint_after(Duration::from_millis(1500));
        }

        egui::Panel::top("top").show(ui, |ui| self.top_panel(ui));
        egui::Panel::left("zones")
            .resizable(true)
            .default_size(440.0)
            .show(ui, |ui| self.left_panel(ui, now));
        egui::Panel::bottom("status").show(ui, |ui| self.bottom_panel(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| self.central_panel(ui, now));

        self.delete_confirmation_dialog(&ctx);
        self.rename_dialog(&ctx);
    }
}

impl App {
    /// Rename the selected (active) profile. Confirmation is inline: a name
    /// field plus Rename/Cancel. Renames never touch the hardware and the
    /// active profile stays active.
    fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(name) = self.pending_rename.clone() else {
            return;
        };
        let mut decision = false;
        egui::Window::new("Rename profile")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Rename {name:?} to:"));
                let mut buf = self.rename_new_name.clone();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut buf)
                        .desired_width(220.0)
                        .hint_text("new profile name"),
                );
                self.rename_new_name = buf;
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    decision = true;
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let can_rename = !self.rename_new_name.trim().is_empty()
                        && name != crate::config::OFF_PROFILE;
                    if ui
                        .add_enabled(can_rename, egui::Button::new("Rename"))
                        .clicked()
                    {
                        decision = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.pending_rename = None;
                    }
                });
            });
        if decision {
            let new_name = self.rename_new_name.trim().to_string();
            self.pending_rename = None;
            self.rename_new_name.clear();
            if let Err(e) = crate::profiles::rename_profile(&mut self.store, &name, &new_name) {
                self.store_error = Some(e.to_string());
            } else {
                self.notice = Some(format!("Renamed {name:?} to {new_name:?}."));
            }
        }
    }

    /// Modal confirmation before a profile is permanently deleted. Deletion
    /// never changes the keyboard lighting.
    fn delete_confirmation_dialog(&mut self, ctx: &egui::Context) {
        let Some(name) = self.pending_delete.clone() else {
            return;
        };
        let mut decision = None;
        egui::Window::new("Delete profile")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Delete profile {name:?} permanently?"));
                ui.label(
                    "The keyboard lighting will not change. If this is the active profile, \
                     the active selection moves to another profile.",
                );
                if name == crate::config::OFF_PROFILE {
                    ui.colored_label(
                        Color32::from_rgb(240, 200, 80),
                        format!(
                            "{name:?} is reserved for the Fn+Space cycle and cannot be deleted."
                        ),
                    );
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let can_delete = name != crate::config::OFF_PROFILE;
                    if ui
                        .add_enabled(can_delete, egui::Button::new("Delete"))
                        .clicked()
                    {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.pending_delete = None;
                self.execute_delete(&name);
            }
            Some(false) => self.pending_delete = None,
            None => {}
        }
    }

    fn execute_delete(&mut self, name: &str) {
        match crate::profiles::delete_profile(&mut self.store, name) {
            Ok(Some(new_active)) => {
                // Active profile was deleted: follow the store's new active
                // selection in the editor, but do NOT touch the hardware.
                self.draft = self.store.cfg.active_config();
                self.hex_buf = self.draft.zones[0].to_hex();
                self.notice = Some(format!(
                    "Deleted {name:?}. It was active — the active profile is now {new_active:?} \
                     (keyboard lighting unchanged)."
                ));
            }
            Ok(None) => {
                self.notice = Some(format!("Deleted profile {name:?}."));
            }
            Err(e) => {
                self.store_error = Some(e.to_string());
            }
        }
    }

    /// Drive the host-rendered colour flow: advance the gradient phase and
    /// write a new static frame to the hardware every ~40 ms. Only runs while
    /// the GUI is open and repainting (the daemon's `listen-hotkeys` renders
    /// it when the GUI is closed).
    fn animate_host_flow(&mut self, ctx: &egui::Context, now: f64) {
        if !self.draft.effect.is_host_rendered() {
            self.flow_phase = 0.0;
            self.last_flow_frame = None;
            self.daemon_notice = None;
            self.animator_spawn_attempted = false;
            return;
        }

        // Single-writer rule: if the background daemon holds the writer lock
        // it owns animation of host-rendered effects. This window must not
        // write frames at the same time (that caused visible flicker) — and,
        // crucially, the daemon keeps the wave moving after this window is
        // closed.
        let lock_dir = crate::instance::default_lock_dir();
        let owner =
            crate::instance::probe(&lock_dir).filter(|o| crate::instance::process_alive(o.pid));
        if let Some(owner) = owner {
            self.daemon_notice = Some(format!(
                "Background animator (pid {}) is driving the colour wave, and keeps it \
                 running if you close this window. Saving a change? It is applied within \
                 ~1 s. Stop it with: pkill -f 'loq-rgb-cli listen-hotkeys'",
                owner.pid
            ));
            self.last_flow_frame = None;
            ctx.request_repaint_after(Duration::from_millis(500));
            return;
        }

        // No background animator: start one so the wave keeps playing after
        // this window closes. Try only once per host-rendered session so a
        // failure falls back to window animation instead of a spawn loop.
        if !self.animator_spawn_attempted {
            self.animator_spawn_attempted = true;
            match crate::daemon::spawn_background_animator() {
                Ok(pid) => {
                    self.daemon_notice = Some(format!(
                        "Starting the background animator (pid {pid}) so the colour wave keeps \
                         running when this window is closed."
                    ));
                    self.last_flow_frame = None;
                    ctx.request_repaint_after(Duration::from_millis(500));
                    return;
                }
                Err(e) => {
                    self.daemon_notice = Some(format!(
                        "Animating from this window (could not start the background animator: \
                         {e}). The wave pauses when you close this window."
                    ));
                }
            }
        }

        let interval = crate::flow::FRAME_INTERVAL as f64;
        let elapsed = match self.last_flow_frame {
            Some(last) => (now - last).min(0.25),
            None => interval, // send the first frame immediately
        };
        if elapsed < interval {
            // Next frame due soon; ask egui to repaint then.
            ctx.request_repaint_after(Duration::from_secs_f64(interval - elapsed));
            return;
        }
        self.last_flow_frame = Some(now);
        self.flow_phase = crate::flow::advance_phase(
            self.draft.effect,
            self.draft.speed,
            self.flow_phase,
            elapsed as f32,
        );
        let zones = crate::flow::frame_zones(&self.draft, self.flow_phase);
        let frame = LightingConfig {
            effect: self.draft.effect,
            speed: self.draft.speed,
            brightness: self.draft.brightness,
            zones,
        };
        if let Some(controller) = self.controller.as_mut()
            && controller.apply(frame).is_ok()
        {
            self.last_apply_time = Some(format_time(now));
            self.start_applied = true;
        }
        ctx.request_repaint_after(Duration::from_secs_f64(interval));
    }

    fn top_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("LOQ RGB");
                ui.label("— Lenovo 4-zone keyboard lighting");
                ui.separator();
                match &self.controller {
                    Some(c) => {
                        ui.colored_label(Color32::from_rgb(90, 220, 120), "● controller connected");
                        ui.weak(c.device_desc());
                    }
                    None => {
                        ui.colored_label(Color32::from_rgb(240, 90, 90), "● no controller");
                        if let Some(e) = &self.open_error {
                            ui.weak(e.to_string());
                        }
                    }
                }
            });
            ui.add_space(4.0);
        });
    }

    fn left_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        egui::Panel::left("zones").show(ui, |ui| {
            ui.add_space(8.0);
            ui.heading("Zones");

            if self.colours_editable() {
                ui.label(
                    "Four independent colour zones. Edit one zone without touching the others.",
                );
                if matches!(self.draft.effect, Effect::FlowLeft | Effect::FlowRight) {
                    if crate::flow::uses_palette(&self.draft) {
                        ui.weak(
                            "Colour wave: these four colours are the wave's colour blocks — \
                             they glide across the keyboard with smooth blends between them. \
                             Runs while this window or the listener daemon is open.",
                        );
                    } else {
                        ui.weak(
                            "All four zone colours are identical — the colour wave shows the \
                             full colour spectrum instead. Pick different zone colours to make \
                             your own palette wave.",
                        );
                    }
                } else if self.draft.effect.is_animated() {
                    ui.weak(
                        "For the wave effects these four colours form the moving wave's palette \
                         (verified on this hardware).",
                    );
                }
            } else {
                ui.colored_label(
                    Color32::from_rgb(240, 200, 80),
                    UnsupportedReason::EffectIgnoresZoneColors.text(),
                );
            }

            self.zone_selector(ui);
            ui.add_space(6.0);

            let editable = self.colours_editable();
            ui.add_enabled_ui(editable, |ui| {
                self.colour_editor(ui, now);
            });
        });
    }

    fn zone_selector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for zone in 0..ZONE_COUNT {
                let colour = self.draft.zones[zone];
                let selected = self.selected_zone == zone;
                let label = format!("Zone {}", zone + 1);
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(84.0, 52.0), Sense::click());
                if ui.is_rect_visible(rect) {
                    let painter = ui.painter();
                    // Colour block
                    let block = Rect::from_min_size(rect.min, Vec2::new(84.0, 34.0)).shrink(2.0);
                    painter.rect_filled(block, 6, colour_to_color32(colour));
                    // Border to indicate selection
                    let stroke = if selected {
                        Stroke::new(2.0, Color32::WHITE)
                    } else {
                        Stroke::new(1.0, Color32::from_gray(90))
                    };
                    painter.rect_stroke(rect, 6, stroke, egui::epaint::StrokeKind::Inside);
                    painter.text(
                        Pos2::new(rect.center().x, rect.max.y - 8.0),
                        egui::Align2::CENTER_BOTTOM,
                        label,
                        egui::FontId::proportional(12.0),
                        Color32::from_gray(200),
                    );
                    if ui.is_rect_visible(rect) && response.clicked() {
                        self.selected_zone = zone;
                        self.hex_buf = self.draft.zones[zone].to_hex();
                    }
                }
                ui.add_space(2.0);
            }
        });
    }

    fn colour_editor(&mut self, ui: &mut egui::Ui, now: f64) {
        let zone = self.selected_zone;
        let colour = self.draft.zones[zone];

        ui.add_space(4.0);
        ui.strong(format!("Colour for zone {}", zone + 1));

        // Hue strip + SV square.
        let (h, s, v) = colour.to_hsv();
        let mut hue = h;
        let mut sat = s;
        let mut val = v;

        let (hue_changed, new_hue) = hue_strip(ui, hue);
        let (sv_changed, new_sat, new_val) = sv_square(ui, hue, sat, val);
        if hue_changed {
            hue = new_hue;
        }
        if sv_changed {
            sat = new_sat;
            val = new_val;
        }
        if hue_changed || sv_changed {
            let rgb = Rgb::from_hsv(hue, sat, val);
            self.edit(move |cfg| cfg.zones[zone] = rgb, now);
        }

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Hex");
            let mut hex = self.hex_buf.clone();
            let response = ui.add(
                egui::TextEdit::singleline(&mut hex)
                    .desired_width(90.0)
                    .hint_text("#rrggbb"),
            );
            if response.changed()
                && hex.len() == 7
                && hex.starts_with('#')
                && let Some(parsed) = Rgb::from_hex(&hex)
            {
                let zone = self.selected_zone;
                self.edit(move |cfg| cfg.zones[zone] = parsed, now);
            }
            self.hex_buf = hex;
            ui.separator();
            if ui
                .button("Save to palette")
                .on_hover_text("Keep this colour for reuse")
                .clicked()
            {
                let saved = self.draft.zones[self.selected_zone];
                if let Err(e) = self.store.add_palette_color(saved) {
                    self.store_error = Some(e.to_string());
                }
            }
        });

        ui.add_space(6.0);
        self.palette_row(ui, now);
    }

    fn palette_row(&mut self, ui: &mut egui::Ui, now: f64) {
        if !self.store.cfg.palette.is_empty() {
            ui.label("Saved colours");
            ui.horizontal_wrapped(|ui| {
                let palette = self.store.cfg.palette.clone();
                for colour in palette {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::new(22.0, 22.0), Sense::click());
                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();
                        painter.rect_filled(rect, 4, colour_to_color32(colour));
                        painter.rect_stroke(
                            rect,
                            4,
                            Stroke::new(1.0, Color32::from_gray(80)),
                            egui::epaint::StrokeKind::Inside,
                        );
                        if response.clicked() {
                            let zone = self.selected_zone;
                            let col = colour;
                            self.edit(move |cfg| cfg.zones[zone] = col, now);
                        }
                        if response.secondary_clicked() {
                            let _ = self.store.remove_palette_color(colour);
                        }
                    }
                }
            });
            ui.label(
                RichText::new("right-click a swatch to remove it")
                    .small()
                    .weak(),
            );
        } else {
            ui.weak("No saved colours yet — pick one and press “Save to palette”.");
        }
    }

    /// Simplified keyboard drawing: four vertical bands labelled 1..4.
    /// Clicking a band selects the zone (only meaningful when the effect uses
    /// zone colours; shown always so the mapping is visible).
    fn keyboard_preview(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let size = Vec2::new(360.0, 132.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let painter = ui.painter();
        let body = rect.shrink(4.0);
        painter.rect_filled(body, 8, Color32::from_gray(28));
        let band_w = (body.width() - 6.0 * (ZONE_COUNT as f32)) / (ZONE_COUNT as f32);
        for zone in 0..ZONE_COUNT {
            let x0 = body.min.x + zone as f32 * (band_w + 6.0);
            let band = Rect::from_min_max(
                Pos2::new(x0, body.min.y + 10.0),
                Pos2::new(x0 + band_w, body.max.y - 24.0),
            );
            // What the LEDs actually show: the sent colours (dimmed at Low,
            // auto rainbow palette for rainbow waves). Smooth flow drives an
            // internal colour we cannot preview; off is dark.
            let colour = match self.draft.effect {
                Effect::Off => Rgb::from_gray(24),
                Effect::Smooth => Rgb::from_gray(64),
                _ => crate::packet::led_zones(&self.draft)[zone],
            };
            painter.rect_filled(band, 4, colour_to_color32(colour));
            if self.selected_zone == zone {
                painter.rect_stroke(
                    band,
                    4,
                    Stroke::new(2.0, Color32::from_rgb(120, 190, 255)),
                    egui::epaint::StrokeKind::Inside,
                );
            }
            painter.text(
                Pos2::new(band.center().x, band.max.y + 6.0),
                egui::Align2::CENTER_TOP,
                format!("zone {}", zone + 1),
                egui::FontId::proportional(11.0),
                Color32::from_gray(160),
            );
        }
        // Clicking the visible keyboard preview selects the zone under the
        // cursor. Only when colours are editable is this meaningful, but the
        // selection state also drives the disabled zone list above.
        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && body.contains(pos)
        {
            let rel = (pos.x - body.min.x) / body.width();
            let zone = ((rel * ZONE_COUNT as f32) as usize).min(ZONE_COUNT - 1);
            self.selected_zone = zone;
            self.hex_buf = self.draft.zones[zone].to_hex();
        }
    }

    fn central_panel(&mut self, ui: &mut egui::Ui, now: f64) {
        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.add_space(8.0);
            ui.heading("Effect");
            ui.label("One global effect drives the whole keyboard — the firmware has no per-zone effect slots.");
            ui.add_space(4.0);

            // Effect list.
            let mut changed_effect = None;
            for info in effects::EFFECTS {
                let selected = self.draft.effect == info.effect;
                let label = format!("{}  —  {}", info.label, info.blurb);
                if ui.selectable_label(selected, label).clicked() {
                    changed_effect = Some(info.effect);
                }
            }
            if let Some(effect) = changed_effect {
                // Switching effect keeps the user's zone colours — the packet
                // builder omits them for effects that drive their own visuals,
                // and they reappear when the user returns to a colour effect.
                self.edit(move |cfg| cfg.effect = effect, now);
            }

            ui.add_space(10.0);
            ui.separator();

            // Speed & brightness.
            let animated = self.draft.effect.is_animated();
            let mut speed = self.draft.speed;
            ui.horizontal(|ui| {
                ui.label("Speed");
                ui.add_enabled_ui(animated, |ui| {
                    ui.add(egui::Slider::new(&mut speed, 1..=4u8).text("1 slow … 4 fast"));
                });
            });
            if animated && speed != self.draft.speed {
                self.edit(move |cfg| cfg.speed = speed, now);
            }

            let mut brightness = self.draft.brightness;
            ui.horizontal(|ui| {
                ui.label("Brightness");
                ui.add_enabled_ui(!self.draft.is_off(), |ui| {
                    ui.selectable_value(&mut brightness, 1u8, "Low (1)");
                    ui.selectable_value(&mut brightness, 2u8, "High (2)");
                });
            });
            if brightness != self.draft.brightness {
                self.edit(move |cfg| cfg.brightness = brightness, now);
            }
            if self.draft.brightness == 1 {
                ui.weak(
                    "Verified on this laptop: the firmware ignores the brightness byte, so Low \
                     is emulated by dimming the colour bytes this app controls (static, \
                     breathing, wave/rainbow palettes and colour flow). Smooth flow is \
                     unaffected.",
                );
            }

            ui.add_space(10.0);
            ui.separator();

            // Profiles.
            ui.heading("Profiles");
            ui.horizontal(|ui| {
                let names: Vec<String> = self.store.cfg.profiles.keys().cloned().collect();
                let active = self.store.cfg.active_profile.clone();
                egui::ComboBox::from_id_salt("profile")
                    .selected_text(&active)
                    .show_ui(ui, |ui| {
                        for name in &names {
                            if ui.selectable_label(*name == active, name).clicked()
                                && name != &active {
                                    let cfg = self.store.cfg.profiles[name];
                                    let n = name.clone();
                                    let mut s = self.store.clone();
                                    let _ = s.set_active_profile(&n);
                                    self.store = s;
                                    self.draft = cfg;
                                    self.dirty = true;
                                    self.last_edit_time = now;
                                    self.hex_buf = cfg.zones[0].to_hex();
                                }
                        }
                    });
                if ui.button("Save current as this profile").clicked() {
                    let name = self.store.cfg.active_profile.clone();
                    let cfg = self.draft;
                    match self.store.upsert_profile(&name, cfg) {
                        Ok(()) => {}
                        Err(e) => self.store_error = Some(e.to_string()),
                    }
                    self.dirty = true;
                    self.last_edit_time = now;
                }
                let active_profile = self.store.cfg.active_profile.clone();
                let delete_allowed = active_profile != crate::config::OFF_PROFILE;
                let delete_response =
                    ui.add_enabled(delete_allowed, egui::Button::new("Delete profile"));
                let delete_response = if delete_allowed {
                    delete_response.on_hover_text(
                        "Permanently remove the selected profile. The keyboard lighting is not changed.",
                    )
                } else {
                    delete_response.on_disabled_hover_text(
                        "Off is reserved for the Fn+Space cycle end step.",
                    )
                };
                if delete_response.clicked() {
                    self.pending_delete = Some(active_profile);
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.save_as_name)
                        .desired_width(160.0)
                        .hint_text("new profile name"),
                );
                if ui.button("Save as…").clicked() {
                    let name = self.save_as_name.trim().to_string();
                    if !name.is_empty() {
                        let cfg = self.draft;
                        match self.store.upsert_profile(&name, cfg) {
                            Ok(()) => {
                                self.save_as_name.clear();
                            }
                            Err(e) => self.store_error = Some(e.to_string()),
                        }
                    }
                }
            });
            ui.weak("Profiles store the whole keyboard configuration (effect + speed + brightness + four zone colours).");

            ui.add_space(8.0);
            self.keyboard_preview(ui);
        });
    }

    fn bottom_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(4.0);
            if let Some(store_error) = &self.store_error {
                ui.colored_label(Color32::from_rgb(240, 90, 90), format!("config: {store_error}"));
            }
            if let Some(notice) = &self.notice {
                ui.colored_label(Color32::from_rgb(150, 220, 150), notice);
            }
            if let Some(daemon) = &self.daemon_notice {
                ui.colored_label(Color32::from_rgb(240, 200, 120), daemon);
            }
            if let Some(e) = self.controller.as_ref().and_then(|c| c.last_error()) {
                ui.colored_label(Color32::from_rgb(240, 120, 90), format!("hardware: {e}"));
                ui.weak(e.hint());
            }
            ui.horizontal(|ui| {
                let applied = match &self.last_apply_time {
                    Some(t) => format!("sent to hardware at {t}"),
                    None => "nothing sent yet".to_string(),
                };
                ui.label(applied);
                ui.separator();
                if ui.button("Apply now").clicked() {
                    let now = ui.input(|i| i.time);
                    self.dirty = false;
                    self.apply_draft(now);
                }
                if ui.button("Read state from keyboard").clicked() {
                    self.readback = Some(match self.controller.as_mut() {
                        Some(c) => c.read_state(),
                        None => Err(Error::DeviceNotFound("controller not connected".into())),
                    });
                }
                if let Some(rb) = &self.readback {
                    ui.separator();
                    match rb {
                        Ok(state) => {
                            ui.label(format!("keyboard reports: {}", describe_state(state)));
                            if let Some(cfg) = self.controller.as_ref().and_then(|c| c.last_config()) {
                                let matches = state_matches_config(state, cfg);
                                if matches {
                                    ui.colored_label(Color32::from_rgb(90, 220, 120), "✓ matches applied config");
                                } else {
                                    ui.colored_label(Color32::from_rgb(240, 200, 80), "✗ differs from applied config (Fn-key cycle or external change?)");
                                }
                            }
                        }
                        Err(e) => {
                            ui.colored_label(Color32::from_rgb(240, 160, 90), format!("readback unavailable: {e}"));
                        }
                    }
                }
            });
            ui.add_space(4.0);
        });
    }
}

// ---------------------------------------------------------------------------
// Colour picker widgets (custom, no external widget dependency)
// ---------------------------------------------------------------------------

fn colour_to_color32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Saturation/value square. Returns (changed, sat, val).
fn sv_square(ui: &mut egui::Ui, hue: f32, sat: f32, val: f32) -> (bool, f32, f32) {
    let size = Vec2::new(300.0, 180.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let mut changed = false;
    let mut new_sat = sat;
    let mut new_val = val;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        // Rows with per-vertex colours give an accurate horizontal gradient.
        let rows = 72usize;
        for row in 0..rows {
            let y0 = rect.min.y + rect.height() * (row as f32 / rows as f32);
            let y1 = rect.min.y + rect.height() * ((row + 1) as f32 / rows as f32);
            let v = 1.0 - (row as f32 / (rows as f32 - 1.0));
            let c_left = colour_to_color32(Rgb::from_hsv(hue, 0.0, v));
            let c_right = colour_to_color32(Rgb::from_hsv(hue, 1.0, v));
            let row_rect = Rect::from_min_max(Pos2::new(rect.min.x, y0), Pos2::new(rect.max.x, y1));
            let mut mesh = egui::Mesh::default();
            let (tl, tr, bl, br) = (
                Pos2::new(row_rect.min.x, row_rect.min.y),
                Pos2::new(row_rect.max.x, row_rect.min.y),
                Pos2::new(row_rect.min.x, row_rect.max.y),
                Pos2::new(row_rect.max.x, row_rect.max.y),
            );
            let idx = mesh.vertices.len() as u32;
            mesh.vertices.push(egui::epaint::Vertex {
                pos: tl,
                uv: egui::epaint::WHITE_UV,
                color: c_left,
            });
            mesh.vertices.push(egui::epaint::Vertex {
                pos: tr,
                uv: egui::epaint::WHITE_UV,
                color: c_right,
            });
            mesh.vertices.push(egui::epaint::Vertex {
                pos: bl,
                uv: egui::epaint::WHITE_UV,
                color: c_left,
            });
            mesh.vertices.push(egui::epaint::Vertex {
                pos: br,
                uv: egui::epaint::WHITE_UV,
                color: c_right,
            });
            mesh.indices
                .extend_from_slice(&[idx, idx + 1, idx + 2, idx + 2, idx + 1, idx + 3]);
            painter.add(mesh);
        }
        // Cursor.
        let pos = Pos2::new(
            rect.min.x + sat * rect.width(),
            rect.max.y - val * rect.height(),
        );
        painter.circle_stroke(pos, 6.0, Stroke::new(2.0, Color32::WHITE));
        painter.circle_stroke(pos, 6.0, Stroke::new(1.0, Color32::BLACK));

        if (response.dragged() || response.clicked())
            && let Some(p) = response.interact_pointer_pos()
        {
            let rel = (p - rect.min) / Vec2::new(rect.width(), rect.height());
            new_sat = rel.x.clamp(0.0, 1.0);
            new_val = (1.0 - rel.y).clamp(0.0, 1.0);
            changed = (new_sat - sat).abs() > 0.002 || (new_val - val).abs() > 0.002;
        }
    }
    (changed, new_sat, new_val)
}

/// Vertical hue strip (rainbow gradient). Returns (changed, hue_degrees).
fn hue_strip(ui: &mut egui::Ui, hue: f32) -> (bool, f32) {
    let size = Vec2::new(22.0, 180.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let mut changed = false;
    let mut new_hue = hue;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bands = 64usize;
        for band in 0..bands {
            let y0 = rect.min.y + rect.height() * (band as f32 / bands as f32);
            let y1 = rect.min.y + rect.height() * ((band + 1) as f32 / bands as f32);
            let h = 360.0 * (band as f32 / (bands as f32 - 1.0));
            let c = colour_to_color32(Rgb::from_hsv(h, 1.0, 1.0));
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(rect.min.x, y0), Pos2::new(rect.max.x, y1)),
                0,
                c,
            );
        }
        let cursor_y = rect.min.y + (hue / 360.0) * rect.height();
        let p = Pos2::new(rect.center().x, cursor_y);
        painter.circle_stroke(p, 7.0, Stroke::new(2.0, Color32::WHITE));
        painter.circle_stroke(p, 7.0, Stroke::new(1.0, Color32::BLACK));
        if (response.dragged() || response.clicked())
            && let Some(pos) = response.interact_pointer_pos()
        {
            let t = ((pos.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
            new_hue = 360.0 * t;
            changed = (new_hue - hue).abs() > 0.5;
        }
    }
    (changed, new_hue)
}

// ---------------------------------------------------------------------------
// Readback description helpers
// ---------------------------------------------------------------------------

fn describe_state(state: &DeviceState) -> String {
    let eff = match state.effect {
        LiveEffect::Known(e) => effects::info(e).label.to_string(),
        LiveEffect::Unknown(code) => format!("unknown effect (0x{code:02x})"),
    };
    let zones: Vec<String> = state.zones.iter().map(|z| z.to_hex()).collect();
    format!(
        "{eff}, speed {}, brightness {}, zones [{}]",
        state.speed,
        state.brightness,
        zones.join(" ")
    )
}

fn state_matches_config(state: &DeviceState, cfg: &LightingConfig) -> bool {
    let effect_matches = match state.effect {
        LiveEffect::Known(e) => {
            // A host-rendered wave writes static frames, so a readback during
            // a wave legitimately reports Static.
            e == cfg.effect || (cfg.effect.is_host_rendered() && e == Effect::Static)
        }
        LiveEffect::Unknown(_) => false,
    };
    // Compare against the exact colour bytes we sent (dimmed at Low, current
    // wave frame for the colour wave) so a readback of the LEDs still matches.
    let colours_match = if cfg.effect.writes_zone_bytes() {
        state.zones == crate::packet::led_zones(cfg)
    } else {
        true // smooth/off bytes are zeroed or internal — nothing to compare
    };
    effect_matches
        && state.speed == cfg.speed
        && state.brightness == cfg.brightness
        && colours_match
}

impl Rgb {
    fn from_gray(v: u8) -> Self {
        Rgb::new(v, v, v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_to_gray_helper() {
        assert_eq!(Rgb::from_gray(60), Rgb::new(60, 60, 60));
    }

    #[test]
    fn state_matching_logic() {
        let cfg = LightingConfig {
            effect: Effect::Static,
            speed: 2,
            brightness: 2,
            zones: [Rgb::new(255, 0, 0); 4],
        };
        let same = DeviceState {
            effect: LiveEffect::Known(Effect::Static),
            speed: 2,
            brightness: 2,
            zones: [Rgb::new(255, 0, 0); 4],
            flag_right: false,
            flag_left: false,
        };
        assert!(state_matches_config(&same, &cfg));

        let different_effect = DeviceState {
            effect: LiveEffect::Known(Effect::Breath),
            ..same
        };
        assert!(!state_matches_config(&different_effect, &cfg));

        // The colour wave sends the current frame's colours: a readback must
        // show exactly those bytes (not black), and a static readback of a
        // wave frame matches because the frame IS a static packet.
        let wave_cfg = LightingConfig {
            effect: Effect::FlowLeft,
            ..cfg
        };
        let matching_wave = DeviceState {
            effect: LiveEffect::Known(Effect::Static),
            zones: crate::packet::led_zones(&wave_cfg),
            ..same
        };
        assert!(state_matches_config(&matching_wave, &wave_cfg));
        let wrong_palette = DeviceState {
            zones: [Rgb::black(); 4],
            ..matching_wave
        };
        assert!(!state_matches_config(&wrong_palette, &wave_cfg));

        // Smooth flow: no colour bytes are sent, so anything reads as matching.
        let smooth_cfg = LightingConfig {
            effect: Effect::Smooth,
            ..cfg
        };
        let smooth_state = DeviceState {
            effect: LiveEffect::Known(Effect::Smooth),
            zones: [Rgb::black(); 4],
            ..same
        };
        assert!(state_matches_config(&smooth_state, &smooth_cfg));
    }
}
