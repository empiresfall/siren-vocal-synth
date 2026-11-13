use crate::{CreatureCallMode, EngineChoice, HarmonyMode, SharedUiState, SirenParams, VoiceMode};
use nih_plug::prelude::{Editor, EnumParam, Param, ParamSetter};
use nih_plug_egui::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind,
    Vec2,
};
use nih_plug_egui::widgets::ParamSlider;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::Arc;

const BG_DARK: Color32 = Color32::from_rgb(9, 11, 16);
const PANEL_BG: Color32 = Color32::from_rgb(17, 21, 30);
const PANEL_BORDER: Color32 = Color32::from_rgb(36, 44, 62);
const ACCENT_CORAL: Color32 = Color32::from_rgb(255, 77, 109);
const ACCENT_CYAN: Color32 = Color32::from_rgb(56, 189, 248);
const ACCENT_GOLD: Color32 = Color32::from_rgb(245, 158, 11);
const TEXT_MAIN: Color32 = Color32::from_rgb(236, 240, 248);
const TEXT_DIM: Color32 = Color32::from_rgb(138, 149, 172);

struct EditorState {
    params: Arc<SirenParams>,
    ui_state: Arc<SharedUiState>,
    xy_dragging: bool,
}

pub(crate) fn create(
    params: Arc<SirenParams>,
    ui_state: Arc<SharedUiState>,
    egui_state: Arc<EguiState>,
) -> Option<Box<dyn Editor>> {
    let state = EditorState {
        params,
        ui_state,
        xy_dragging: false,
    };

    create_egui_editor(
        egui_state,
        state,
        |ctx, _state| {
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = BG_DARK;
            visuals.window_fill = PANEL_BG;
            visuals.widgets.noninteractive.bg_fill = PANEL_BG;
            visuals.widgets.inactive.bg_fill = Color32::from_rgb(25, 31, 44);
            visuals.widgets.hovered.bg_fill = Color32::from_rgb(38, 47, 66);
            visuals.widgets.active.bg_fill = ACCENT_CORAL;
            visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(255, 77, 109, 90);
            visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT_CORAL);
            ctx.set_visuals(visuals);
        },
        |ctx, setter, state| {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            let snap = state.ui_state.snapshot();

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(BG_DARK).inner_margin(12.0_f32))
                .show(ctx, |ui| {
                    draw_header(ui, setter, &state.params, &snap);
                    ui.add_space(8.0_f32);

                    ui.columns(3, |cols| {
                        // Left Column: Voice / Body / Creature / Harmony
                        draw_panel(&mut cols[0], "VOICE • ANATOMY • CHOIR", |ui| {
                            draw_enum_combo(ui, setter, "Voice / Mode", &state.params.voice_mode);
                            ui.add_space(4.0_f32);

                            // Quick preset strip
                            ui.horizontal_wrapped(|ui| {
                                for (label, mode) in [
                                    ("Female", VoiceMode::HumanFemale),
                                    ("Male", VoiceMode::HumanMale),
                                    ("Child", VoiceMode::HumanChild),
                                    ("Beatbox", VoiceMode::BeatboxKit),
                                    ("Songbird", VoiceMode::Songbird),
                                    ("Whale", VoiceMode::HumpbackWhale),
                                ] {
                                    let active = state.params.voice_mode.value() == mode;
                                    let btn = egui::Button::new(
                                        RichText::new(label).size(11.0_f32).color(if active {
                                            Color32::WHITE
                                        } else {
                                            TEXT_DIM
                                        }),
                                    )
                                    .fill(if active {
                                        ACCENT_CORAL
                                    } else {
                                        Color32::from_rgb(24, 30, 42)
                                    })
                                    .corner_radius(CornerRadius::same(4));
                                    if ui.add(btn).clicked() {
                                        set_enum(setter, &state.params.voice_mode, mode);
                                    }
                                }
                            });

                            ui.add_space(6.0_f32);
                            let vmode = state.params.voice_mode.value();
                            if vmode == VoiceMode::HumanMorph {
                                param_row(ui, setter, "Female Axis", &state.params.body_female);
                                param_row(ui, setter, "Child Axis", &state.params.body_child);
                            } else if vmode.creature_index().is_some() {
                                draw_enum_combo(
                                    ui,
                                    setter,
                                    "Creature Trigger",
                                    &state.params.creature_call,
                                );
                                ui.label(
                                    RichText::new(
                                        "MIDI keys scale creature pitch; Call 1–4 fires authored vocalizations.",
                                    )
                                    .size(10.5_f32)
                                    .color(TEXT_DIM),
                                );
                            } else if vmode == VoiceMode::BeatboxKit {
                                ui.label(
                                    RichText::new(
                                        "GM Drum Pads: C1 Kick • C#1 Rim • D1 Snare • D#1 Pop • E1 Inward K • F#1 Hat • A#1 Open Hat • C2+ Hum Bass",
                                    )
                                    .size(10.5_f32)
                                    .color(ACCENT_GOLD),
                                );
                            }

                            ui.separator();
                            ui.label(
                                RichText::new("HARMONY ENSEMBLE (SHARED TRACT)")
                                    .size(10.5_f32)
                                    .strong()
                                    .color(ACCENT_GOLD),
                            );
                            draw_enum_combo(ui, setter, "Choir Voicing", &state.params.harmony);
                            param_row(ui, setter, "Golden 1/φ", &state.params.golden_voice);

                            ui.separator();
                            ui.label(
                                RichText::new("GLOTTAL SOURCE")
                                    .size(10.5_f32)
                                    .strong()
                                    .color(ACCENT_CYAN),
                            );
                            param_row(ui, setter, "Pressure", &state.params.pressure);
                            param_row(ui, setter, "Breathiness", &state.params.breathiness);
                            param_row(ui, setter, "Tension", &state.params.tension);
                            param_row(ui, setter, "Brightness", &state.params.brightness);
                            param_row(ui, setter, "Vocal Fry", &state.params.fry);
                            param_row(ui, setter, "Roughness", &state.params.roughness);
                        });

                        // Center Column: 2D Vowel XY Pad + Live Vocal Tract & Scope
                        draw_panel(&mut cols[1], "VOWEL ARTICULATION & LIVE AIRWAY", |ui| {
                            draw_vowel_xy_pad(ui, setter, state);
                            ui.add_space(6.0_f32);
                            draw_airway_and_scope(ui, &snap);
                            ui.add_space(6.0_f32);
                            param_row(ui, setter, "Nasal Port", &state.params.nasal);
                            param_row(ui, setter, "Twang", &state.params.twang);
                            param_row(ui, setter, "Larynx Ht", &state.params.larynx);
                        });

                        // Right Column: Pitch / Vibrato + Studio FX Rack
                        draw_panel(&mut cols[2], "EXPRESSION & STUDIO FX RACK", |ui| {
                            ui.horizontal(|ui| {
                                draw_bool_pill(
                                    ui,
                                    setter,
                                    "VOICED FOLDS",
                                    "WHISPER MODE",
                                    &state.params.voiced,
                                    ACCENT_CYAN,
                                );
                            });
                            ui.add_space(4.0_f32);
                            param_row(ui, setter, "Vibrato Amt", &state.params.vibrato_depth);
                            param_row(ui, setter, "Vibrato Hz", &state.params.vibrato_rate);
                            param_row(ui, setter, "Glide Time", &state.params.glide);

                            ui.separator();
                            ui.label(
                                RichText::new("CHIMERA STEREO FX")
                                    .size(10.5_f32)
                                    .strong()
                                    .color(ACCENT_CORAL),
                            );
                            param_row(ui, setter, "Punch Comp", &state.params.fx_punch);
                            param_row(ui, setter, "Phone Lo-Fi", &state.params.fx_phone);
                            param_row(ui, setter, "Delay Mix", &state.params.fx_delay_mix);
                            param_row(ui, setter, "Delay Time", &state.params.fx_delay_time);
                            param_row(ui, setter, "Golden Verb", &state.params.fx_golden_mix);
                            param_row(ui, setter, "Verb Decay", &state.params.fx_golden_decay);

                            ui.separator();
                            param_row(ui, setter, "Output Gain", &state.params.output_gain);
                        });
                    });
                });
        },
    )
}

fn draw_header(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &SirenParams,
    snap: &crate::UiSnapshot,
) {
    egui::Frame::NONE
        .fill(PANEL_BG)
        .stroke(Stroke::new(1.0_f32, PANEL_BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("SIREN")
                                .size(20.0_f32)
                                .strong()
                                .color(TEXT_MAIN),
                        );
                        ui.label(
                            RichText::new("CHIMERA VOCAL SYNTH")
                                .size(11.0_f32)
                                .strong()
                                .color(ACCENT_CORAL),
                        );
                    });
                    ui.label(
                        RichText::new(
                            "Physical-Modelling Articulatory Airway • Bestiary • Beatbox Instrument",
                        )
                        .size(10.5_f32)
                        .color(TEXT_DIM),
                    );
                });

                ui.add_space(12.0_f32);

                // Engine Mode Pills
                for (label, choice) in [
                    ("AIRWAY", EngineChoice::Airway),
                    ("WAVEGUIDE", EngineChoice::Waveguide),
                    ("FORMANT", EngineChoice::Formant),
                    ("NEURAL", EngineChoice::Neural),
                ] {
                    let selected = params.engine_kind.value() == choice;
                    let btn = egui::Button::new(
                        RichText::new(label)
                            .size(10.5_f32)
                            .strong()
                            .color(if selected { BG_DARK } else { TEXT_DIM }),
                    )
                    .fill(if selected {
                        ACCENT_CYAN
                    } else {
                        Color32::from_rgb(24, 30, 44)
                    })
                    .corner_radius(CornerRadius::same(4));
                    if ui.add(btn).clicked() {
                        set_enum(setter, &params.engine_kind, choice);
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Stereo meter
                    let meter_size = Vec2::new(72.0_f32, 20.0_f32);
                    let (rect, _) = ui.allocate_exact_size(meter_size, Sense::hover());
                    let p = ui.painter_at(rect);
                    p.rect_filled(rect, 4.0_f32, Color32::from_rgb(10, 12, 18));
                    let l_w = (snap.rms_l * 2.4_f32).clamp(0.0_f32, 1.0_f32) * (rect.width() - 4.0_f32);
                    let r_w = (snap.rms_r * 2.4_f32).clamp(0.0_f32, 1.0_f32) * (rect.width() - 4.0_f32);
                    p.rect_filled(
                        Rect::from_min_size(rect.min + Vec2::new(2.0_f32, 3.0_f32), Vec2::new(l_w, 6.0_f32)),
                        2.0_f32,
                        ACCENT_CYAN,
                    );
                    p.rect_filled(
                        Rect::from_min_size(rect.min + Vec2::new(2.0_f32, 11.0_f32), Vec2::new(r_w, 6.0_f32)),
                        2.0_f32,
                        ACCENT_CORAL,
                    );

                    ui.add_space(8.0_f32);

                    // Audition Drone Toggle
                    let drone_on = params.drone.value();
                    let drone_btn = egui::Button::new(
                        RichText::new(if drone_on { "DRONE ON" } else { "DRONE" })
                            .size(11.0_f32)
                            .strong()
                            .color(if drone_on { Color32::WHITE } else { TEXT_MAIN }),
                    )
                    .fill(if drone_on {
                        ACCENT_CORAL
                    } else {
                        Color32::from_rgb(30, 36, 52)
                    })
                    .corner_radius(CornerRadius::same(5));
                    if ui.add(drone_btn).clicked() {
                        setter.begin_set_parameter(&params.drone);
                        setter.set_parameter(&params.drone, !drone_on);
                        setter.end_set_parameter(&params.drone);
                    }

                    ui.add_space(8.0_f32);
                    let note_txt = if snap.current_note >= 0 {
                        format!(
                            "{} • {:.1} Hz",
                            midi_note_name(snap.current_note as u8),
                            snap.current_f0
                        )
                    } else if snap.gate > 0.02_f32 {
                        format!("{:.1} Hz", snap.current_f0)
                    } else {
                        "MIDI READY".to_string()
                    };
                    ui.label(
                        RichText::new(note_txt)
                            .monospace()
                            .size(11.5_f32)
                            .color(ACCENT_GOLD),
                    );
                });
            });
        });
}

fn draw_panel(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE
        .fill(PANEL_BG)
        .stroke(Stroke::new(1.0_f32, PANEL_BORDER))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_height(462.0_f32);
            ui.label(
                RichText::new(title)
                    .size(11.0_f32)
                    .strong()
                    .color(TEXT_DIM),
            );
            ui.add_space(6.0_f32);
            add_contents(ui);
        });
}

fn draw_vowel_xy_pad(ui: &mut egui::Ui, setter: &ParamSetter, state: &mut EditorState) {
    let w = ui.available_width();
    let pad_size = Vec2::new(w, 205.0_f32);
    let (rect, response) = ui.allocate_exact_size(pad_size, Sense::click_and_drag());

    if response.drag_started() || response.clicked() {
        setter.begin_set_parameter(&state.params.vowel_x);
        setter.begin_set_parameter(&state.params.vowel_y);
        state.xy_dragging = true;
    }
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
        let nx = ((pos.x - rect.left()) / rect.width()).clamp(0.0_f32, 1.0_f32);
        let ny = ((pos.y - rect.top()) / rect.height()).clamp(0.0_f32, 1.0_f32);
        setter.set_parameter_normalized(&state.params.vowel_x, nx);
        setter.set_parameter_normalized(&state.params.vowel_y, ny);
        }
    }
    if state.xy_dragging && (response.drag_stopped() || !ui.input(|i| i.pointer.any_down())) {
        setter.end_set_parameter(&state.params.vowel_x);
        setter.end_set_parameter(&state.params.vowel_y);
        state.xy_dragging = false;
    }

    let p = ui.painter_at(rect);
    p.rect_filled(rect, 6.0_f32, Color32::from_rgb(11, 14, 22));
    p.rect_stroke(
        rect,
        6.0_f32,
        Stroke::new(1.0_f32, Color32::from_rgb(42, 52, 74)),
        StrokeKind::Inside,
    );

    // Subtle 4x4 grid
    for i in 1..4 {
        let t = i as f32 / 4.0_f32;
        let gx = egui::lerp(rect.left()..=rect.right(), t);
        let gy = egui::lerp(rect.top()..=rect.bottom(), t);
        p.line_segment(
            [Pos2::new(gx, rect.top()), Pos2::new(gx, rect.bottom())],
            Stroke::new(1.0_f32, Color32::from_rgb(20, 26, 38)),
        );
        p.line_segment(
            [Pos2::new(rect.left(), gy), Pos2::new(rect.right(), gy)],
            Stroke::new(1.0_f32, Color32::from_rgb(20, 26, 38)),
        );
    }

    // IPA Corner Labels
    let font = FontId::proportional(13.0_f32);
    p.text(
        rect.left_top() + Vec2::new(10.0_f32, 8.0_f32),
        Align2::LEFT_TOP,
        "/i/ EE",
        font.clone(),
        ACCENT_CYAN,
    );
    p.text(
        rect.right_top() + Vec2::new(-10.0_f32, 8.0_f32),
        Align2::RIGHT_TOP,
        "/u/ OO",
        font.clone(),
        ACCENT_CYAN,
    );
    p.text(
        rect.left_bottom() + Vec2::new(10.0_f32, -8.0_f32),
        Align2::LEFT_BOTTOM,
        "/æ/ AE",
        font.clone(),
        ACCENT_GOLD,
    );
    p.text(
        rect.right_bottom() + Vec2::new(-10.0_f32, -8.0_f32),
        Align2::RIGHT_BOTTOM,
        "/ɑ/ AH",
        font,
        ACCENT_CORAL,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "/ə/",
        FontId::proportional(12.0_f32),
        Color32::from_rgba_unmultiplied(236, 240, 248, 90),
    );

    let vx = state.params.vowel_x.modulated_normalized_value();
    let vy = state.params.vowel_y.modulated_normalized_value();
    let cursor = Pos2::new(
        egui::lerp(rect.left() + 8.0_f32..=rect.right() - 8.0_f32, vx),
        egui::lerp(rect.top() + 8.0_f32..=rect.bottom() - 8.0_f32, vy),
    );

    p.line_segment(
        [Pos2::new(cursor.x, rect.top()), Pos2::new(cursor.x, rect.bottom())],
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 77, 109, 70)),
    );
    p.line_segment(
        [Pos2::new(rect.left(), cursor.y), Pos2::new(rect.right(), cursor.y)],
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 77, 109, 70)),
    );
    p.circle_filled(
        cursor,
        12.0_f32,
        Color32::from_rgba_unmultiplied(255, 77, 109, 55),
    );
    p.circle_filled(cursor, 5.5_f32, ACCENT_CORAL);
    p.circle_stroke(cursor, 8.5_f32, Stroke::new(1.5_f32, Color32::WHITE));
}

fn draw_airway_and_scope(ui: &mut egui::Ui, snap: &crate::UiSnapshot) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 128.0_f32), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 6.0_f32, Color32::from_rgb(11, 14, 22));
    p.rect_stroke(
        rect,
        6.0_f32,
        Stroke::new(1.0_f32, Color32::from_rgb(36, 44, 62)),
        StrokeKind::Inside,
    );

    let split_y = rect.top() + rect.height() * 0.54_f32;
    let tract_rect = Rect::from_min_max(
        rect.min + Vec2::new(6.0_f32, 6.0_f32),
        Pos2::new(rect.right() - 6.0_f32, split_y - 3.0_f32),
    );
    let scope_rect = Rect::from_min_max(
        Pos2::new(rect.left() + 6.0_f32, split_y + 3.0_f32),
        rect.max - Vec2::new(6.0_f32, 6.0_f32),
    );

    // Top half: vocal tract cross-sectional area profile (glottis -> pharynx -> palate -> lips)
    p.text(
        tract_rect.left_top(),
        Align2::LEFT_TOP,
        "VOCAL TRACT AREA (GLOTTIS → LIPS)",
        FontId::proportional(9.5_f32),
        TEXT_DIM,
    );
    if snap.tract_len >= 2 {
        let n = snap.tract_len;
        let mid_y = tract_rect.center().y + 4.0_f32;
        let max_h = tract_rect.height() * 0.42_f32;
        let mut upper = Vec::with_capacity(n);
        let mut lower = Vec::with_capacity(n);
        for i in 0..n {
            let x = egui::lerp(
                tract_rect.left()..=tract_rect.right(),
                i as f32 / (n - 1) as f32,
            );
            let r = (snap.tract_areas[i].max(0.0_f32).sqrt() / 3.6_f32).clamp(0.04_f32, 1.0_f32) * max_h;
            upper.push(Pos2::new(x, mid_y - r));
            lower.push(Pos2::new(x, mid_y + r));
        }
        p.add(egui::Shape::line(upper, Stroke::new(1.8_f32, ACCENT_CYAN)));
        p.add(egui::Shape::line(lower, Stroke::new(1.8_f32, ACCENT_CYAN)));
    }

    // Divider
    p.line_segment(
        [
            Pos2::new(rect.left() + 4.0_f32, split_y),
            Pos2::new(rect.right() - 4.0_f32, split_y),
        ],
        Stroke::new(1.0_f32, Color32::from_rgb(26, 32, 46)),
    );

    // Bottom half: live output oscilloscope
    p.text(
        scope_rect.left_top(),
        Align2::LEFT_TOP,
        "RADIATED PRESSURE WAVE",
        FontId::proportional(9.5_f32),
        TEXT_DIM,
    );
    let mid_y = scope_rect.center().y + 3.0_f32;
    let amp = scope_rect.height() * 0.44_f32;
    let pts: Vec<Pos2> = snap
        .scope
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let x = egui::lerp(
                scope_rect.left()..=scope_rect.right(),
                i as f32 / (snap.scope.len() - 1) as f32,
            );
            Pos2::new(x, mid_y - (s * 1.6_f32).clamp(-1.0_f32, 1.0_f32) * amp)
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.5_f32, ACCENT_CORAL)));
}

fn param_row<P: Param>(ui: &mut egui::Ui, setter: &ParamSetter, label: &str, param: &P) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(78.0_f32, 18.0_f32),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(RichText::new(label).size(11.0_f32).color(TEXT_MAIN));
            },
        );
        let w = (ui.available_width() - 4.0_f32).max(80.0_f32);
        ui.add(ParamSlider::for_param(param, setter).with_width(w));
    });
}

fn draw_enum_combo<E: nih_plug::prelude::Enum + PartialEq + 'static>(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    label: &str,
    param: &EnumParam<E>,
) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(82.0_f32, 20.0_f32),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(RichText::new(label).size(11.0_f32).color(TEXT_MAIN));
            },
        );
        let current_idx = param.value().to_index();
        let variants = E::variants();
        let current_name = variants.get(current_idx).copied().unwrap_or("?");
        egui::ComboBox::from_id_salt(label)
            .width((ui.available_width() - 4.0_f32).max(90.0_f32))
            .selected_text(RichText::new(current_name).size(11.0_f32).color(TEXT_MAIN))
            .show_ui(ui, |ui| {
                for (idx, &name) in variants.iter().enumerate() {
                    if ui.selectable_label(idx == current_idx, name).clicked() {
                        set_enum(setter, param, E::from_index(idx));
                    }
                }
            });
    });
}

fn set_enum<E: nih_plug::prelude::Enum + PartialEq + 'static>(
    setter: &ParamSetter,
    param: &EnumParam<E>,
    val: E,
) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, val);
    setter.end_set_parameter(param);
}

fn draw_bool_pill(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    on_label: &str,
    off_label: &str,
    param: &nih_plug::prelude::BoolParam,
    color: Color32,
) {
    let val = param.value();
    let btn = egui::Button::new(
        RichText::new(if val { on_label } else { off_label })
            .size(10.5_f32)
            .strong()
            .color(if val { BG_DARK } else { TEXT_MAIN }),
    )
    .fill(if val {
        color
    } else {
        Color32::from_rgb(28, 35, 50)
    })
    .corner_radius(CornerRadius::same(4));
    if ui.add_sized([ui.available_width(), 22.0_f32], btn).clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !val);
        setter.end_set_parameter(param);
    }
}

fn midi_note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (note as i32 / 12) - 1;
    format!("{}{}", NAMES[(note % 12) as usize], octave)
}

#[allow(dead_code)]
const _: fn(CreatureCallMode) = |_| {};
const _: fn(HarmonyMode) = |_| {};
