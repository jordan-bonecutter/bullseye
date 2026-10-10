//! A selection and a small toolbar, drawn directly over the frozen desktop.
use crate::document::{Document, Mark, Tool};
use egui::{Align2, Color32, Context, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use image::RgbaImage;
use std::{collections::BTreeMap, path::PathBuf};

const ACCENT: Color32 = Color32::from_rgb(113, 175, 255);
const INK: Color32 = Color32::from_rgb(236, 239, 243);
const BAR: Color32 = Color32::from_rgb(30, 33, 38);

pub struct OverlayUi {
    pub ctx: Context,
    pub doc: Document,
    pub claimed: bool,
    pub done: bool,
    pub hits: BTreeMap<String, Rect>,
    pub toolbar_visible: bool,
    tool: Tool,
    color: Color32,
    width: f32,
    draft: Option<Mark>,
    moving: Option<(Rect, Pos2)>,
    text_anchor: Option<Pos2>,
    text: String,
    text_focus: bool,
    save_dialog: bool,
    destination: String,
    explicit_destination: bool,
    error: Option<String>,
}
impl OverlayUi {
    pub fn new(image: RgbaImage, destination: PathBuf, explicit_destination: bool) -> Self {
        let ctx = Context::default();
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = BAR;
        visuals.panel_fill = Color32::TRANSPARENT;
        visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        visuals.widgets.inactive.bg_stroke = Stroke::NONE;
        visuals.selection.bg_fill = ACCENT.gamma_multiply(0.25);
        ctx.set_visuals(visuals);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(3., 3.);
            s.spacing.button_padding = Vec2::new(6., 5.);
            s.interaction.tooltip_delay = 0.0;
        });
        Self {
            ctx,
            doc: Document::new(image),
            claimed: false,
            done: false,
            hits: BTreeMap::new(),
            toolbar_visible: false,
            tool: Tool::Crop,
            color: Color32::from_rgb(255, 91, 96),
            width: 4.,
            draft: None,
            moving: None,
            text_anchor: None,
            text: String::new(),
            text_focus: false,
            save_dialog: false,
            destination: destination.display().to_string(),
            explicit_destination,
            error: None,
        }
    }
    pub fn pending_text(&self) -> Option<&str> {
        self.text_anchor.map(|_| self.text.as_str())
    }
    pub fn tool(&self) -> &'static str {
        self.tool.shortcut()
    }
    pub fn deactivate(&mut self) {
        self.doc.reset_crop();
        self.draft = None;
        self.moving = None;
        self.text_anchor = None;
    }
    fn save(&mut self) {
        match crate::save_png(&self.doc.export(), std::path::Path::new(&self.destination)) {
            Ok(()) => self.done = true,
            Err(e) => self.error = Some(format!("Save failed: {e:#}")),
        }
    }
    fn copy(&mut self) {
        match crate::clipboard::copy(&self.doc.export()) {
            Ok(()) => self.done = true,
            Err(e) => self.error = Some(format!("Copy failed: {e:#}")),
        }
    }
    fn commit_text(&mut self) {
        if let Some(pos) = self.text_anchor.take()
            && !self.text.trim().is_empty()
        {
            self.doc.add(Mark {
                tool: Tool::Text,
                points: vec![pos],
                color: self.color,
                width: self.width,
                text: self.text.clone(),
            });
        }
    }
    pub fn frame(&mut self, raw: egui::RawInput, logical: Vec2, active: bool) -> egui::FullOutput {
        self.claimed = false;
        self.hits.clear();
        self.toolbar_visible = false;
        let ctx = self.ctx.clone();
        let native = self.doc.bounds().size();
        let scale = native / logical;
        self.gesture(&raw.events, logical, active);
        ctx.run(raw, |ctx| {
            let key = |k| ctx.input(|i| i.key_pressed(k));
            if key(egui::Key::Escape) {
                if self.save_dialog {
                    self.save_dialog = false;
                } else if self.text_anchor.is_some() {
                    self.text_anchor = None;
                } else if self.draft.is_some() {
                    self.draft = None;
                    self.moving = None;
                } else {
                    self.done = true;
                }
            }
            if active && !ctx.wants_keyboard_input() {
                let command=|k| ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key{key,pressed:true,modifiers,..} if *key==k && modifiers.command)));
                if [egui::Key::C,egui::Key::S,egui::Key::Z,egui::Key::Y].into_iter().any(command) {
                    if command(egui::Key::C) && self.doc.crop.is_some() {
                        self.copy();
                    }
                    if command(egui::Key::S) && self.doc.crop.is_some() {
                        if self.explicit_destination {
                            self.save();
                        } else {
                            self.save_dialog = true;
                        }
                    }
                    if command(egui::Key::Z) {
                        if ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key{key:egui::Key::Z,pressed:true,modifiers,..} if modifiers.shift))) {
                            self.doc.redo();
                        } else {
                            self.doc.undo();
                        }
                    }
                    if command(egui::Key::Y) {
                        self.doc.redo();
                    }
                } else {
                    for (k, t) in [
                        (egui::Key::S, Tool::Crop),
                        (egui::Key::P, Tool::Pen),
                        (egui::Key::A, Tool::Arrow),
                        (egui::Key::R, Tool::Rectangle),
                        (egui::Key::E, Tool::Ellipse),
                        (egui::Key::H, Tool::Highlight),
                        (egui::Key::B, Tool::Redact),
                        (egui::Key::T, Tool::Text),
                        (egui::Key::N, Tool::Pin),
                    ] {
                        if key(k) {
                            self.tool = t;
                        }
                    }
                }
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let screen = Rect::from_min_size(Pos2::ZERO, logical);
                    ui.interact(screen, ui.id().with("desktop"), Sense::hover());
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                    if let Some(crop) = self.visible_crop() {
                        let r = Rect::from_min_max(
                            (crop.min.to_vec2() / scale).to_pos2(),
                            (crop.max.to_vec2() / scale).to_pos2(),
                        );
                        ui.painter().rect_stroke(
                            r,
                            0,
                            Stroke::new(1.0_f32, ACCENT),
                            StrokeKind::Outside,
                        );
                        for p in [
                            r.left_top(),
                            r.right_top(),
                            r.left_bottom(),
                            r.right_bottom(),
                        ] {
                            ui.painter().rect_filled(
                                Rect::from_center_size(p, Vec2::splat(5.)),
                                0,
                                Color32::WHITE,
                            );
                        }
                        let dimensions = format!(
                            "{} × {}",
                            (crop.max.x.ceil() - crop.min.x.floor()) as u32,
                            (crop.max.y.ceil() - crop.min.y.floor()) as u32
                        );
                        let pos = Pos2::new(r.min.x + 3., (r.min.y - 20.).max(3.));
                        let galley =
                            ui.painter()
                                .layout_no_wrap(dimensions, FontId::monospace(11.), INK);
                        let label = Rect::from_min_size(pos, galley.size() + Vec2::new(10., 4.));
                        ui.painter().rect_filled(label, 3, BAR);
                        ui.painter().galley(pos + Vec2::new(5., 2.), galley, INK);
                    } else if active {
                        ui.painter().text(
                            Pos2::new(logical.x / 2., logical.y - 36.),
                            Align2::CENTER_CENTER,
                            "Drag to select · Esc to cancel",
                            FontId::proportional(13.),
                            INK,
                        );
                    }
                });
            if active && self.doc.crop.is_some() && self.draft.is_none() {
                self.toolbar(ctx, logical, scale);
            }
            if let Some(anchor) = self.text_anchor {
                let pos = (anchor.to_vec2() / scale).to_pos2();
                egui::Area::new("caption".into())
                    .order(egui::Order::Foreground)
                    .fade_in(false)
                    .fixed_pos(Pos2::new(
                        pos.x.min(logical.x - 280.).max(0.),
                        pos.y.min(logical.y - 80.).max(0.),
                    ))
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style())
                            .inner_margin(8)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let edit = ui.add(
                                        egui::TextEdit::singleline(&mut self.text)
                                            .desired_width(190.)
                                            .hint_text("Type text…"),
                                    );
                                    if self.text_focus {
                                        edit.request_focus();
                                        self.text_focus = false;
                                    }
                                    if ui.button("Add").clicked() || key(egui::Key::Enter) {
                                        self.commit_text();
                                    }
                                });
                            });
                    });
            }
            if self.save_dialog {
                egui::Window::new("Save screenshot")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .show(ctx, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.destination).desired_width(380.),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Save PNG").clicked() || key(egui::Key::Enter) {
                                self.save();
                            }
                            if ui.button("Cancel").clicked() {
                                self.save_dialog = false;
                            }
                        });
                    });
            }
            if let Some(error) = &self.error {
                egui::Area::new("error".into())
                    .anchor(Align2::CENTER_BOTTOM, Vec2::new(0., -16.))
                    .order(egui::Order::Foreground)
                    .fade_in(false)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style())
                            .inner_margin(10)
                            .show(ui, |ui| {
                                ui.label(error);
                            });
                    });
            }
        })
    }
    fn gesture(&mut self, events: &[egui::Event], logical: Vec2, active: bool) {
        let native = self.doc.bounds().size();
        let scale = native / logical;
        let pixel = |p: Pos2| {
            Pos2::new(
                (p.x * scale.x).clamp(0., native.x),
                (p.y * scale.y).clamp(0., native.y),
            )
        };
        for event in events {
            match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => {
                    if self.save_dialog
                        || self.text_anchor.is_some()
                        || self
                            .ctx
                            .layer_id_at(*pos)
                            .is_some_and(|layer| layer.order >= egui::Order::Foreground)
                    {
                        continue;
                    }
                    self.claimed = true;
                    let p = pixel(*pos);
                    let crop = self.doc.crop;
                    let corner = crop.and_then(|r| {
                        [
                            (r.left_top(), r.right_bottom()),
                            (r.right_top(), r.left_bottom()),
                            (r.left_bottom(), r.right_top()),
                            (r.right_bottom(), r.left_top()),
                        ]
                        .into_iter()
                        .find(|(a, _)| ((a.to_vec2() - p.to_vec2()) / scale).length() < 9.)
                    });
                    let tool = if crop.is_none()
                        || !active
                        || !crop.unwrap().contains(p)
                        || corner.is_some()
                    {
                        Tool::Crop
                    } else {
                        self.tool
                    };
                    if tool == Tool::Text {
                        self.text_anchor = Some(p);
                        self.text.clear();
                        self.text_focus = true;
                        continue;
                    }
                    if tool == Tool::Pin {
                        self.doc.add(Mark {
                            tool,
                            points: vec![p],
                            color: self.color,
                            width: self.width,
                            text: (self
                                .doc
                                .marks
                                .iter()
                                .filter(|m| m.tool == Tool::Pin)
                                .count()
                                + 1)
                            .to_string(),
                        });
                        continue;
                    }
                    if tool == Tool::Crop
                        && corner.is_none()
                        && let Some(r) = crop
                        && r.contains(p)
                    {
                        self.moving = Some((r, p));
                    }
                    self.draft = Some(Mark {
                        tool,
                        points: vec![corner.map(|(_, opposite)| opposite).unwrap_or(p), p],
                        color: self.color,
                        width: self.width,
                        text: String::new(),
                    });
                }
                egui::Event::PointerMoved(pos) => {
                    if let Some(mark) = &mut self.draft {
                        let p = pixel(*pos);
                        if let Some((crop, origin)) = self.moving {
                            let min = (crop.min + (p - origin))
                                .to_vec2()
                                .clamp(Vec2::ZERO, native - crop.size());
                            mark.points = vec![min.to_pos2(), (min + crop.size()).to_pos2()];
                        } else if mark.tool == Tool::Pen {
                            mark.points.push(p);
                        } else {
                            *mark.points.last_mut().unwrap() = p;
                        }
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    ..
                } => {
                    if let Some(mut mark) = self.draft.take() {
                        if self.moving.is_none() {
                            if mark.tool == Tool::Pen {
                                mark.points.push(pixel(*pos));
                            } else {
                                *mark.points.last_mut().unwrap() = pixel(*pos);
                            }
                        }
                        if mark.tool == Tool::Crop {
                            self.doc.set_crop(mark.bounds());
                            if self.doc.crop.is_some() {
                                self.tool = Tool::Pen;
                            }
                        } else if mark.points.len() > 3
                            || mark.points[0].distance(*mark.points.last().unwrap()) > 1.
                        {
                            self.doc.add(mark);
                        }
                        self.moving = None;
                    }
                }
                _ => {}
            }
        }
    }
    fn visible_crop(&self) -> Option<Rect> {
        self.draft
            .as_ref()
            .filter(|m| m.tool == Tool::Crop)
            .map(Mark::bounds)
            .or(self.doc.crop)
    }
    pub fn background(&self, active: bool) -> RgbaImage {
        let mut image = if active {
            self.doc.rendered(self.draft.as_ref())
        } else {
            self.doc.original.clone()
        };
        let crop = if active { self.visible_crop() } else { None };
        let width = image.width() as usize;
        for (y, row) in image.as_mut().chunks_exact_mut(width * 4).enumerate() {
            let spans = if let Some(r) =
                crop.filter(|r| y as f32 >= r.min.y.floor() && (y as f32) < r.max.y.ceil())
            {
                [
                    (0, (r.min.x.floor() as usize).min(width) * 4),
                    ((r.max.x.ceil() as usize).min(width) * 4, width * 4),
                ]
            } else {
                [(0, width * 4), (0, 0)]
            };
            for (start, end) in spans {
                for p in row[start..end].as_chunks_mut::<4>().0 {
                    for c in p.iter_mut().take(3) {
                        *c = (u16::from(*c) * 135 / 255) as u8;
                    }
                }
            }
        }
        image
    }
    fn toolbar(&mut self, ctx: &Context, logical: Vec2, scale: Vec2) {
        let crop = self.doc.crop.unwrap();
        let r = Rect::from_min_max(
            (crop.min.to_vec2() / scale).to_pos2(),
            (crop.max.to_vec2() / scale).to_pos2(),
        );
        let width: f32 = 500.;
        let x = r.center().x.clamp(
            (width / 2.).min(logical.x / 2.),
            (logical.x - width / 2.).max(logical.x / 2.),
        );
        let y = if r.max.y + 52. < logical.y {
            r.max.y + 12.
        } else if r.min.y >= 58. {
            r.min.y - 52.
        } else {
            logical.y - 48.
        };
        egui::Area::new("toolbar".into())
            .pivot(Align2::CENTER_TOP)
            .order(egui::Order::Foreground)
            .fade_in(false)
            .fixed_pos(Pos2::new(x, y.max(4.)))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(BAR)
                    .stroke(Stroke::new(1.0_f32, Color32::from_gray(65)))
                    .corner_radius(5)
                    .inner_margin(5)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            self.toolbar_visible = ui.is_visible();
                            for tool in Tool::ALL {
                                let response = tool_button(ui, tool, self.tool == tool);
                                self.hits
                                    .insert(tool.shortcut().to_lowercase(), response.rect);
                                if response.clicked() {
                                    self.tool = tool;
                                    self.text_anchor = None;
                                }
                                response.on_hover_text(format!(
                                    "{} ({})",
                                    tool.label(),
                                    tool.shortcut()
                                ));
                            }
                            ui.separator();
                            let palette = ui.menu_button(" ", |ui| {
                                ui.horizontal(|ui| {
                                    for color in [
                                        Color32::from_rgb(255, 91, 96),
                                        Color32::from_rgb(255, 200, 75),
                                        Color32::from_rgb(86, 213, 143),
                                        ACCENT,
                                        Color32::WHITE,
                                        Color32::BLACK,
                                    ] {
                                        let response = ui
                                            .add(egui::Button::new(" ").min_size(Vec2::splat(24.)));
                                        ui.painter().circle_filled(
                                            response.rect.center(),
                                            6.,
                                            color,
                                        );
                                        if response.clicked() {
                                            self.color = color;
                                            ui.close();
                                        }
                                    }
                                });
                                ui.add(
                                    egui::Slider::new(&mut self.width, 2.0..=16.0)
                                        .text("Stroke")
                                        .integer(),
                                );
                            });
                            ui.painter().circle_filled(
                                palette.response.rect.center(),
                                5.,
                                self.color,
                            );
                            for (name, enabled) in [
                                ("undo", self.doc.can_undo()),
                                ("redo", self.doc.can_redo()),
                                ("copy", true),
                                ("save", true),
                                ("close", true),
                            ] {
                                let response = action_button(ui, name, enabled);
                                self.hits.insert(name.into(), response.rect);
                                if response.clicked() {
                                    match name {
                                        "undo" => self.doc.undo(),
                                        "redo" => self.doc.redo(),
                                        "copy" => self.copy(),
                                        "save" => {
                                            if self.explicit_destination {
                                                self.save();
                                            } else {
                                                self.save_dialog = true;
                                            }
                                        }
                                        "close" => self.done = true,
                                        _ => {}
                                    }
                                }
                                response.on_hover_text(match name {
                                    "undo" => "Undo (Ctrl+Z)",
                                    "redo" => "Redo (Ctrl+Shift+Z)",
                                    "copy" => "Copy and close (Ctrl+C)",
                                    "save" => "Save and close (Ctrl+S)",
                                    _ => "Cancel (Esc)",
                                });
                            }
                        });
                    });
            });
    }
}
fn tool_button(ui: &mut egui::Ui, tool: Tool, selected: bool) -> egui::Response {
    let (r, response) = ui.allocate_exact_size(Vec2::splat(29.), Sense::click());
    if response.hovered() || selected {
        ui.painter().rect_filled(
            r,
            3,
            if selected {
                Color32::from_gray(65)
            } else {
                Color32::from_gray(48)
            },
        );
    }
    let c = r.center();
    let s = Stroke::new(1.5_f32, INK);
    let boxr = Rect::from_center_size(c, Vec2::new(16., 13.));
    match tool {
        Tool::Crop => {
            ui.painter().line(
                vec![
                    c + Vec2::new(-7., -10.),
                    c + Vec2::new(-7., 7.),
                    c + Vec2::new(10., 7.),
                ],
                s,
            );
            ui.painter().line(
                vec![
                    c + Vec2::new(-10., -7.),
                    c + Vec2::new(7., -7.),
                    c + Vec2::new(7., 10.),
                ],
                s,
            );
        }
        Tool::Pen => {
            ui.painter().line(
                vec![
                    c + Vec2::new(-7., 7.),
                    c + Vec2::new(-5., 1.),
                    c + Vec2::new(5., -9.),
                    c + Vec2::new(9., -5.),
                    c + Vec2::new(-1., 5.),
                    c + Vec2::new(-7., 7.),
                ],
                s,
            );
        }
        Tool::Arrow => {
            ui.painter()
                .arrow(boxr.left_bottom(), boxr.right_top() - boxr.left_bottom(), s);
        }
        Tool::Rectangle => {
            ui.painter().rect_stroke(boxr, 0, s, StrokeKind::Inside);
        }
        Tool::Ellipse => {
            ui.painter()
                .add(egui::Shape::ellipse_stroke(c, Vec2::new(8., 6.5), s));
        }
        Tool::Highlight => {
            ui.painter().line_segment(
                [c + Vec2::new(-5., 5.), c + Vec2::new(5., -5.)],
                Stroke::new(5.0_f32, INK),
            );
            ui.painter()
                .line_segment([c + Vec2::new(-8., 9.), c + Vec2::new(8., 9.)], s);
        }
        Tool::Redact => {
            ui.painter().rect_filled(boxr, 1, INK);
        }
        Tool::Text => {
            ui.painter().text(
                c,
                Align2::CENTER_CENTER,
                "T",
                FontId::proportional(20.),
                INK,
            );
        }
        Tool::Pin => {
            ui.painter().circle_stroke(c, 8., s);
            ui.painter().text(
                c,
                Align2::CENTER_CENTER,
                "1",
                FontId::proportional(12.),
                INK,
            );
        }
    }
    response
}
fn action_button(ui: &mut egui::Ui, name: &str, enabled: bool) -> egui::Response {
    let (r, response) = ui.allocate_exact_size(
        Vec2::splat(29.),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if response.hovered() && enabled {
        ui.painter().rect_filled(r, 3, Color32::from_gray(48));
    }
    let c = r.center();
    let s = Stroke::new(1.5_f32, if enabled { INK } else { Color32::from_gray(85) });
    match name {
        "copy" => {
            ui.painter().rect_stroke(
                Rect::from_center_size(c + Vec2::new(2., 2.), Vec2::splat(12.)),
                1,
                s,
                StrokeKind::Inside,
            );
            ui.painter().line(
                vec![
                    c + Vec2::new(-1., -9.),
                    c + Vec2::new(-9., -9.),
                    c + Vec2::new(-9., 1.),
                ],
                s,
            );
        }
        "save" => {
            ui.painter()
                .arrow(c + Vec2::new(0., -9.), Vec2::new(0., 12.), s);
            ui.painter().line(
                vec![
                    c + Vec2::new(-8., 3.),
                    c + Vec2::new(-8., 8.),
                    c + Vec2::new(8., 8.),
                    c + Vec2::new(8., 3.),
                ],
                s,
            );
        }
        "close" => {
            ui.painter()
                .line_segment([c - Vec2::splat(6.), c + Vec2::splat(6.)], s);
            ui.painter()
                .line_segment([c + Vec2::new(-6., 6.), c + Vec2::new(6., -6.)], s);
        }
        _ => {
            let sign = if name == "redo" { -1. } else { 1. };
            ui.painter().line(
                [
                    (-7., -2.),
                    (-3., -6.),
                    (4., -6.),
                    (8., -2.),
                    (8., 4.),
                    (4., 7.),
                    (-2., 7.),
                ]
                .into_iter()
                .map(|(x, y)| c + Vec2::new(x * sign, y))
                .collect(),
                s,
            );
            ui.painter().line(
                vec![
                    c + Vec2::new(-7. * sign, -7.),
                    c + Vec2::new(-7. * sign, -2.),
                    c + Vec2::new(-2. * sign, -2.),
                ],
                s,
            );
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toolbar_becomes_visible_and_accepts_a_batched_click() {
        let mut ui = OverlayUi::new(RgbaImage::new(1440, 900), PathBuf::from("unused"), true);
        ui.doc.set_crop(Rect::from_min_max(
            egui::pos2(300., 200.),
            egui::pos2(1000., 600.),
        ));
        let raw = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440., 900.))),
            ..Default::default()
        };
        let mut raster = crate::raster::Raster::default();
        let first = ui.frame(raw(), Vec2::new(1440., 900.), true);
        for (id, delta) in &first.textures_delta.set {
            raster.update(*id, delta);
        }
        let second = ui.frame(raw(), Vec2::new(1440., 900.), true);
        for (id, delta) in &second.textures_delta.set {
            raster.update(*id, delta);
        }
        let mut image = ui.background(true);
        let primitives = ui.ctx.tessellate(second.shapes, second.pixels_per_point);
        raster.draw(&mut image, &primitives, second.pixels_per_point);
        let position = ui.hits["save"].center();
        assert_ne!(
            image
                .get_pixel(position.x as u32 - 7, position.y as u32 - 7)
                .0,
            [0, 0, 0, 0],
            "Toolbar must be painted"
        );
        assert!(ui.toolbar_visible);
        let pos = ui.hits["a"].center();
        let mut input = raw();
        input.events = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ];
        ui.frame(input, Vec2::new(1440., 900.), true);
        assert_eq!(ui.tool, Tool::Arrow);
    }
    #[test]
    fn caption_accepts_text_and_enter() {
        let mut ui = OverlayUi::new(RgbaImage::new(1440, 900), PathBuf::from("unused"), true);
        ui.doc.set_crop(Rect::from_min_max(
            egui::pos2(300., 200.),
            egui::pos2(1000., 600.),
        ));
        ui.tool = Tool::Text;
        let input = |events| egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1440., 900.))),
            events,
            ..Default::default()
        };
        let pos = egui::pos2(450., 350.);
        ui.frame(
            input(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]),
            Vec2::new(1440., 900.),
            true,
        );
        ui.frame(input(vec![]), Vec2::new(1440., 900.), true);
        ui.frame(
            input(vec![egui::Event::Text("hello".into())]),
            Vec2::new(1440., 900.),
            true,
        );
        assert_eq!(ui.pending_text(), Some("hello"));
        ui.frame(
            input(vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }]),
            Vec2::new(1440., 900.),
            true,
        );
        assert_eq!(ui.doc.marks.len(), 1);
        assert_eq!(ui.doc.marks[0].text, "hello");
    }
    #[test]
    fn a_complete_drag_in_one_frame_selects_exact_native_pixels() {
        let mut ui = OverlayUi::new(RgbaImage::new(1440, 900), PathBuf::from("unused"), true);
        let start = egui::pos2(100., 100.);
        let end = egui::pos2(500., 300.);
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(720., 450.))),
            events: vec![
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerMoved(end),
                egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        };
        ui.frame(raw, Vec2::new(720., 450.), true);
        assert_eq!(ui.doc.crop_pixels(), (200, 200, 800, 400));
    }
}
