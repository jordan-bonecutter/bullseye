use crate::document::{Document, Mark, Tool};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

const BG: Color32 = Color32::from_rgb(13, 17, 23);
const PANEL: Color32 = Color32::from_rgb(20, 25, 33);
const BORDER: Color32 = Color32::from_rgb(43, 52, 63);
const MUTED: Color32 = Color32::from_rgb(141, 155, 169);
const INK: Color32 = Color32::from_rgb(231, 237, 241);
const MINT: Color32 = Color32::from_rgb(132, 239, 205);

pub struct Editor {
    doc: Document,
    texture: Option<egui::TextureHandle>,
    dirty: bool,
    tool: Tool,
    color: Color32,
    width: f32,
    draft: Option<Mark>,
    text: String,
    zoom: f32,
    display_scale: f32,
    output: String,
    save_path: String,
    status: String,
    status_error: bool,
    recapture: Arc<AtomicBool>,
    help: bool,
    started: Instant,
}
impl Editor {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        image: image::RgbaImage,
        label: String,
        path: PathBuf,
        recapture: Arc<AtomicBool>,
    ) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = PANEL;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = BG;
        visuals.override_text_color = Some(INK);
        visuals.selection.bg_fill = Color32::from_rgb(40, 81, 73);
        visuals.selection.stroke = Stroke::new(1.0_f32, MINT);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(31, 39, 49);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(31, 39, 49);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 60, 67);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, MINT.gamma_multiply(0.6));
        visuals.widgets.active.bg_fill = Color32::from_rgb(44, 76, 68);
        cc.egui_ctx.set_visuals(visuals);
        cc.egui_ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(10.0, 10.0);
            s.spacing.button_padding = egui::vec2(14.0, 9.0);
            s.text_styles
                .insert(egui::TextStyle::Body, FontId::proportional(14.0));
            s.text_styles
                .insert(egui::TextStyle::Button, FontId::proportional(14.0));
        });
        Self {
            doc: Document::new(image),
            texture: None,
            dirty: true,
            tool: Tool::Crop,
            color: MINT,
            width: 4.0,
            draft: None,
            text: "Look here".into(),
            zoom: 1.0,
            display_scale: 1.0,
            output: label,
            save_path: path.display().to_string(),
            status: "Ready when you are. Drag to select a region.".into(),
            status_error: false,
            recapture,
            help: false,
            started: Instant::now(),
        }
    }
    fn message(&mut self, text: impl Into<String>, error: bool) {
        self.status = text.into();
        self.status_error = error;
    }
    fn save(&mut self) {
        match crate::save_png(&self.doc.export(), std::path::Path::new(&self.save_path)) {
            Ok(()) => self.message(format!("Saved to {}", self.save_path), false),
            Err(e) => self.message(format!("Save failed: {e:#}"), true),
        }
    }
    fn copy(&mut self) {
        match crate::clipboard::copy(&self.doc.export()) {
            Ok(()) => self.message("Copied! Your screenshot is ready to paste.", false),
            Err(e) => self.message(format!("Copy failed: {e:#}"), true),
        }
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let key = |k| ctx.input(|i| i.key_pressed(k));
        let command = ctx.input(|i| i.modifiers.command);
        if command {
            if key(egui::Key::S) {
                self.save();
            }
            if key(egui::Key::C) {
                self.copy();
            }
            if key(egui::Key::Z) {
                if ctx.input(|i| i.modifiers.shift) {
                    self.doc.redo();
                } else {
                    self.doc.undo();
                }
                self.dirty = true;
            }
            if key(egui::Key::Y) {
                self.doc.redo();
                self.dirty = true;
            }
            if key(egui::Key::A) {
                self.doc.reset_crop();
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
                    self.draft = None;
                }
            }
            if key(egui::Key::Escape) {
                if self.help {
                    self.help = false;
                } else if self.draft.is_some() {
                    self.draft = None;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            if key(egui::Key::F1) {
                self.help = !self.help;
            }
        }
    }
    fn top(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .exact_height(82.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(24, 17)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(38.0, 38.0), Sense::hover());
                    for radius in [15.0, 8.5, 2.0] {
                        ui.painter()
                            .circle_stroke(r.center(), radius, Stroke::new(2.0_f32, MINT));
                    }
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("bullseye").size(25.0).strong());
                        ui.label(
                            egui::RichText::new("A good shot. A little polish.")
                                .size(12.0)
                                .color(MUTED),
                        );
                    });
                    ui.add_space(24.0);
                    ui.separator();
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("SCREENSHOT / EDITOR")
                                .size(10.0)
                                .color(MUTED)
                                .extra_letter_spacing(1.5),
                        );
                        ui.label(egui::RichText::new(&self.output).size(13.0));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("Save image  ↗").color(BG).strong(),
                                )
                                .fill(MINT)
                                .corner_radius(8),
                            )
                            .on_hover_text("Save PNG · Ctrl+S")
                            .clicked()
                        {
                            self.save();
                        }
                        if ui
                            .add(egui::Button::new("Copy image").corner_radius(8))
                            .on_hover_text("Copy PNG · Ctrl+C")
                            .clicked()
                        {
                            self.copy();
                        }
                        ui.add_space(4.0);
                        if ui
                            .add_enabled(
                                true,
                                egui::Button::new("New capture")
                                    .fill(Color32::TRANSPARENT)
                                    .frame(false),
                            )
                            .clicked()
                        {
                            self.recapture.store(true, Ordering::Relaxed);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
            });
    }
    fn tools(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("tools")
            .exact_width(82.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(12, 20)),
            )
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("TOOLS")
                                .size(10.0)
                                .color(MUTED)
                                .extra_letter_spacing(1.2),
                        );
                        ui.add_space(8.0);
                        for tool in Tool::ALL {
                            let (r, response) =
                                ui.allocate_exact_size(egui::vec2(52.0, 44.0), Sense::click());
                            let selected = self.tool == tool;
                            if selected || response.hovered() {
                                ui.painter().rect(
                                    r,
                                    8,
                                    if selected {
                                        Color32::from_rgb(34, 62, 57)
                                    } else {
                                        Color32::from_rgb(31, 39, 49)
                                    },
                                    Stroke::new(
                                        1.0_f32,
                                        if selected {
                                            MINT.gamma_multiply(0.45)
                                        } else {
                                            BORDER
                                        },
                                    ),
                                    StrokeKind::Inside,
                                );
                            }
                            icon(
                                ui.painter(),
                                r.center(),
                                tool,
                                if selected { MINT } else { MUTED },
                            );
                            if response.clicked() {
                                self.tool = tool;
                                self.draft = None;
                            }
                            response.on_hover_text(format!(
                                "{} · {}",
                                tool.label(),
                                tool.shortcut()
                            ));
                        }
                        ui.add_space(16.0);
                        ui.separator();
                        ui.add_space(8.0);
                        if history_button(ui, false, self.doc.can_undo())
                            .on_hover_text("Undo · Ctrl+Z")
                            .clicked()
                        {
                            self.doc.undo();
                            self.dirty = true;
                        }
                        if history_button(ui, true, self.doc.can_redo())
                            .on_hover_text("Redo · Ctrl+Shift+Z")
                            .clicked()
                        {
                            self.doc.redo();
                            self.dirty = true;
                        }
                        ui.add_space(16.0);
                        if ui
                            .button("?")
                            .on_hover_text("Keyboard shortcuts · F1")
                            .clicked()
                        {
                            self.help = !self.help;
                        }
                    });
                });
            });
    }
    fn inspector(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector")
            .exact_width(252.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(22, 24)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    eyebrow(ui, "MAKE IT YOURS");
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new(self.tool.label()).size(23.0).strong());
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(match self.tool {
                            Tool::Crop => "Drag across the image to frame your shot.",
                            Tool::Pen => "Draw freely. A little emphasis goes a long way.",
                            Tool::Arrow => "Point them in the right direction.",
                            Tool::Rectangle | Tool::Ellipse => "Draw a shape around what matters.",
                            Tool::Highlight => "Give the important bits a little glow.",
                            Tool::Redact => "Cover private details with an opaque block.",
                            Tool::Text => "Write a caption, then click to place it.",
                            Tool::Pin => "Click to add a numbered step.",
                        })
                        .size(13.0)
                        .color(MUTED),
                    );
                    ui.add_space(22.0);
                    ui.separator();
                    ui.add_space(18.0);
                    if !matches!(self.tool, Tool::Crop | Tool::Redact) {
                        eyebrow(ui, "COLOR");
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            for color in [
                                MINT,
                                Color32::from_rgb(255, 125, 141),
                                Color32::from_rgb(255, 209, 123),
                                Color32::from_rgb(156, 159, 255),
                                Color32::from_rgb(111, 194, 255),
                                INK,
                            ] {
                                let (r, response) =
                                    ui.allocate_exact_size(egui::vec2(25.0, 29.0), Sense::click());
                                ui.painter().circle_filled(r.center(), 8.0, color);
                                if self.color == color {
                                    ui.painter().circle_stroke(
                                        r.center(),
                                        11.5,
                                        Stroke::new(1.3_f32, color),
                                    );
                                }
                                if response.clicked() {
                                    self.color = color;
                                }
                            }
                        });
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            eyebrow(ui, "STROKE");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{} px", self.width as u32))
                                            .color(MUTED)
                                            .size(12.0),
                                    );
                                },
                            );
                        });
                        ui.add(
                            egui::Slider::new(&mut self.width, 2.0..=16.0)
                                .show_value(false)
                                .integer(),
                        );
                        ui.add_space(18.0);
                        ui.separator();
                        ui.add_space(18.0);
                    }
                    if self.tool == Tool::Text {
                        eyebrow(ui, "CAPTION");
                        ui.add_space(6.0);
                        ui.add(
                            egui::TextEdit::multiline(&mut self.text)
                                .desired_width(200.0)
                                .desired_rows(3)
                                .hint_text("Say something…"),
                        );
                        ui.add_space(18.0);
                        ui.separator();
                        ui.add_space(18.0);
                    }
                    eyebrow(ui, "YOUR SELECTION");
                    ui.add_space(10.0);
                    let (_, _, w, h) = self.doc.crop_pixels();
                    egui::Frame::new()
                        .fill(BG)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .corner_radius(8)
                        .inner_margin(14)
                        .show(ui, |ui| {
                            ui.set_min_width(172.0);
                            ui.label(egui::RichText::new(format!("{w} × {h}")).size(24.0));
                            ui.label(
                                egui::RichText::new(if self.doc.crop.is_some() {
                                    "Selected region · pixels"
                                } else {
                                    "Entire screenshot · pixels"
                                })
                                .size(11.0)
                                .color(MUTED),
                            );
                        });
                    if self.doc.crop.is_some()
                        && ui
                            .add(egui::Button::new("Reset to full image").frame(false))
                            .clicked()
                    {
                        self.doc.reset_crop();
                    }
                    ui.add_space(20.0);
                    eyebrow(ui, "EXPORT");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label("PNG");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(egui::RichText::new("Lossless").color(MINT).size(12.0));
                        });
                    });
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Save location").size(12.0).color(MUTED));
                    ui.add(
                        egui::TextEdit::multiline(&mut self.save_path)
                            .desired_rows(3)
                            .desired_width(200.0)
                            .font(FontId::monospace(11.0)),
                    );
                    ui.add_space(24.0);
                    egui::Frame::new()
                        .fill(Color32::from_rgb(24, 40, 39))
                        .corner_radius(8)
                        .inner_margin(14)
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new("Made for your flow")
                                    .size(13.0)
                                    .color(MINT),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Native Wayland capture. All edits stay on your machine.",
                                )
                                .size(12.0)
                                .color(MUTED),
                            );
                        });
                });
            });
    }
    fn footer(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("footer")
            .exact_height(43.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(24, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(8., 8.), Sense::hover());
                    ui.painter().circle_filled(
                        r.center(),
                        3.,
                        if self.status_error {
                            Color32::from_rgb(255, 125, 141)
                        } else {
                            MINT
                        },
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&self.status)
                                .color(if self.status_error {
                                    Color32::from_rgb(255, 125, 141)
                                } else {
                                    MUTED
                                })
                                .size(12.0),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("F1  Shortcuts     Esc  Close")
                                .size(11.0)
                                .color(MUTED),
                        );
                    });
                });
            });
    }
    fn canvas(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(28))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("THE CANVAS")
                                .size(10.0)
                                .color(MUTED)
                                .extra_letter_spacing(1.5),
                        );
                        ui.label(egui::RichText::new("Bring the details into focus.").size(16.0));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Fit").clicked() {
                            self.zoom = 1.0;
                        }
                        if ui.small_button("+").clicked() {
                            self.zoom = (self.zoom * 1.25).min(4.0);
                        }
                        ui.label(
                            egui::RichText::new(format!(
                                "{}%",
                                (self.display_scale * 100.0).round() as u32
                            ))
                            .size(12.0)
                            .color(MUTED),
                        );
                        if ui.small_button("−").clicked() {
                            self.zoom = (self.zoom / 1.25).max(0.5);
                        }
                    });
                });
                ui.add_space(22.0);
                let available = ui.available_size() - egui::vec2(0.0, 32.0);
                let size = self.doc.bounds().size();
                let scale = ((available.x - 16.0) / size.x)
                    .min((available.y - 16.0) / size.y)
                    .max(0.01)
                    * self.zoom;
                self.display_scale = scale;
                if self.dirty {
                    let image = self.doc.rendered(None);
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    );
                    if let Some(texture) = &mut self.texture {
                        texture.set(color_image, egui::TextureOptions::LINEAR);
                    } else {
                        self.texture = Some(ctx.load_texture(
                            "screenshot",
                            color_image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                    self.dirty = false;
                }
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .max_height(available.y)
                    .show(ui, |ui| {
                        let area_size = (size * scale + egui::vec2(16., 16.)).max(available);
                        let (area, _) = ui.allocate_exact_size(area_size, Sense::hover());
                        let painter = ui.painter();
                        // A quiet dot grid gives the screenshot its own workspace.
                        let visible = area.intersect(ui.clip_rect());
                        let mut y = visible.min.y;
                        while y < visible.max.y {
                            let mut x = visible.min.x;
                            while x < visible.max.x {
                                painter.circle_filled(
                                    Pos2::new(x, y),
                                    0.7,
                                    Color32::from_rgb(36, 44, 54),
                                );
                                x += 24.0;
                            }
                            y += 24.0;
                        }
                        let rect = Rect::from_center_size(area.center(), size * scale);
                        painter.rect_filled(rect.expand(7.0), 8, Color32::from_rgb(22, 29, 37));
                        painter.rect_stroke(
                            rect.expand(7.0),
                            8,
                            Stroke::new(1.0_f32, BORDER),
                            StrokeKind::Outside,
                        );
                        if let Some(texture) = &self.texture {
                            painter.image(
                                texture.id(),
                                rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        let response =
                            ui.interact(rect, ui.id().with("image"), Sense::click_and_drag());
                        if response.hovered() {
                            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                        }
                        let to_pixel = |p: Pos2| -> Pos2 {
                            let p = (p - rect.min) / scale;
                            Pos2::new(p.x.clamp(0.0, size.x), p.y.clamp(0.0, size.y))
                        };
                        if response.drag_started() && !matches!(self.tool, Tool::Text | Tool::Pin) {
                            // egui reports the pointer after the drag threshold; recover the actual press origin.
                            let pos = ctx
                                .input(|i| i.pointer.press_origin())
                                .or_else(|| response.interact_pointer_pos());
                            if let Some(pos) = pos {
                                let p = to_pixel(pos);
                                let anchor = if self.tool == Tool::Crop {
                                    self.doc
                                        .crop
                                        .and_then(|crop| {
                                            [
                                                (crop.left_top(), crop.right_bottom()),
                                                (crop.right_top(), crop.left_bottom()),
                                                (crop.left_bottom(), crop.right_top()),
                                                (crop.right_bottom(), crop.left_top()),
                                            ]
                                            .into_iter()
                                            .find(|(corner, _)| corner.distance(p) * scale < 12.0)
                                            .map(|(_, opposite)| opposite)
                                        })
                                        .unwrap_or(p)
                                } else {
                                    p
                                };
                                self.draft = Some(Mark {
                                    tool: self.tool,
                                    points: vec![anchor, p],
                                    color: self.color,
                                    width: self.width,
                                    text: String::new(),
                                });
                            }
                        }
                        if response.dragged()
                            && let (Some(mark), Some(pos)) =
                                (&mut self.draft, response.interact_pointer_pos())
                        {
                            let p = to_pixel(pos);
                            if mark.tool == Tool::Pen {
                                if mark.points.last().unwrap().distance(p) > 0.5 {
                                    mark.points.push(p);
                                }
                            } else {
                                *mark.points.last_mut().unwrap() = p;
                            }
                        }
                        if response.drag_stopped()
                            && let Some(mut mark) = self.draft.take()
                        {
                            if let Some(pos) = response.interact_pointer_pos() {
                                if mark.tool == Tool::Pen {
                                    mark.points.push(to_pixel(pos));
                                } else {
                                    *mark.points.last_mut().unwrap() = to_pixel(pos);
                                }
                            }
                            if mark.tool == Tool::Crop {
                                self.doc.set_crop(mark.bounds());
                                self.message(
                                    "Region selected. Add a little polish, or save your shot.",
                                    false,
                                );
                            } else if mark.points[0].distance(*mark.points.last().unwrap()) > 1.0
                                || mark.tool == Tool::Pen
                            {
                                self.doc.add(mark);
                                self.dirty = true;
                            }
                        }
                        if response.clicked()
                            && matches!(self.tool, Tool::Text | Tool::Pin)
                            && let Some(pos) = response.interact_pointer_pos()
                        {
                            let text = if self.tool == Tool::Pin {
                                (self
                                    .doc
                                    .marks
                                    .iter()
                                    .filter(|m| m.tool == Tool::Pin)
                                    .count()
                                    + 1)
                                .to_string()
                            } else {
                                self.text.clone()
                            };
                            if !text.trim().is_empty() {
                                self.doc.add(Mark {
                                    tool: self.tool,
                                    points: vec![to_pixel(pos)],
                                    color: self.color,
                                    width: self.width,
                                    text,
                                });
                                self.dirty = true;
                            }
                        }
                        let crop = self
                            .draft
                            .as_ref()
                            .filter(|m| m.tool == Tool::Crop)
                            .map(Mark::bounds)
                            .or(self.doc.crop);
                        if let Some(crop) = crop {
                            let selected = Rect::from_min_max(
                                rect.min + crop.min.to_vec2() * scale,
                                rect.min + crop.max.to_vec2() * scale,
                            );
                            let shade = Color32::from_black_alpha(155);
                            for r in [
                                Rect::from_min_max(rect.min, Pos2::new(rect.max.x, selected.min.y)),
                                Rect::from_min_max(Pos2::new(rect.min.x, selected.max.y), rect.max),
                                Rect::from_min_max(
                                    Pos2::new(rect.min.x, selected.min.y),
                                    Pos2::new(selected.min.x, selected.max.y),
                                ),
                                Rect::from_min_max(
                                    Pos2::new(selected.max.x, selected.min.y),
                                    Pos2::new(rect.max.x, selected.max.y),
                                ),
                            ] {
                                painter.rect_filled(r, 0, shade);
                            }
                            painter.rect_stroke(
                                selected,
                                0,
                                Stroke::new(1.5_f32, MINT),
                                StrokeKind::Outside,
                            );
                            for pos in [
                                selected.left_top(),
                                selected.right_top(),
                                selected.left_bottom(),
                                selected.right_bottom(),
                            ] {
                                painter.rect_filled(
                                    Rect::from_center_size(pos, Vec2::splat(6.0)),
                                    1,
                                    MINT,
                                );
                            }
                            let label = format!(
                                "{} × {}",
                                (crop.max.x.ceil() - crop.min.x.floor()) as u32,
                                (crop.max.y.ceil() - crop.min.y.floor()) as u32
                            );
                            let tag = Rect::from_min_size(
                                Pos2::new(selected.min.x, (selected.min.y - 30.0).max(rect.min.y)),
                                egui::vec2(110., 24.),
                            );
                            painter.rect_filled(tag, 4, MINT);
                            painter.text(
                                tag.center(),
                                Align2::CENTER_CENTER,
                                label,
                                FontId::monospace(11.),
                                BG,
                            );
                        }
                        if let Some(draft) = &self.draft {
                            preview(painter, draft, rect.min, scale);
                        }
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} × {} original",
                            size.x as u32, size.y as u32
                        ))
                        .size(11.0)
                        .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} annotations  ·  Local only",
                                self.doc.marks.len()
                            ))
                            .size(11.0)
                            .color(MUTED),
                        );
                    });
                });
            });
    }
}
impl eframe::App for Editor {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.shortcuts(ctx);
        self.top(ctx);
        self.footer(ctx);
        self.tools(ctx);
        self.inspector(ctx);
        self.canvas(ctx);
        if self.help {
            egui::Window::new("A few handy shortcuts")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .open(&mut self.help)
                .show(ctx, |ui| {
                    ui.add_space(8.0);
                    egui::Grid::new("shortcuts")
                        .spacing(egui::vec2(30., 10.))
                        .show(ui, |ui| {
                            for tool in Tool::ALL {
                                ui.label(tool.shortcut());
                                ui.label(tool.label());
                                ui.end_row();
                            }
                            for (key, action) in [
                                ("Ctrl+S", "Save PNG"),
                                ("Ctrl+C", "Copy image"),
                                ("Ctrl+Z", "Undo"),
                                ("Ctrl+Shift+Z / Ctrl+Y", "Redo"),
                                ("Ctrl+A", "Reset selection"),
                                ("Esc", "Cancel drag / close"),
                                ("F1", "Show shortcuts"),
                            ] {
                                ui.label(key);
                                ui.label(action);
                                ui.end_row();
                            }
                        });
                    ui.add_space(8.0);
                });
        }
        // Optional bounded UI smoke test, used by the isolated headless harness.
        if let Ok(seconds) = std::env::var("BULLSEYE_SMOKE_SECONDS")
            && let Ok(seconds) = seconds.parse::<u64>()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            if self.started.elapsed().as_secs() >= seconds {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}
fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(10.0)
            .color(MUTED)
            .extra_letter_spacing(1.2),
    );
}
fn icon(p: &egui::Painter, c: Pos2, tool: Tool, color: Color32) {
    let s = Stroke::new(1.7_f32, color);
    let r = Rect::from_center_size(c, egui::vec2(19., 16.));
    match tool {
        Tool::Crop => {
            for (a, b, d) in [
                (
                    r.left_top() + egui::vec2(-3., 4.),
                    r.left_top(),
                    r.left_top() + egui::vec2(5., 0.),
                ),
                (
                    r.right_bottom() + egui::vec2(3., -4.),
                    r.right_bottom(),
                    r.right_bottom() + egui::vec2(-5., 0.),
                ),
            ] {
                p.line(vec![a, b, d], s);
            }
            p.rect_stroke(r.shrink(3.), 0, s, StrokeKind::Inside);
        }
        Tool::Pen => {
            p.line(
                vec![
                    c + egui::vec2(-8., 8.),
                    c + egui::vec2(-6., 2.),
                    c + egui::vec2(5., -9.),
                    c + egui::vec2(9., -5.),
                    c + egui::vec2(-2., 6.),
                    c + egui::vec2(-8., 8.),
                ],
                s,
            );
        }
        Tool::Arrow => {
            p.arrow(r.left_bottom(), r.right_top() - r.left_bottom(), s);
        }
        Tool::Rectangle => {
            p.rect_stroke(r, 2, s, StrokeKind::Inside);
        }
        Tool::Ellipse => {
            p.add(egui::Shape::ellipse_stroke(c, egui::vec2(10., 8.), s));
        }
        Tool::Highlight => {
            p.line_segment(
                [c + egui::vec2(-7., 6.), c + egui::vec2(6., -7.)],
                Stroke::new(6.0_f32, color),
            );
            p.line_segment([c + egui::vec2(-10., 10.), c + egui::vec2(10., 10.)], s);
        }
        Tool::Redact => {
            p.rect_filled(r, 2, color);
            p.line_segment(
                [
                    r.left_top() + egui::vec2(3., 3.),
                    r.right_bottom() - egui::vec2(3., 3.),
                ],
                Stroke::new(2.0_f32, PANEL),
            );
        }
        Tool::Text => {
            p.text(
                c,
                Align2::CENTER_CENTER,
                "T",
                FontId::proportional(24.),
                color,
            );
        }
        Tool::Pin => {
            p.circle_stroke(c, 10., s);
            p.text(
                c,
                Align2::CENTER_CENTER,
                "1",
                FontId::proportional(14.),
                color,
            );
        }
    }
}
fn preview(p: &egui::Painter, mark: &Mark, origin: Pos2, scale: f32) {
    let points: Vec<_> = mark
        .points
        .iter()
        .map(|v| origin + v.to_vec2() * scale)
        .collect();
    let a = points[0];
    let b = *points.last().unwrap();
    let r = Rect::from_two_pos(a, b);
    let s = Stroke::new(mark.width * scale, mark.color);
    match mark.tool {
        Tool::Pen => {
            p.line(points, s);
        }
        Tool::Arrow => {
            p.line_segment([a, b], s);
            let delta = b - a;
            if delta.length() > 0.0 {
                let unit = delta.normalized();
                let wing = egui::vec2(-unit.y, unit.x);
                let length = ((mark.width * 4. + 10.) * scale).min(delta.length() * 0.5);
                p.line(
                    vec![
                        b - unit * length + wing * length * 0.5,
                        b,
                        b - unit * length - wing * length * 0.5,
                    ],
                    s,
                );
            }
        }
        Tool::Rectangle => {
            p.rect_stroke(r, 0, s, StrokeKind::Middle);
        }
        Tool::Ellipse => {
            p.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2., s));
        }
        Tool::Highlight => {
            p.rect_filled(
                r,
                0,
                Color32::from_rgba_unmultiplied(mark.color.r(), mark.color.g(), mark.color.b(), 75),
            );
        }
        Tool::Redact => {
            p.rect_filled(r, 0, Color32::from_rgb(12, 16, 22));
        }
        _ => {}
    }
}

fn history_button(ui: &mut egui::Ui, redo: bool, enabled: bool) -> egui::Response {
    let (r, response) = ui.allocate_exact_size(
        egui::vec2(48.0, 32.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if response.hovered() && enabled {
        ui.painter()
            .rect_filled(r, 6, Color32::from_rgb(31, 39, 49));
    }
    let color = if enabled { MUTED } else { BORDER };
    let c = r.center();
    let sign = if redo { -1.0 } else { 1.0 };
    let points = [
        (-8., -3.),
        (-4., -7.),
        (3., -7.),
        (8., -3.),
        (8., 3.),
        (4., 7.),
        (-2., 7.),
    ]
    .into_iter()
    .map(|(x, y)| c + egui::vec2(x * sign, y))
    .collect();
    ui.painter().line(points, Stroke::new(1.6_f32, color));
    ui.painter().line(
        vec![
            c + egui::vec2(-8. * sign, -8.),
            c + egui::vec2(-8. * sign, -3.),
            c + egui::vec2(-3. * sign, -3.),
        ],
        Stroke::new(1.6_f32, color),
    );
    response
}
