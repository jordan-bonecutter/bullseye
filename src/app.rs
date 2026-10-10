//! A selection and a small toolbar, drawn directly over the frozen desktop.
use crate::document::{Document, Mark, Tool, highlight_color};
use crate::fonts::{Catalog, TextFont};
use egui::emath::GuiRounding;
use egui::{Align2, Color32, Context, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use image::RgbaImage;
use std::{collections::BTreeMap, path::PathBuf};

const ACCENT: Color32 = Color32::from_rgb(113, 175, 255);
const INK: Color32 = Color32::from_rgb(236, 239, 243);
const BAR: Color32 = Color32::from_rgb(30, 33, 38);

struct FontPreview {
    texture: egui::TextureHandle,
    size: Vec2,
    pixels_per_point: f32,
    last_used: u64,
}
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
    font_size: f32,
    font: TextFont,
    font_search: String,
    font_definitions: egui::FontDefinitions,
    font_previews: BTreeMap<String, FontPreview>,
    preview_clock: u64,
    draft: Option<Mark>,
    moving: Option<(Rect, Pos2)>,
    text_anchor: Option<Pos2>,
    text: String,
    frozen: Option<egui::TextureHandle>,
    native_frozen: Option<egui::TextureId>,
    save_request: bool,
    picker: Option<std::sync::mpsc::Receiver<anyhow::Result<Option<PathBuf>>>>,
    save_dialog: bool,
    destination: String,
    explicit_destination: bool,
    error: Option<String>,
}
impl OverlayUi {
    pub fn new(image: RgbaImage, destination: PathBuf, explicit_destination: bool) -> Self {
        let ctx = Context::default();
        // The selection overlay needs only a short hint; rasterize other glyphs when first used.
        ctx.options_mut(|options| options.preload_font_glyphs = false);
        let font = TextFont::default();
        let mut font_definitions = egui::FontDefinitions::default();
        font.install(&ctx, &mut font_definitions);
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
            font_size: 28.,
            font,
            font_search: String::new(),
            font_definitions,
            font_previews: BTreeMap::new(),
            preview_clock: 0,
            draft: None,
            moving: None,
            text_anchor: None,
            text: String::new(),
            frozen: None,
            native_frozen: None,
            save_request: false,
            picker: None,
            save_dialog: false,
            destination: destination.display().to_string(),
            explicit_destination,
            error: None,
        }
    }
    pub fn pending_text(&self) -> Option<&str> {
        self.text_anchor.map(|_| self.text.as_str())
    }
    pub fn set_frozen_texture(&mut self, texture: egui::TextureId) {
        self.native_frozen = Some(texture);
    }
    pub fn font_size(&self) -> f32 {
        self.font_size
    }
    pub fn selected_font(&self) -> &str {
        &self.font.name
    }
    pub fn color(&self) -> [u8; 4] {
        self.color.to_srgba_unmultiplied()
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
        self.commit_text();
        match crate::save_png(&self.doc.export(), std::path::Path::new(&self.destination)) {
            Ok(()) => self.done = true,
            Err(e) => self.error = Some(format!("Save failed: {e:#}")),
        }
    }
    fn copy(&mut self) {
        self.commit_text();
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
                font_size: self.font_size,
                text: self.text.clone(),
                font: self.font.clone(),
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
        self.poll_picker();
        self.gesture(&raw.events, logical, active);
        ctx.run(raw, |ctx| {
            let key = |k| ctx.input(|i| i.key_pressed(k));
            if key(egui::Key::Escape) {
                if egui::Popup::is_any_open(ctx) {
                    egui::Popup::close_all(ctx);
                } else if self.save_dialog {
                    self.save_dialog = false;
                    self.error = None;
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
                            self.save_request = true;
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
                } else if self.text_anchor.is_none() {
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
                    self.paint_desktop(ui.painter(), logical, active);
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
        let caption_input =
            !self.ctx.wants_keyboard_input() && !egui::Popup::is_any_open(&self.ctx);
        for event in events {
            match event {
                egui::Event::Text(text) | egui::Event::Paste(text)
                    if self.text_anchor.is_some() && caption_input =>
                {
                    self.text.push_str(text)
                }
                egui::Event::Key {
                    key: egui::Key::Backspace,
                    pressed: true,
                    ..
                } if self.text_anchor.is_some() && caption_input => {
                    self.text.pop();
                }
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    ..
                } if self.text_anchor.is_some() && caption_input => self.commit_text(),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => {
                    if self.save_dialog
                        || self.picker.is_some()
                        || self
                            .ctx
                            .layer_id_at(*pos)
                            .is_some_and(|layer| layer.order >= egui::Order::Foreground)
                    {
                        continue;
                    }
                    self.commit_text();
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

                        continue;
                    }
                    if tool == Tool::Pin {
                        self.doc.add(Mark {
                            tool,
                            font: self.font.clone(),
                            points: vec![p],
                            color: self.color,
                            width: self.width,
                            font_size: self.font_size,
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
                        font: self.font.clone(),
                        points: vec![corner.map(|(_, opposite)| opposite).unwrap_or(p), p],
                        color: self.color,
                        width: self.width,
                        font_size: self.font_size,
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
                        } else if matches!(mark.tool, Tool::Pen | Tool::Highlight) {
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
                            if matches!(mark.tool, Tool::Pen | Tool::Highlight) {
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
                        } else if matches!(mark.tool, Tool::Pen | Tool::Highlight)
                            || mark.points.len() > 3
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
    pub fn fallback_dialog(&self) -> bool {
        self.save_dialog
    }
    pub fn picker_active(&self) -> bool {
        self.picker.is_some()
    }
    pub fn start_picker(&mut self) {
        if !self.save_request || self.picker.is_some() {
            return;
        }
        self.save_request = false;
        self.commit_text();
        let path = PathBuf::from(&self.destination);
        let (tx, rx) = std::sync::mpsc::channel();
        self.picker = Some(rx);
        std::thread::spawn(move || {
            let file = crate::file_picker::save(&path);
            let _ = tx.send(file);
        });
    }
    pub fn poll_picker(&mut self) {
        if let Some(rx) = &self.picker {
            match rx.try_recv() {
                Ok(Ok(Some(path))) => {
                    self.picker = None;
                    self.destination = path.display().to_string();
                    self.save();
                }
                Ok(Ok(None)) => {
                    self.picker = None;
                }
                Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.picker = None;
                    self.save_dialog = true;
                    self.error = Some("System file picker unavailable; enter a PNG path.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    self.ctx
                        .request_repaint_after(std::time::Duration::from_millis(30));
                }
            }
        }
    }
    fn paint_desktop(&mut self, painter: &egui::Painter, logical: Vec2, active: bool) {
        let texture = self.native_frozen.unwrap_or_else(|| {
            self.frozen
                .get_or_insert_with(|| {
                    self.ctx.load_texture(
                        "frozen desktop",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [
                                self.doc.original.width() as usize,
                                self.doc.original.height() as usize,
                            ],
                            self.doc.original.as_raw(),
                        ),
                        egui::TextureOptions::LINEAR,
                    )
                })
                .id()
        });
        let screen = Rect::from_min_size(Pos2::ZERO, logical);
        painter.image(
            texture,
            screen,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
            Color32::WHITE,
        );
        let scale = self.doc.bounds().size() / logical;
        if active {
            for mark in self.doc.marks.iter().chain(self.draft.as_ref()) {
                paint_mark(painter, mark, scale);
            }
            if let Some(anchor) = self.text_anchor {
                let position = (anchor.to_vec2() / scale).to_pos2();
                let font = self.font.font_id(self.font_size / scale.x);
                let galley = painter.layout_no_wrap(self.text.clone(), font, self.color);
                let caret = galley
                    .pos_from_cursor(egui::text::CCursor::new(self.text.chars().count()))
                    .translate(position.to_vec2() + Vec2::new(1., 0.));
                painter.galley(position, galley, self.color);
                // Draw the caret as geometry: font fallback must never turn it into a missing glyph.
                painter.line_segment(
                    [caret.left_top(), caret.left_bottom()],
                    Stroke::new(1.0_f32, self.color),
                );
            }
        }
        let shade = Color32::from_black_alpha(120);
        if let Some(crop) = self.visible_crop().filter(|_| active) {
            let r = Rect::from_min_max(
                (crop.min.to_vec2() / scale).to_pos2(),
                (crop.max.to_vec2() / scale).to_pos2(),
            );
            for area in [
                Rect::from_min_max(screen.min, Pos2::new(screen.max.x, r.min.y)),
                Rect::from_min_max(Pos2::new(0., r.max.y), screen.max),
                Rect::from_min_max(Pos2::new(0., r.min.y), Pos2::new(r.min.x, r.max.y)),
                Rect::from_min_max(
                    Pos2::new(r.max.x, r.min.y),
                    Pos2::new(screen.max.x, r.max.y),
                ),
            ] {
                painter.rect_filled(area, 0, shade);
            }
        } else {
            painter.rect_filled(screen, 0, shade);
        }
    }
    fn font_preview(&mut self, ui: &egui::Ui, choice: &crate::fonts::FontChoice, rect: Rect) {
        let ppp = ui.ctx().pixels_per_point();
        if self
            .font_previews
            .get(&choice.key)
            .is_some_and(|p| p.pixels_per_point != ppp)
        {
            self.font_previews.remove(&choice.key);
        }
        self.preview_clock += 1;
        if !self.font_previews.contains_key(&choice.key)
            && let Some(font) = Catalog::system().load(choice)
        {
            if self.font_previews.len() >= 64 {
                let oldest = self
                    .font_previews
                    .iter()
                    .min_by_key(|(_, p)| p.last_used)
                    .map(|(key, _)| key.clone())
                    .unwrap();
                self.font_previews.remove(&oldest);
            }
            // Two samples per native pixel preserve fine strokes during GPU downsampling.
            let size = Vec2::new(
                (font.preview_width(20.) * ppp).ceil() / ppp,
                (28. * ppp).ceil() / ppp,
            );
            let resolution =
                (2. * ppp).min(ui.ctx().input(|i| i.max_texture_side) as f32 / size.x.max(size.y));
            let image = font.preview(
                [
                    (size.x * resolution).ceil() as usize,
                    (size.y * resolution).ceil() as usize,
                ],
                20. * resolution,
                INK,
            );
            let texture = ui.ctx().load_texture(
                format!("font-preview-{}", choice.key),
                image,
                egui::TextureOptions::LINEAR,
            );
            self.font_previews.insert(
                choice.key.clone(),
                FontPreview {
                    texture,
                    size,
                    pixels_per_point: ppp,
                    last_used: self.preview_clock,
                },
            );
        }
        if let Some(preview) = self.font_previews.get_mut(&choice.key) {
            preview.last_used = self.preview_clock;
            ui.painter().image(
                preview.texture.id(),
                Rect::from_min_size(
                    (rect.min + Vec2::new(4., 2.)).round_to_pixels(ppp),
                    preview.size,
                ),
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1., 1.)),
                Color32::WHITE,
            );
        } else {
            ui.painter().text(
                rect.left_center() + Vec2::new(4., 0.),
                Align2::LEFT_CENTER,
                &choice.name,
                FontId::proportional(14.),
                INK,
            );
        }
    }
    fn font_selector(&mut self, ui: &mut egui::Ui) {
        let button = ui.add_sized(
            Vec2::new(130., 29.),
            egui::Button::new(&self.font.name).truncate(),
        );
        self.hits.insert("font".into(), button.rect);
        let popup = egui::Popup::menu(&button);
        let width = if popup.is_open() || button.clicked() {
            Catalog::system().picker_width()
        } else {
            440.
        }
        .min((ui.ctx().screen_rect().width() - 24.).max(1.));
        popup
            .width(width)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.font_search)
                        .id_salt("font-search")
                        .desired_width(ui.available_width())
                        .hint_text("Find a font…"),
                );
                if button.clicked() {
                    search.request_focus();
                }
                self.hits.insert("font-search".into(), search.rect);
                let mut results = egui::ScrollArea::vertical()
                    .id_salt("font-results")
                    .max_height(220.)
                    .auto_shrink([false, false]);
                if button.clicked() || search.changed() {
                    results = results.vertical_scroll_offset(0.);
                }
                let results = results.show(ui, |ui| {
                    let catalog = Catalog::system();
                    let search = self.font_search.to_lowercase();
                    for choice in &catalog.choices {
                        if !choice.name.to_lowercase().contains(&search) {
                            continue;
                        }
                        let (rect, response) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 32.),
                            Sense::click(),
                        );
                        if ui.is_rect_visible(rect) {
                            self.hits.insert(format!("font-{}", choice.key), rect);
                            if response.hovered() || self.font.key == choice.key {
                                ui.painter().rect_filled(
                                    rect,
                                    3,
                                    if self.font.key == choice.key {
                                        ACCENT.gamma_multiply(0.25)
                                    } else {
                                        Color32::from_gray(48)
                                    },
                                );
                            }
                            self.font_preview(ui, choice, rect);
                        }
                        if response.clicked() {
                            if let Some(font) = catalog.load(choice) {
                                font.install(&self.ctx, &mut self.font_definitions);
                                self.font = font;
                                self.font_search.clear();
                                self.ctx.request_repaint();
                                ui.close();
                            } else {
                                self.error =
                                    Some("This font cannot be loaded for annotations.".into());
                            }
                        }
                    }
                });
                self.hits.insert("font-results".into(), results.inner_rect);
            });
    }
    fn toolbar(&mut self, ctx: &Context, logical: Vec2, scale: Vec2) {
        let crop = self.doc.crop.unwrap();
        let r = Rect::from_min_max(
            (crop.min.to_vec2() / scale).to_pos2(),
            (crop.max.to_vec2() / scale).to_pos2(),
        );
        let width: f32 = if self.tool == Tool::Text { 712. } else { 500. };
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
                                    self.commit_text();
                                }
                                response.on_hover_text(format!(
                                    "{} ({})",
                                    tool.label(),
                                    tool.shortcut()
                                ));
                            }
                            if self.tool == Tool::Text {
                                self.font_selector(ui);
                                let size = ui.add_sized(
                                    Vec2::new(64., 29.),
                                    egui::DragValue::new(&mut self.font_size)
                                        .range(8.0..=256.0)
                                        .speed(1.)
                                        .suffix(" px"),
                                );
                                self.hits.insert("font-size".into(), size.rect);
                                size.on_hover_text(
                                    "Text size in screenshot pixels. Drag or click to type.",
                                );
                            }
                            ui.separator();
                            let palette = ui.menu_button(" ", |ui| {
                                ui.spacing_mut().slider_width = 220.;
                                egui::color_picker::color_picker_color32(
                                    ui,
                                    &mut self.color,
                                    egui::color_picker::Alpha::OnlyBlend,
                                );
                                self.hits.insert("color-picker".into(), ui.min_rect());
                                ui.add(
                                    egui::Slider::new(&mut self.width, 2.0..=16.0)
                                        .text("Stroke")
                                        .integer(),
                                );
                            });
                            self.hits.insert("color".into(), palette.response.rect);
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
                                                self.save_request = true;
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
            5,
            if selected {
                ACCENT.gamma_multiply(0.18)
            } else {
                Color32::from_gray(48)
            },
        );
    }
    let icon = match tool {
        Tool::Crop => crate::icons::Icon::Crop,
        Tool::Pen => crate::icons::Icon::Pencil,
        Tool::Arrow => crate::icons::Icon::Arrow,
        Tool::Rectangle => crate::icons::Icon::Rectangle,
        Tool::Ellipse => crate::icons::Icon::Ellipse,
        Tool::Highlight => crate::icons::Icon::Highlighter,
        Tool::Redact => crate::icons::Icon::Redact,
        Tool::Text => crate::icons::Icon::Text,
        Tool::Pin => crate::icons::Icon::Pin,
    };
    crate::icons::paint(
        ui.painter(),
        r.center(),
        icon,
        if selected { ACCENT } else { INK },
    );
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
        ui.painter().rect_filled(r, 5, Color32::from_gray(48));
    }
    let icon = match name {
        "undo" => crate::icons::Icon::Undo,
        "redo" => crate::icons::Icon::Redo,
        "copy" => crate::icons::Icon::Copy,
        "save" => crate::icons::Icon::Save,
        "close" => crate::icons::Icon::Close,
        _ => unreachable!("unknown toolbar action"),
    };
    crate::icons::paint(
        ui.painter(),
        r.center(),
        icon,
        if enabled {
            INK
        } else {
            Color32::from_gray(100)
        },
    );
    response
}

fn annotation_font(size: f32) -> FontId {
    TextFont::default().font_id(size)
}

fn paint_mark(p: &egui::Painter, mark: &Mark, scale: Vec2) {
    if mark.points.is_empty() || mark.tool == Tool::Crop {
        return;
    }
    let point = |v: Pos2| (v.to_vec2() / scale).to_pos2();
    let a = point(mark.points[0]);
    let b = point(*mark.points.last().unwrap());
    let rect = Rect::from_two_pos(a, b);
    let color = if mark.tool == Tool::Highlight {
        highlight_color(mark.color)
    } else {
        mark.color
    };
    let width = mark.width * if mark.tool == Tool::Highlight { 6. } else { 1. } / scale.x;
    let stroke = Stroke::new(width, color);
    match mark.tool {
        Tool::Pen | Tool::Highlight => {
            let points: Vec<_> = mark.points.iter().copied().map(point).collect();
            // One continuous mesh avoids dark overlaps along translucent highlighter strokes.
            if points.iter().all(|v| v.distance(a) < 0.01) {
                p.circle_filled(a, width / 2., color);
            } else {
                p.add(egui::Shape::line(points, stroke));
            }
        }
        Tool::Arrow => {
            p.line_segment([a, b], stroke);
            let delta = b - a;
            if delta.length() > 0. {
                let unit = delta.normalized();
                let wing = egui::vec2(-unit.y, unit.x);
                let length = ((mark.width * 4. + 10.) / scale.x).min(delta.length() * 0.5);
                p.add(egui::Shape::line(
                    vec![
                        b - unit * length + wing * length * 0.5,
                        b,
                        b - unit * length - wing * length * 0.5,
                    ],
                    stroke,
                ));
            }
        }
        Tool::Rectangle => {
            p.rect_stroke(rect, 0, stroke, StrokeKind::Middle);
        }
        Tool::Ellipse => {
            p.add(egui::epaint::EllipseShape::stroke(
                rect.center(),
                rect.size() / 2.,
                stroke,
            ));
        }
        Tool::Redact => {
            p.rect_filled(rect, 0, Color32::from_rgb(12, 16, 22));
        }
        Tool::Text => {
            p.text(
                a,
                Align2::LEFT_TOP,
                &mark.text,
                mark.font.font_id(mark.font_size / scale.x),
                color,
            );
        }
        Tool::Pin => {
            p.circle_filled(a, (mark.width * 2.5 + 10.) / scale.x, color);
            p.text(
                a,
                Align2::CENTER_CENTER,
                &mark.text,
                annotation_font(24. / scale.x),
                Color32::from_rgb(16, 25, 25),
            );
        }
        Tool::Crop => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_reuses_the_frozen_gpu_texture() {
        let mut ui = OverlayUi::new(RgbaImage::new(1440, 900), PathBuf::from("unused"), true);
        let first = ui.frame(egui::RawInput::default(), Vec2::new(1440., 900.), true);
        let frozen = first
            .textures_delta
            .set
            .iter()
            .find(|(_, delta)| delta.image.size() == [1440, 900])
            .unwrap()
            .0;
        for x in [400., 600., 800.] {
            let pos = egui::pos2(x, 400.);
            let events = if x == 400. {
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }]
            } else {
                vec![egui::Event::PointerMoved(pos)]
            };
            let output = ui.frame(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                Vec2::new(1440., 900.),
                true,
            );
            assert!(
                output
                    .textures_delta
                    .set
                    .iter()
                    .all(|(id, _)| *id != frozen),
                "Dragging must never re-upload the screenshot"
            );
        }
    }
    #[test]
    fn a_stationary_batched_pen_click_is_kept() {
        let mut ui = OverlayUi::new(
            RgbaImage::from_pixel(1440, 900, image::Rgba([255; 4])),
            PathBuf::from("unused"),
            true,
        );
        ui.doc.set_crop(Rect::from_min_max(
            egui::pos2(300., 200.),
            egui::pos2(1000., 600.),
        ));
        ui.tool = Tool::Pen;
        let pos = egui::pos2(500., 350.);
        let events = [true, false]
            .into_iter()
            .map(|pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            })
            .collect();
        ui.frame(
            egui::RawInput {
                events,
                ..Default::default()
            },
            Vec2::new(1440., 900.),
            true,
        );
        assert_eq!(ui.doc.marks.len(), 1);
        assert_ne!(ui.doc.export().get_pixel(200, 150).0, [255; 4]);
    }
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
        ui.frame(raw(), Vec2::new(1440., 900.), true);
        let second = ui.frame(raw(), Vec2::new(1440., 900.), true);
        assert!(
            !ui.ctx
                .tessellate(second.shapes, second.pixels_per_point)
                .is_empty()
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
