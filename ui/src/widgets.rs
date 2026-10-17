//! Custom widgets: rotary knob, track overview and scrolling detail waveform.

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo};
use eframe::egui::Role;
use rbx_engine::deck::{BeatMap, DeckState, Grid};
use rbx_engine::session::DeckWave;
use rekordbox_formats::anlz::waveform::ColorColumn;

/// Hot cue colours A–H.
pub const HOT_CUE_COLORS: [Color32; 8] = [
    Color32::from_rgb(40, 226, 20),
    Color32::from_rgb(255, 18, 123),
    Color32::from_rgb(16, 177, 118),
    Color32::from_rgb(255, 160, 0),
    Color32::from_rgb(48, 90, 255),
    Color32::from_rgb(195, 44, 255),
    Color32::from_rgb(0, 210, 255),
    Color32::from_rgb(255, 230, 0),
];

pub const ACCENT: Color32 = Color32::from_rgb(0, 160, 255);
pub const CUE_COLOR: Color32 = Color32::from_rgb(255, 130, 0);
pub const LOOP_COLOR: Color32 = Color32::from_rgb(255, 200, 0);

/// Rotary knob: drag up/down, double-click resets. Value in `range`.
pub fn knob(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, default: f32, label: &str) -> Response {
    let size = Vec2::new(46.0, 46.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    if resp.dragged() {
        let d = -resp.drag_delta().y / 120.0 * (hi - lo);
        *value = (*value + d).clamp(lo, hi);
        resp.mark_changed();
    }
    if resp.double_clicked() {
        *value = default;
        resp.mark_changed();
    }
    let v = *value;
    resp.widget_info(|| WidgetInfo::slider(true, f64::from(v), label));
    let painter = ui.painter_at(rect);
    let c = Pos2::new(rect.center().x, rect.top() + 18.0);
    let r = 14.0;
    let visuals = ui.style().interact(&resp);
    painter.circle_filled(c, r, Color32::from_gray(38));
    painter.circle_stroke(c, r, Stroke::new(1.0, visuals.fg_stroke.color.gamma_multiply(0.5)));
    // 270° travel, 0 at 7:30 o'clock
    let frac = (v - lo) / (hi - lo).max(1e-6);
    let a0 = std::f32::consts::PI * 0.75;
    let angle = a0 + frac * std::f32::consts::PI * 1.5;
    let steps = 24;
    let mut pts = Vec::new();
    let ad = a0 + (default - lo) / (hi - lo).max(1e-6) * std::f32::consts::PI * 1.5;
    let (from, to) = if angle < ad { (angle, ad) } else { (ad, angle) };
    for i in 0..=steps {
        let a = from + (to - from) * i as f32 / steps as f32;
        pts.push(c + Vec2::angled(a) * (r + 3.0));
    }
    painter.add(egui::Shape::line(pts, Stroke::new(2.5, ACCENT)));
    painter.line_segment([c, c + Vec2::angled(angle) * (r - 2.0)], Stroke::new(2.5, Color32::WHITE));
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 5.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.0),
        ui.visuals().weak_text_color(),
    );
    resp.on_hover_text(format!("{label}: {v:.2} (Doppelklick: zurücksetzen)"))
}

/// Colour of a waveform column (c0 = bass → red, c1 = mids → green, c2 = highs → blue).
pub fn column_color(c: &ColorColumn) -> Color32 {
    let f = |v: u8| -> u8 { (40.0 + f32::from(v) / 7.0 * 215.0) as u8 };
    Color32::from_rgb(f(c.c0), f(c.c1), f(c.c2))
}

/// Beat positions (time, is downbeat) in `[t0, t1]`.
pub fn beats_in(map: Option<&BeatMap>, grid: Option<Grid>, t0: f64, t1: f64) -> Vec<(f64, bool)> {
    let mut out = Vec::new();
    if let Some(m) = map {
        let (b0, b1) = (m.beat_at(t0).ceil() as i64, m.beat_at(t1).floor() as i64);
        for b in b0..=b1.min(b0 + 2000) {
            out.push((m.time_of(b as f64), b.rem_euclid(4) == 0));
        }
    } else if let Some(g) = grid {
        let (b0, b1) = (g.beat_at(t0).ceil() as i64, g.beat_at(t1).floor() as i64);
        for b in b0..=b1.min(b0 + 2000) {
            out.push((g.first_beat_s + b as f64 * g.beat_s(), b.rem_euclid(4) == 0));
        }
    }
    out
}

/// Whole-track overview. Returns a seek fraction when clicked or dragged.
pub fn overview(ui: &mut Ui, wave: Option<&DeckWave>, s: &DeckState, height: f32, label: &str) -> Option<f64> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click_and_drag());
    resp.widget_info(|| WidgetInfo::labeled(Role::Slider, true, label));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_gray(18));
    let dur = s.duration_s.max(0.001);
    if let Some(w) = wave.filter(|w| !w.columns.is_empty()) {
        let n = w.columns.len();
        let width = rect.width().max(1.0) as usize;
        let played = (s.position_s / dur * width as f64) as usize;
        for x in 0..width {
            let (a, b) = (x * n / width, ((x + 1) * n / width).max(x * n / width + 1).min(n));
            let cols = &w.columns[a..b];
            let h = cols.iter().map(|c| c.height).max().unwrap_or(0);
            let mid = &cols[cols.len() / 2];
            let mut col = column_color(mid);
            if x < played {
                col = col.gamma_multiply(0.45);
            }
            let hh = f32::from(h) / 31.0 * (rect.height() * 0.5 - 2.0);
            let xx = rect.left() + x as f32 + 0.5;
            painter.line_segment([Pos2::new(xx, rect.center().y - hh), Pos2::new(xx, rect.center().y + hh)], Stroke::new(1.0, col));
        }
    }
    if s.loaded {
        let x_of = |t: f64| rect.left() + (t / dur) as f32 * rect.width();
        if let Some((a, b)) = s.loop_s {
            let col = if s.looping { LOOP_COLOR.gamma_multiply(0.35) } else { LOOP_COLOR.gamma_multiply(0.15) };
            painter.rect_filled(Rect::from_x_y_ranges(x_of(a)..=x_of(b).max(x_of(a) + 1.0), rect.y_range()), 0.0, col);
        }
        for (i, h) in s.hot_cue_s.iter().enumerate() {
            if let Some(t) = h {
                let x = x_of(*t);
                painter.vline(x, rect.top()..=rect.top() + 7.0, Stroke::new(3.0, HOT_CUE_COLORS[i]));
            }
        }
        let cx = x_of(s.cue_s);
        painter.add(egui::Shape::convex_polygon(
            vec![Pos2::new(cx - 4.0, rect.bottom()), Pos2::new(cx + 4.0, rect.bottom()), Pos2::new(cx, rect.bottom() - 6.0)],
            CUE_COLOR,
            Stroke::NONE,
        ));
        let px = x_of(s.position_s);
        painter.vline(px, rect.y_range(), Stroke::new(2.0, Color32::WHITE));
    }
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_gray(60)), StrokeKind::Inside);
    if s.loaded && (resp.clicked() || resp.dragged()) {
        if let Some(p) = resp.interact_pointer_pos() {
            return Some(f64::from((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
        }
    }
    None
}

/// What the user did on the detail waveform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DetailAction {
    /// Dragging started / ended (like touching the jog wheel).
    Touch(bool),
    /// Dragged by this many seconds (positive = forward in the track).
    Scrub(f64),
    /// Mouse wheel: nudge (pitch bend) by ticks.
    Nudge(i32),
}

/// Scrolling detail waveform centred on the play position, `zoom_s` seconds wide, with beat grid,
/// cue, hot cues and loop.
#[allow(clippy::too_many_arguments)]
pub fn detail(
    ui: &mut Ui,
    wave: Option<&DeckWave>,
    map: Option<&BeatMap>,
    grid: Option<Grid>,
    s: &DeckState,
    zoom_s: f64,
    height: f32,
    label: &str,
) -> Vec<DetailAction> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click_and_drag());
    resp.widget_info(|| WidgetInfo::labeled(Role::Canvas, true, label));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 3.0, Color32::from_gray(12));
    let mut actions = Vec::new();
    let secs_per_px = zoom_s / f64::from(rect.width().max(1.0));
    let center_x = rect.center().x;
    let t_of = |x: f32| s.position_s + f64::from(x - center_x) * secs_per_px;
    let x_of = |t: f64| center_x + ((t - s.position_s) / secs_per_px) as f32;
    if s.loaded {
        let (t0, t1) = (t_of(rect.left()), t_of(rect.right()));
        if let Some((a, b)) = s.loop_s {
            let col = if s.looping { LOOP_COLOR.gamma_multiply(0.3) } else { LOOP_COLOR.gamma_multiply(0.12) };
            painter.rect_filled(Rect::from_x_y_ranges(x_of(a)..=x_of(b), rect.y_range()), 0.0, col);
        }
        for (t, down) in beats_in(map, grid, t0.max(0.0), t1.min(s.duration_s)) {
            let x = x_of(t);
            let (col, w) = if down { (Color32::from_rgb(200, 40, 40), 1.5) } else { (Color32::from_gray(90), 1.0) };
            painter.vline(x, rect.y_range(), Stroke::new(w, col));
        }
        if let Some(w) = wave {
            let width = rect.width() as usize;
            for x in 0..width {
                let xx = rect.left() + x as f32 + 0.5;
                let Some(c) = w.at(t_of(xx)).filter(|_| t_of(xx) <= s.duration_s) else { continue };
                let hh = f32::from(c.height) / 31.0 * (rect.height() * 0.5 - 3.0);
                painter.line_segment(
                    [Pos2::new(xx, rect.center().y - hh), Pos2::new(xx, rect.center().y + hh)],
                    Stroke::new(1.0, column_color(c)),
                );
            }
        }
        for (i, h) in s.hot_cue_s.iter().enumerate() {
            if let Some(t) = h {
                let x = x_of(*t);
                painter.vline(x, rect.y_range(), Stroke::new(2.0, HOT_CUE_COLORS[i]));
                painter.text(
                    Pos2::new(x + 3.0, rect.top() + 2.0),
                    egui::Align2::LEFT_TOP,
                    (b'A' + i as u8) as char,
                    egui::FontId::monospace(11.0),
                    HOT_CUE_COLORS[i],
                );
            }
        }
        painter.vline(x_of(s.cue_s), rect.y_range(), Stroke::new(1.5, CUE_COLOR));
    }
    painter.vline(center_x, rect.y_range(), Stroke::new(2.0, Color32::WHITE));
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_gray(60)), StrokeKind::Inside);
    if s.loaded {
        if resp.drag_started() {
            actions.push(DetailAction::Touch(true));
        }
        if resp.dragged() {
            let dx = resp.drag_delta().x;
            if dx != 0.0 {
                actions.push(DetailAction::Scrub(-f64::from(dx) * secs_per_px));
            }
        }
        if resp.drag_stopped() {
            actions.push(DetailAction::Touch(false));
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                actions.push(DetailAction::Nudge((scroll / 4.0).round() as i32));
            }
        }
    }
    actions
}

/// Horizontal level meter 0..1 (clip above 1).
pub fn meter(ui: &mut Ui, level: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_gray(30));
    let l = level.clamp(0.0, 1.0);
    let col = if level >= 1.0 {
        Color32::RED
    } else if level > 0.7 {
        Color32::from_rgb(255, 190, 0)
    } else {
        Color32::from_rgb(40, 200, 80)
    };
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width() * l, rect.height())), 2.0, col);
}

/// Vertical level meter 0..1 (clip above 1).
pub fn meter_v(ui: &mut Ui, level: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(6.0, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_gray(30));
    let l = level.clamp(0.0, 1.0);
    let col = if level >= 1.0 {
        Color32::RED
    } else if level > 0.7 {
        Color32::from_rgb(255, 190, 0)
    } else {
        Color32::from_rgb(40, 200, 80)
    };
    let top = rect.bottom() - rect.height() * l;
    painter.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), top), rect.max), 2.0, col);
}
