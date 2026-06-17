//! The egui application: a visual keyboard plus side-panel controls.

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use eframe::CreationContext;
use egui::{
    Align2, Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, StrokeKind, pos2,
    vec2,
};

#[cfg(not(target_arch = "wasm32"))]
use crate::config::Profile;
use crate::ipc::Status;
use crate::protocol::consts::{ACTUATION_MAX, ACTUATION_MIN, LedSequence, TOTAL_KEYS};
use crate::protocol::layout::{self, WASD_KEYS};

use super::worker::{Command, Event, Worker};

/// A visual key: a device-slot name (empty = gap) and its width in key units.
type Vk = (&'static str, f32);

/// Hand-authored A75 (75% ANSI) layout for the main block, in key units. Names
/// map to device slots via [`layout::index_of`].
#[rustfmt::skip]
const MAIN_ROWS: &[&[Vk]] = &[
    &[("ESC", 1.0), ("", 0.5),
      ("F1", 1.0), ("F2", 1.0), ("F3", 1.0), ("F4", 1.0), ("", 0.5),
      ("F5", 1.0), ("F6", 1.0), ("F7", 1.0), ("F8", 1.0), ("", 0.5),
      ("F9", 1.0), ("F10", 1.0), ("F11", 1.0), ("F12", 1.0)],
    &[("TILDE", 1.0), ("1", 1.0), ("2", 1.0), ("3", 1.0), ("4", 1.0), ("5", 1.0),
      ("6", 1.0), ("7", 1.0), ("8", 1.0), ("9", 1.0), ("0", 1.0),
      ("MINUS", 1.0), ("PLUS", 1.0), ("BACK", 2.0)],
    &[("TAB", 1.5), ("Q", 1.0), ("W", 1.0), ("E", 1.0), ("R", 1.0), ("T", 1.0),
      ("Y", 1.0), ("U", 1.0), ("I", 1.0), ("O", 1.0), ("P", 1.0),
      ("BRKTS_L", 1.0), ("BRKTS_R", 1.0), ("SLASH_K29", 1.5)],
    &[("CAPS", 1.75), ("A", 1.0), ("S", 1.0), ("D", 1.0), ("F", 1.0), ("G", 1.0),
      ("H", 1.0), ("J", 1.0), ("K", 1.0), ("L", 1.0), ("COLON", 1.0),
      ("QOTATN", 1.0), ("RETURN", 2.25)],
    &[("SHF_L", 2.25), ("Z", 1.0), ("X", 1.0), ("C", 1.0), ("V", 1.0), ("B", 1.0),
      ("N", 1.0), ("M", 1.0), ("COMMA", 1.0), ("PERIOD", 1.0), ("SLASH", 1.0),
      ("SHF_R", 2.75)],
    &[("CTRL_L", 1.25), ("WIN_L", 1.25), ("ALT_L", 1.25), ("SPACE", 6.25),
      ("ALT_R", 1.25), ("FN1", 1.25), ("APP", 1.25)],
];

/// Right-hand nav column + inverted-T arrow cluster, aligned row-for-row with
/// [`MAIN_ROWS`]. `ARR_UP` is offset to sit above `ARR_DW` (down).
#[rustfmt::skip]
const RIGHT_ROWS: &[&[Vk]] = &[
    &[("HOME", 1.0)],
    &[("PGUP", 1.0)],
    &[("PGDN", 1.0)],
    &[("END", 1.0)],
    &[("", 1.0), ("ARR_UP", 1.0)],
    &[("ARR_L", 1.0), ("ARR_DW", 1.0), ("ARR_R", 1.0)],
];

const MM_MIN: f32 = ACTUATION_MIN as f32 / 10.0;
const MM_MAX: f32 = ACTUATION_MAX as f32 / 10.0;

/// Width of one key unit and key height, in points.
const KEY_U: f32 = 40.0;
const KEY_H: f32 = 48.0;
const KEY_GAP: f32 = 5.0;
/// Approximate maximum travel reading from the key-depth stream (about 4.0 mm).
const DEPTH_MAX_RAW: u8 = 40;

// ── palette ─────────────────────────────────────────────────────────────
const BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1e);
const CARD: Color32 = Color32::from_rgb(0x20, 0x23, 0x2b);
const CAP_BG: Color32 = Color32::from_rgb(0x2b, 0x30, 0x3b);
const ACCENT: Color32 = Color32::from_rgb(0x6d, 0x8c, 0xff);
const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xf0);
const TEXT_DIM: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xad);
const OK: Color32 = Color32::from_rgb(0x4f, 0xc8, 0x82);
const BAD: Color32 = Color32::from_rgb(0xe0, 0x6b, 0x6b);

const SEQUENCES: &[(LedSequence, &str)] = &[
    (LedSequence::Off, "Off"),
    (LedSequence::Always, "Always on"),
    (LedSequence::Spectrum, "Spectrum"),
    (LedSequence::Breath, "Breath"),
    (LedSequence::Press, "Press"),
    (LedSequence::Stars, "Stars"),
    (LedSequence::Wave, "Wave"),
    (LedSequence::Surf, "Surf"),
    (LedSequence::Ripple, "Ripple"),
    (LedSequence::Snake, "Snake"),
];

pub struct App {
    worker: Worker,

    info: Option<Status>,
    connected: bool,
    status: String,

    actuation: [f32; TOTAL_KEYS],
    selected: [bool; TOTAL_KEYS],
    rapid_trigger: bool,
    turbo: bool,

    /// Live key-depth view: when on, caps show real-time travel.
    live: bool,
    depths: [u8; TOTAL_KEYS],

    brush_mm: f32,
    global_mm: f32,

    led_sequence: LedSequence,
    led_speed: u8,
    led_brightness: u8,
    led_color: u8,
}

impl App {
    pub fn new(cc: &CreationContext<'_>) -> App {
        setup_style(&cc.egui_ctx);
        App {
            worker: Worker::spawn(cc.egui_ctx.clone()),
            info: None,
            connected: false,
            status: "starting…".into(),
            actuation: [2.0; TOTAL_KEYS],
            selected: [false; TOTAL_KEYS],
            rapid_trigger: false,
            turbo: false,
            live: false,
            depths: [0; TOTAL_KEYS],
            brush_mm: 1.5,
            global_mm: 2.0,
            led_sequence: LedSequence::Off,
            led_speed: 5,
            led_brightness: 9,
            led_color: 0xff,
        }
    }

    fn drain_events(&mut self) {
        for evt in self.worker.poll() {
            match evt {
                Event::Connected(status) => {
                    self.connected = true;
                    self.rapid_trigger = status.rapid_trigger;
                    self.turbo = status.turbo;
                    self.status = format!("Connected to {}", status.model);
                    self.info = Some(status);
                }
                Event::Disconnected(why) => {
                    self.connected = false;
                    self.info = None;
                    self.status = format!("Disconnected: {why}");
                }
                Event::Status(s) => self.status = s,
                Event::Error(e) => self.status = format!("Error: {e}"),
                Event::Depths(frame) => self.depths = *frame,
            }
        }
    }

    fn selected_indices(&self) -> Vec<usize> {
        (0..TOTAL_KEYS).filter(|&i| self.selected[i]).collect()
    }

    fn apply_to_keyboard(&self) {
        self.worker
            .send(Command::ApplyActuation(self.actuation.to_vec()));
        self.worker.send(Command::SetRapidTrigger {
            rapid_trigger: self.rapid_trigger,
            turbo: self.turbo,
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn load_profile(&mut self, profile: &Profile) {
        self.actuation.fill(profile.actuation);
        for (name, mm) in &profile.keys {
            if let Some(idx) = layout::index_of(name) {
                self.actuation[idx] = *mm;
            }
        }
        self.rapid_trigger = profile.rapid_trigger;
        self.turbo = profile.turbo;
        self.global_mm = profile.actuation;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn build_profile(&self) -> Profile {
        let mut profile = Profile {
            actuation: self.global_mm,
            rapid_trigger: self.rapid_trigger,
            turbo: self.turbo,
            keys: Default::default(),
        };
        for i in 0..TOTAL_KEYS {
            if let Some(name) = layout::name_of(i) {
                profile.keys.insert(name.to_string(), self.actuation[i]);
            }
        }
        profile
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.top_bar(ui);
        self.side_panel(ui);
        self.key_grid(ui);
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top")
            .frame(
                egui::Frame::NONE
                    .fill(BG)
                    .inner_margin(Margin::symmetric(14, 10)),
            )
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("fawnd").size(20.0).strong().color(TEXT));
                    ui.label(RichText::new("DrunkDeer A75").color(TEXT_DIM));
                    ui.add_space(8.0);
                    status_pill(ui, self.connected);
                    if let Some(info) = &self.info {
                        let mut label = format!("{} · fw {}", info.model, info.firmware);
                        if let Some(profile) = &info.active_profile {
                            label.push_str(&format!(" · {profile}"));
                        }
                        ui.label(RichText::new(label).color(TEXT_DIM));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reconnect").clicked() {
                            self.worker.send(Command::Reconnect);
                        }
                        if ui.button("Refresh").clicked() {
                            self.worker.send(Command::Refresh);
                        }
                        ui.add_space(6.0);
                        let live_label = RichText::new("◉ Live key depth").color(if self.live {
                            ACCENT
                        } else {
                            TEXT_DIM
                        });
                        if ui.selectable_label(self.live, live_label).clicked() {
                            self.live = !self.live;
                            self.worker.send(Command::SetLiveDepth(self.live));
                            if !self.live {
                                self.depths = [0; TOTAL_KEYS];
                            }
                        }
                    });
                });
            });

        egui::Panel::bottom("status")
            .frame(
                egui::Frame::NONE
                    .fill(CARD)
                    .inner_margin(Margin::symmetric(14, 6)),
            )
            .show_inside(ui, |ui| {
                ui.label(RichText::new(&self.status).color(TEXT_DIM));
            });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("controls")
            .resizable(false)
            .default_size(280.0)
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::same(12)))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.spacing_mut().slider_width = 150.0;

                    card(ui, "Global actuation", |ui| {
                        ui.add(slider(&mut self.global_mm));
                        if accent_button(ui, "Set all keys", false).clicked() {
                            self.actuation.fill(self.global_mm);
                        }
                    });

                    card(ui, "Selected keys", |ui| {
                        let n = self.selected_indices().len();
                        ui.label(RichText::new(format!("{n} key(s) selected")).color(TEXT_DIM));
                        ui.add(slider(&mut self.brush_mm));
                        if accent_button(ui, "Apply to selection", false).clicked() {
                            for i in self.selected_indices() {
                                self.actuation[i] = self.brush_mm;
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.button("All").clicked() {
                                for i in 0..TOTAL_KEYS {
                                    self.selected[i] = layout::name_of(i).is_some();
                                }
                            }
                            if ui.button("None").clicked() {
                                self.selected = [false; TOTAL_KEYS];
                            }
                            if ui.button("WASD").clicked() {
                                self.selected = [false; TOTAL_KEYS];
                                for &i in WASD_KEYS {
                                    self.selected[i] = true;
                                }
                            }
                        });
                    });

                    card(ui, "Rapid trigger", |ui| {
                        ui.checkbox(&mut self.rapid_trigger, "Rapid trigger");
                        ui.checkbox(&mut self.turbo, "Turbo (snap-tap)");
                        if ui.button("Apply toggles").clicked() {
                            self.worker.send(Command::SetRapidTrigger {
                                rapid_trigger: self.rapid_trigger,
                                turbo: self.turbo,
                            });
                        }
                    });

                    card(ui, "Lighting", |ui| {
                        egui::ComboBox::from_label("Effect")
                            .selected_text(sequence_name(self.led_sequence))
                            .show_ui(ui, |ui| {
                                for (seq, name) in SEQUENCES {
                                    ui.selectable_value(&mut self.led_sequence, *seq, *name);
                                }
                            });
                        ui.add(egui::Slider::new(&mut self.led_speed, 0..=10).text("speed"));
                        ui.add(
                            egui::Slider::new(&mut self.led_brightness, 0..=10).text("brightness"),
                        );
                        ui.add(egui::Slider::new(&mut self.led_color, 0..=255).text("color"));
                        if ui.button("Apply lighting").clicked() {
                            self.worker.send(Command::SetLed {
                                direction: 0,
                                sequence: self.led_sequence,
                                speed: self.led_speed,
                                brightness: self.led_brightness,
                                rgb: self.led_color,
                            });
                        }
                    });

                    self.profile_card(ui);

                    ui.add_space(4.0);
                    if accent_button(ui, "⏏  Apply to keyboard", true).clicked() {
                        self.apply_to_keyboard();
                    }
                    if ui.button("Restore defaults").clicked() {
                        self.worker.send(Command::Reset);
                        self.actuation.fill(2.0);
                        self.rapid_trigger = false;
                        self.turbo = false;
                    }
                });
            });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn profile_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Profile", |ui| {
            ui.horizontal(|ui| {
                if ui.button("Load...").clicked() {
                    if let Some(path) = pick_open() {
                        match Profile::load(&path) {
                            Ok(p) => {
                                self.load_profile(&p);
                                self.status = format!("Loaded {}", path.display());
                            }
                            Err(e) => self.status = format!("Load failed: {e}"),
                        }
                    }
                }
                if ui.button("Save...").clicked() {
                    if let Some(path) = pick_save() {
                        match self.build_profile().save(&path) {
                            Ok(()) => self.status = format!("Saved {}", path.display()),
                            Err(e) => self.status = format!("Save failed: {e}"),
                        }
                    }
                }
            });
        });
    }

    #[cfg(target_arch = "wasm32")]
    fn profile_card(&mut self, ui: &mut egui::Ui) {
        card(ui, "Profile", |ui| {
            ui.label(
                RichText::new("Profile file load/save is available in the native app.")
                    .color(TEXT_DIM),
            );
        });
    }

    fn key_grid(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BG).inner_margin(Margin::same(16)))
            .show_inside(ui, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        // Main block.
                        ui.vertical(|ui| {
                            for row in MAIN_ROWS {
                                self.visual_row(ui, row);
                            }
                        });
                        ui.add_space(22.0);
                        // Nav column + arrow cluster.
                        ui.vertical(|ui| {
                            for row in RIGHT_ROWS {
                                self.visual_row(ui, row);
                            }
                        });
                    });

                    ui.add_space(16.0);
                    legend(ui, self.live);
                });
            });
    }

    fn visual_row(&mut self, ui: &mut egui::Ui, row: &[Vk]) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = KEY_GAP;
            for &(name, width) in row {
                self.visual_key(ui, name, width);
            }
        });
    }

    fn visual_key(&mut self, ui: &mut egui::Ui, name: &str, width: f32) {
        let size = vec2(KEY_U * width, KEY_H);
        // Gaps, or keys this device doesn't have, just reserve space.
        if name.is_empty() {
            ui.allocate_space(size);
            return;
        }
        let Some(idx) = layout::index_of(name) else {
            ui.allocate_space(size);
            return;
        };

        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }

        let mm = self.actuation[idx];
        let selected = self.selected[idx];
        let p = ui.painter().clone();

        if self.live {
            self.paint_live_cap(&p, rect, idx, name, mm, selected, resp.hovered());
        } else {
            self.paint_config_cap(&p, rect, name, mm, selected, resp.hovered());
        }

        if resp.clicked() {
            self.selected[idx] = !selected;
        }
        let hover = if self.live {
            format!("{name} · {:.1} mm pressed", self.depths[idx] as f32 / 10.0)
        } else {
            format!("{name} · {mm:.1} mm")
        };
        resp.on_hover_text(hover);
    }

    /// Configuration view: heat tint + actuation depth bar.
    fn paint_config_cap(
        &self,
        p: &egui::Painter,
        rect: Rect,
        name: &str,
        mm: f32,
        selected: bool,
        hovered: bool,
    ) {
        let t = ((mm - MM_MIN) / (MM_MAX - MM_MIN)).clamp(0.0, 1.0);
        let heat = actuation_color(mm);
        let mut fill = lerp_color(CAP_BG, heat, 0.30);
        if hovered {
            fill = lerp_color(fill, Color32::WHITE, 0.10);
        }
        if selected {
            fill = lerp_color(fill, ACCENT, 0.25);
        }

        let r = CornerRadius::same(6);
        p.rect_filled(rect, r, fill);
        let border = if selected {
            Stroke::new(2.0, ACCENT)
        } else {
            Stroke::new(1.0, Color32::from_black_alpha(70))
        };
        p.rect_stroke(rect, r, border, StrokeKind::Inside);

        p.text(
            pos2(rect.center().x, rect.top() + 16.0),
            Align2::CENTER_CENTER,
            short_label(name),
            FontId::proportional(10.5),
            TEXT,
        );
        p.text(
            pos2(rect.center().x, rect.bottom() - 14.0),
            Align2::CENTER_CENTER,
            format!("{mm:.1}"),
            FontId::proportional(9.0),
            TEXT_DIM,
        );

        let pad = 6.0;
        let track = Rect::from_min_max(
            pos2(rect.left() + pad, rect.bottom() - 7.0),
            pos2(rect.right() - pad, rect.bottom() - 4.0),
        );
        p.rect_filled(track, CornerRadius::same(2), Color32::from_black_alpha(110));
        let filled = Rect::from_min_max(
            track.min,
            pos2(track.left() + track.width() * t, track.bottom()),
        );
        p.rect_filled(filled, CornerRadius::same(2), heat);
    }

    /// Live view: cap fills from the bottom by current key travel; turns green
    /// once it crosses the configured actuation point.
    fn paint_live_cap(
        &self,
        p: &egui::Painter,
        rect: Rect,
        idx: usize,
        name: &str,
        mm: f32,
        selected: bool,
        hovered: bool,
    ) {
        let raw = self.depths[idx];
        let frac = (raw as f32 / DEPTH_MAX_RAW as f32).clamp(0.0, 1.0);
        let actuated = raw as f32 >= mm * 10.0 && raw > 0;

        let r = CornerRadius::same(6);
        let base = if hovered {
            lerp_color(CAP_BG, Color32::WHITE, 0.08)
        } else {
            CAP_BG
        };
        p.rect_filled(rect, r, base);

        // Rising fill proportional to travel.
        if frac > 0.01 {
            let h = (rect.height() - 2.0) * frac;
            let fill_rect = Rect::from_min_max(
                pos2(rect.left() + 1.0, rect.bottom() - 1.0 - h),
                pos2(rect.right() - 1.0, rect.bottom() - 1.0),
            );
            let col = if actuated { OK } else { ACCENT };
            p.rect_filled(fill_rect, r, col.linear_multiply(0.6));
        }

        let border = if actuated {
            Stroke::new(2.0, OK)
        } else if selected {
            Stroke::new(2.0, ACCENT)
        } else {
            Stroke::new(1.0, Color32::from_black_alpha(70))
        };
        p.rect_stroke(rect, r, border, StrokeKind::Inside);

        p.text(
            pos2(rect.center().x, rect.top() + 15.0),
            Align2::CENTER_CENTER,
            short_label(name),
            FontId::proportional(10.5),
            TEXT,
        );
        if raw > 0 {
            p.text(
                pos2(rect.center().x, rect.bottom() - 12.0),
                Align2::CENTER_CENTER,
                format!("{:.1}", raw as f32 / 10.0),
                FontId::proportional(9.5),
                Color32::WHITE,
            );
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────────

fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = BG;
    v.window_fill = CARD;
    v.extreme_bg_color = Color32::from_rgb(0x12, 0x14, 0x18);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT.linear_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.inactive.bg_fill = CAP_BG;
    v.widgets.inactive.weak_bg_fill = CAP_BG;
    v.widgets.hovered.bg_fill = lerp_color(CAP_BG, Color32::WHITE, 0.10);
    v.widgets.hovered.weak_bg_fill = lerp_color(CAP_BG, Color32::WHITE, 0.10);

    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(10.0, 6.0);
    ctx.set_global_style(style);
}

/// A titled card container for the side panel.
fn card(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).strong().color(TEXT));
            ui.add_space(6.0);
            contents(ui);
        });
    ui.add_space(10.0);
}

fn slider(value: &mut f32) -> egui::Slider<'_> {
    egui::Slider::new(value, MM_MIN..=MM_MAX)
        .suffix(" mm")
        .step_by(0.1)
        .fixed_decimals(1)
}

fn accent_button(ui: &mut egui::Ui, text: &str, primary: bool) -> egui::Response {
    let fill = if primary { ACCENT } else { CAP_BG };
    let label = if primary {
        RichText::new(text).strong().color(Color32::WHITE)
    } else {
        RichText::new(text).color(TEXT)
    };
    ui.add_sized(
        [ui.available_width(), 30.0],
        egui::Button::new(label).fill(fill),
    )
}

fn status_pill(ui: &mut egui::Ui, connected: bool) {
    let (color, text) = if connected {
        (OK, "Connected")
    } else {
        (BAD, "Offline")
    };
    egui::Frame::NONE
        .fill(color.linear_multiply(0.18))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(format!("● {text}")).color(color).small());
        });
}

/// A legend explaining the current cap colours.
fn legend(ui: &mut egui::Ui, live: bool) {
    ui.horizontal(|ui| {
        if live {
            swatch(ui, ACCENT.linear_multiply(0.6));
            ui.label(RichText::new("travel").color(TEXT_DIM).small());
            ui.add_space(10.0);
            swatch(ui, OK);
            ui.label(
                RichText::new("past actuation point — press keys to see live depth")
                    .color(TEXT_DIM)
                    .small(),
            );
            return;
        }

        ui.label(RichText::new("0.2 mm · sensitive").color(TEXT_DIM).small());
        let (rect, _) = ui.allocate_exact_size(vec2(180.0, 12.0), Sense::hover());
        let p = ui.painter();
        let steps = 48;
        for i in 0..steps {
            let t0 = i as f32 / steps as f32;
            let x0 = rect.left() + rect.width() * t0;
            let x1 = rect.left() + rect.width() * (i + 1) as f32 / steps as f32;
            let mm = MM_MIN + t0 * (MM_MAX - MM_MIN);
            let seg = Rect::from_min_max(pos2(x0, rect.top()), pos2(x1, rect.bottom()));
            p.rect_filled(seg, CornerRadius::ZERO, actuation_color(mm));
        }
        ui.label(RichText::new("3.8 mm · deep").color(TEXT_DIM).small());
    });
}

/// A small colour swatch used in the legend.
fn swatch(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 12.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(3), color);
}

/// Map actuation distance to a heat colour: shallow = warm orange, deep = cool blue.
fn actuation_color(mm: f32) -> Color32 {
    let t = ((mm - MM_MIN) / (MM_MAX - MM_MIN)).clamp(0.0, 1.0);
    lerp_color(
        Color32::from_rgb(0xff, 0x8a, 0x5b),
        Color32::from_rgb(0x5b, 0x8c, 0xff),
        t,
    )
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Friendlier on-cap labels for the device's terse key names.
fn short_label(name: &str) -> &str {
    match name {
        "TILDE" => "~",
        "MINUS" => "-",
        "PLUS" => "=",
        "BACK" => "Bksp",
        "TAB" => "Tab",
        "BRKTS_L" => "[",
        "BRKTS_R" => "]",
        "SLASH_K29" => "\\",
        "CAPS" => "Caps",
        "COLON" => ";",
        "QOTATN" => "'",
        "RETURN" => "Enter",
        "SHF_L" | "SHF_R" => "Shift",
        "EUR_K45" => "\\",
        "COMMA" => ",",
        "PERIOD" => ".",
        "SLASH" => "/",
        "CTRL_L" | "CTRL_R" => "Ctrl",
        "WIN_L" => "Win",
        "ALT_L" | "ALT_R" => "Alt",
        "SPACE" => "Space",
        "APP" => "Menu",
        "ARR_UP" => "Up",
        "ARR_DW" => "Dn",
        "ARR_L" => "Lt",
        "ARR_R" => "Rt",
        "HOME" => "Home",
        "PGUP" => "PgUp",
        "PGDN" => "PgDn",
        "END" => "End",
        "DEL" => "Del",
        other => other,
    }
}

fn sequence_name(seq: LedSequence) -> &'static str {
    SEQUENCES
        .iter()
        .find(|(s, _)| *s == seq)
        .map(|(_, n)| *n)
        .unwrap_or("Custom")
}

#[cfg(not(target_arch = "wasm32"))]
fn pick_open() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("TOML profile", &["toml"])
        .pick_file()
}

#[cfg(not(target_arch = "wasm32"))]
fn pick_save() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("TOML profile", &["toml"])
        .set_file_name("profile.toml")
        .save_file()
}
