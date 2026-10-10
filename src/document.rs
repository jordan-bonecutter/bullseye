use ab_glyph::{Font, FontRef, ScaleFont};
use egui::{Color32, Pos2, Rect};
use image::{Rgba, RgbaImage};
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Crop,
    Pen,
    Arrow,
    Rectangle,
    Ellipse,
    Highlight,
    Redact,
    Text,
    Pin,
}
impl Tool {
    pub const ALL: [Self; 9] = [
        Self::Crop,
        Self::Pen,
        Self::Arrow,
        Self::Rectangle,
        Self::Ellipse,
        Self::Highlight,
        Self::Redact,
        Self::Text,
        Self::Pin,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Crop => "Select region",
            Self::Pen => "Freehand",
            Self::Arrow => "Arrow",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Highlight => "Highlight",
            Self::Redact => "Redact",
            Self::Text => "Text",
            Self::Pin => "Numbered pin",
        }
    }
    pub fn shortcut(self) -> &'static str {
        match self {
            Self::Crop => "S",
            Self::Pen => "P",
            Self::Arrow => "A",
            Self::Rectangle => "R",
            Self::Ellipse => "E",
            Self::Highlight => "H",
            Self::Redact => "B",
            Self::Text => "T",
            Self::Pin => "N",
        }
    }
}
#[derive(Clone)]
pub struct Mark {
    pub tool: Tool,
    pub points: Vec<Pos2>,
    pub color: Color32,
    pub width: f32,
    pub text: String,
}
impl Mark {
    pub fn bounds(&self) -> Rect {
        Rect::from_two_pos(self.points[0], *self.points.last().unwrap())
    }
}
#[derive(Clone)]
struct Snapshot {
    crop: Option<Rect>,
    marks: Vec<Mark>,
}
pub struct Document {
    pub original: RgbaImage,
    pub crop: Option<Rect>,
    pub marks: Vec<Mark>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}
impl Document {
    pub fn new(original: RgbaImage) -> Self {
        Self {
            original,
            crop: None,
            marks: vec![],
            undo: vec![],
            redo: vec![],
        }
    }
    pub fn bounds(&self) -> Rect {
        Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(self.original.width() as f32, self.original.height() as f32),
        )
    }
    pub fn checkpoint(&mut self) {
        self.undo.push(Snapshot {
            crop: self.crop,
            marks: self.marks.clone(),
        });
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn add(&mut self, mark: Mark) {
        self.checkpoint();
        self.marks.push(mark);
    }
    pub fn set_crop(&mut self, rect: Rect) {
        let rect = rect.intersect(self.bounds());
        if rect.width() >= 2.0 && rect.height() >= 2.0 {
            self.checkpoint();
            self.crop = Some(rect);
        }
    }
    pub fn reset_crop(&mut self) {
        if self.crop.is_some() {
            self.checkpoint();
            self.crop = None;
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(Snapshot {
                crop: self.crop,
                marks: self.marks.clone(),
            });
            self.crop = s.crop;
            self.marks = s.marks;
        }
    }
    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(Snapshot {
                crop: self.crop,
                marks: self.marks.clone(),
            });
            self.crop = s.crop;
            self.marks = s.marks;
        }
    }
    pub fn rendered(&self, draft: Option<&Mark>) -> RgbaImage {
        let mut image = self.original.clone();
        for mark in self.marks.iter().chain(draft) {
            render_mark(&mut image, mark);
        }
        image
    }
    pub fn export(&self) -> RgbaImage {
        let image = self.rendered(None);
        let (x, y, width, height) = self.crop_pixels();
        image::imageops::crop_imm(&image, x, y, width, height).to_image()
    }
    pub fn crop_pixels(&self) -> (u32, u32, u32, u32) {
        let r = self.crop.unwrap_or(self.bounds()).intersect(self.bounds());
        let x = r.min.x.floor().max(0.0) as u32;
        let y = r.min.y.floor().max(0.0) as u32;
        let right = (r.max.x.ceil() as u32).min(self.original.width());
        let bottom = (r.max.y.ceil() as u32).min(self.original.height());
        (x, y, right - x, bottom - y)
    }
}
fn render_mark(image: &mut RgbaImage, mark: &Mark) {
    if mark.points.is_empty() {
        return;
    }
    let a = mark.points[0];
    let b = *mark.points.last().unwrap();
    let rect = mark.bounds();
    if mark.tool == Tool::Redact {
        // Cover every intersected pixel completely; antialiased edges can reveal content.
        let x = rect.min.x.floor().max(0.0) as u32;
        let y = rect.min.y.floor().max(0.0) as u32;
        let right = rect.max.x.ceil().max(0.0).min(image.width() as f32) as u32;
        let bottom = rect.max.y.ceil().max(0.0).min(image.height() as f32) as u32;
        for yy in y..bottom {
            for xx in x..right {
                image.put_pixel(xx, yy, Rgba([12, 16, 22, 255]));
            }
        }
        return;
    }
    if matches!(mark.tool, Tool::Text | Tool::Pin) {
        if mark.tool == Tool::Pin {
            let mut pb = PathBuilder::new();
            pb.push_circle(a.x, a.y, mark.width * 2.5 + 10.0);
            composite_path(image, pb.finish(), mark.color, 0.0, true);
            draw_text(
                image,
                &mark.text,
                Pos2::new(a.x - 6.0 * mark.text.len() as f32, a.y - 12.0),
                24.0,
                Color32::from_rgb(16, 25, 25),
            );
        } else {
            draw_text(image, &mark.text, a, mark.width * 3.0 + 16.0, mark.color);
        }
        return;
    }
    if matches!(mark.tool, Tool::Pen | Tool::Highlight)
        && mark.points.iter().all(|p| p.distance(a) < 0.01)
    {
        let width = mark.width * if mark.tool == Tool::Highlight { 6. } else { 1. };
        let mut pb = PathBuilder::new();
        pb.push_circle(a.x, a.y, width / 2.);
        let color = if mark.tool == Tool::Highlight {
            highlight_color(mark.color)
        } else {
            mark.color
        };
        composite_path(image, pb.finish(), color, width, true);
        return;
    }
    let mut pb = PathBuilder::new();
    match mark.tool {
        Tool::Pen | Tool::Highlight => {
            pb.move_to(a.x, a.y);
            for p in mark.points.iter().skip(1) {
                pb.line_to(p.x, p.y);
            }
        }
        Tool::Arrow => {
            pb.move_to(a.x, a.y);
            pb.line_to(b.x, b.y);
            let delta = b - a;
            if delta.length() > 0.0 {
                let unit = delta.normalized();
                let wing = egui::vec2(-unit.y, unit.x);
                let length = (mark.width * 4.0 + 10.0).min(delta.length() * 0.5);
                let left = b - unit * length + wing * length * 0.5;
                let right = b - unit * length - wing * length * 0.5;
                pb.move_to(left.x, left.y);
                pb.line_to(b.x, b.y);
                pb.line_to(right.x, right.y);
            }
        }
        Tool::Rectangle | Tool::Redact => {
            if let Some(r) =
                tiny_skia::Rect::from_xywh(rect.min.x, rect.min.y, rect.width(), rect.height())
            {
                pb.push_rect(r);
            }
        }
        Tool::Ellipse => {
            if let Some(r) =
                tiny_skia::Rect::from_xywh(rect.min.x, rect.min.y, rect.width(), rect.height())
            {
                pb.push_oval(r);
            }
        }
        _ => return,
    }
    let color = match mark.tool {
        Tool::Highlight => highlight_color(mark.color),
        Tool::Redact => Color32::from_rgb(12, 16, 22),
        _ => mark.color,
    };
    composite_path(
        image,
        pb.finish(),
        color,
        mark.width * if mark.tool == Tool::Highlight { 6. } else { 1. },
        matches!(mark.tool, Tool::Redact),
    );
}
fn composite_path(
    image: &mut RgbaImage,
    path: Option<tiny_skia::Path>,
    color: Color32,
    width: f32,
    fill: bool,
) {
    let Some(path) = path else {
        return;
    };
    // Only allocate the mark's bounding box; full-resolution strokes remain responsive on 4K displays.
    let bounds = path.bounds();
    let margin = width + 2.0;
    let x = (bounds.left() - margin).floor().max(0.0) as u32;
    let y = (bounds.top() - margin).floor().max(0.0) as u32;
    let right = (bounds.right() + margin)
        .ceil()
        .max(0.0)
        .min(image.width() as f32) as u32;
    let bottom = (bounds.bottom() + margin)
        .ceil()
        .max(0.0)
        .min(image.height() as f32) as u32;
    if right <= x || bottom <= y {
        return;
    }
    let mut pixmap = Pixmap::new(right - x, bottom - y).unwrap();
    let mut paint = Paint::default();
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    paint.set_color_rgba8(r, g, b, a);
    let transform = Transform::from_translate(-(x as f32), -(y as f32));
    if fill {
        pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None);
    } else {
        pixmap.stroke_path(
            &path,
            &paint,
            &Stroke {
                width,
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..Default::default()
            },
            transform,
            None,
        );
    }
    for yy in 0..pixmap.height() {
        for xx in 0..pixmap.width() {
            let p = pixmap.pixel(xx, yy).unwrap().demultiply();
            blend(
                image,
                x + xx,
                y + yy,
                [p.red(), p.green(), p.blue(), p.alpha()],
            );
        }
    }
}
fn blend(image: &mut RgbaImage, x: u32, y: u32, rgba: [u8; 4]) {
    if x >= image.width() || y >= image.height() {
        return;
    }
    let pixel = image.get_pixel_mut(x, y);
    let alpha = rgba[3] as u32;
    for (i, channel) in rgba.iter().take(3).enumerate() {
        pixel.0[i] = ((*channel as u32 * alpha + pixel.0[i] as u32 * (255 - alpha)) / 255) as u8;
    }
    pixel.0[3] = 255;
}
pub fn draw_text(image: &mut RgbaImage, text: &str, pos: Pos2, size: f32, color: Color32) {
    let [red, green, blue, alpha] = color.to_srgba_unmultiplied();
    let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).unwrap();
    let scaled = font.as_scaled(size);
    let mut cursor = ab_glyph::point(pos.x, pos.y + scaled.ascent());
    let mut previous = None;
    for c in text.chars() {
        if c == '\n' {
            cursor.x = pos.x;
            cursor.y += size * 1.3;
            previous = None;
            continue;
        }
        let id = scaled.glyph_id(c);
        if let Some(prev) = previous {
            cursor.x += scaled.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(size, cursor);
        cursor.x += scaled.h_advance(id);
        previous = Some(id);
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|x, y, coverage| {
                let x = bounds.min.x as i32 + x as i32;
                let y = bounds.min.y as i32 + y as i32;
                if x >= 0 && y >= 0 {
                    blend(
                        image,
                        x as u32,
                        y as u32,
                        [red, green, blue, (coverage * alpha as f32) as u8],
                    );
                }
            });
        }
    }
}

pub fn demo_image() -> RgbaImage {
    let mut image = RgbaImage::from_fn(1440, 900, |x, y| {
        let t = x as f32 / 1440.0;
        let u = y as f32 / 900.0;
        Rgba([
            (25.0 + 14.0 * t) as u8,
            (40.0 + 18.0 * (1.0 - u)) as u8,
            (50.0 + 20.0 * t) as u8,
            255,
        ])
    });
    let mut panel = |r: [f32; 4], c: Color32| {
        let mut p = PathBuilder::new();
        p.push_rect(tiny_skia::Rect::from_xywh(r[0], r[1], r[2], r[3]).unwrap());
        composite_path(&mut image, p.finish(), c, 0.0, true);
    };
    panel([0.0, 0.0, 1440.0, 40.0], Color32::from_rgb(17, 24, 32));
    panel([170.0, 140.0, 1100.0, 640.0], Color32::from_rgb(17, 24, 32));
    panel([170.0, 140.0, 1100.0, 48.0], Color32::from_rgb(31, 41, 51));
    panel([170.0, 188.0, 230.0, 592.0], Color32::from_rgb(21, 30, 39));
    panel([194.0, 290.0, 182.0, 40.0], Color32::from_rgb(36, 62, 65));
    panel([446.0, 374.0, 358.0, 156.0], Color32::from_rgb(26, 39, 49));
    panel([830.0, 374.0, 390.0, 156.0], Color32::from_rgb(26, 39, 49));
    panel([446.0, 560.0, 774.0, 160.0], Color32::from_rgb(26, 39, 49));
    let mint = Color32::from_rgb(128, 235, 201);
    let white = Color32::from_rgb(225, 233, 237);
    let gray = Color32::from_rgb(141, 159, 171);
    for (text, x, y, size, color) in [
        ("sway", 24., 10., 20., mint),
        ("1   2   3", 660., 10., 18., gray),
        ("09:41", 1350., 10., 18., white),
        ("Workspace / Overview", 206., 153., 21., white),
        ("STUDIO", 200., 222., 18., mint),
        ("Overview", 212., 299., 20., white),
        ("Projects", 212., 359., 20., gray),
        ("Notes", 212., 411., 20., gray),
        ("Settings", 212., 463., 20., gray),
        ("A little room to create.", 446., 236., 38., white),
        (
            "Your ideas, your pace. Make something worth sharing.",
            446.,
            297.,
            21.,
            gray,
        ),
        ("IN PROGRESS", 470., 399., 16., mint),
        ("Screenshot workflow", 470., 444., 27., white),
        ("Small tools. Thoughtful details.", 470., 489., 18., gray),
        ("THIS WEEK", 854., 399., 16., mint),
        ("Less friction, more flow.", 854., 444., 27., white),
        ("Built for the way you work.", 854., 489., 18., gray),
        ("A note to self", 470., 585., 24., white),
        (
            "The best tools disappear when you need them",
            470.,
            632.,
            23.,
            gray,
        ),
        ("and feel good when you use them.", 470., 668., 23., gray),
    ] {
        draw_text(&mut image, text, Pos2::new(x, y), size, color);
    }
    image
}

pub fn highlight_color(color: Color32) -> Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, (u16::from(a) * 75 / 255) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc() -> Document {
        Document::new(RgbaImage::from_pixel(100, 80, Rgba([255, 255, 255, 255])))
    }
    #[test]
    fn reverse_crop_clamps_and_rounds_outward() {
        let mut d = doc();
        d.set_crop(Rect::from_two_pos(
            Pos2::new(130.0, 60.2),
            Pos2::new(10.7, -2.0),
        ));
        assert_eq!(d.crop_pixels(), (10, 0, 90, 61));
        assert_eq!(d.export().dimensions(), (90, 61));
    }
    #[test]
    fn undo_redo_restores_crop_and_marks_and_new_edit_clears_redo() {
        let mut d = doc();
        d.set_crop(Rect::from_min_max(Pos2::new(10., 10.), Pos2::new(50., 50.)));
        d.add(Mark {
            tool: Tool::Redact,
            points: vec![Pos2::new(15., 15.), Pos2::new(25., 25.)],
            color: Color32::RED,
            width: 3.,
            text: String::new(),
        });
        assert_eq!(d.export().get_pixel(7, 7).0, [12, 16, 22, 255]);
        d.undo();
        assert!(d.marks.is_empty());
        d.undo();
        assert!(d.crop.is_none());
        d.redo();
        assert!(d.crop.is_some());
        d.redo();
        assert_eq!(d.marks.len(), 1);
        d.undo();
        d.reset_crop();
        assert!(!d.can_redo());
    }
    #[test]
    fn fractional_redaction_covers_edge_pixels_completely() {
        let mut d = doc();
        d.add(Mark {
            tool: Tool::Redact,
            points: vec![Pos2::new(10.8, 11.2), Pos2::new(20.1, 21.9)],
            color: Color32::RED,
            width: 3.0,
            text: String::new(),
        });
        let image = d.export();
        assert_eq!(image.get_pixel(10, 11).0, [12, 16, 22, 255]);
        assert_eq!(image.get_pixel(20, 21).0, [12, 16, 22, 255]);
        assert_eq!(image.get_pixel(9, 11).0, [255; 4]);
    }
    #[test]
    fn translucent_pen_and_highlighter_keep_the_selected_rgb() {
        for (tool, alpha, expected) in [(Tool::Pen, 128, 127), (Tool::Highlight, 255, 180)] {
            let mut d = doc();
            d.add(Mark {
                tool,
                points: vec![Pos2::new(50., 50.); 3],
                color: Color32::from_rgba_unmultiplied(255, 0, 0, alpha),
                width: 4.,
                text: String::new(),
            });
            let image = d.export();
            let [r, g, b, a] = image.get_pixel(50, 50).0;
            assert_eq!((r, a), (255, 255));
            assert!((i32::from(g) - expected).abs() <= 1 && (i32::from(b) - expected).abs() <= 1);
        }
    }
    #[test]
    fn exported_text_and_arrow_change_pixels() {
        let mut d = doc();
        for tool in [Tool::Text, Tool::Arrow] {
            d.add(Mark {
                tool,
                points: vec![Pos2::new(5., 5.), Pos2::new(80., 65.)],
                color: Color32::RED,
                width: 3.,
                text: "Hi".into(),
            });
        }
        assert!(d.export().pixels().any(|p| p.0 != [255; 4]));
    }
}
