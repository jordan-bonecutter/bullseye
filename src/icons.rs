//! Native vector adaptations of Lucide icons (assets/icons/LICENSE).
//! All symbols share a 24-unit grid, rounded 2-unit strokes, and a 20-point footprint.
use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

#[derive(Clone, Copy)]
pub enum Icon {
    Crop,
    Pencil,
    Arrow,
    Rectangle,
    Ellipse,
    Highlighter,
    Redact,
    Text,
    Pin,
    Undo,
    Redo,
    Copy,
    Save,
    Close,
}

pub fn paint(painter: &Painter, center: Pos2, icon: Icon, color: Color32) {
    let pen = Pen {
        painter,
        origin: center - Vec2::splat(10.),
        color,
    };
    match icon {
        Icon::Crop => {
            pen.path(&[(6., 2.), (6., 16.)])
                .curve((6., 17.1), (6.9, 18.), (8., 18.))
                .line((22., 18.))
                .finish();
            pen.path(&[(18., 22.), (18., 8.)])
                .curve((18., 6.9), (17.1, 6.), (16., 6.))
                .line((2., 6.))
                .finish();
        }
        Icon::Pencil => {
            pen.path(&[
                (16., 3.),
                (21., 8.),
                (8., 21.),
                (2., 22.),
                (3., 16.),
                (16., 3.),
            ])
            .finish();
            pen.path(&[(14., 5.), (19., 10.)]).finish();
            pen.path(&[(3., 16.), (8., 21.)]).finish();
        }
        Icon::Arrow => {
            pen.path(&[(13., 5.), (19., 5.), (19., 11.)]).finish();
            pen.path(&[(19., 5.), (5., 19.)]).finish();
        }
        Icon::Rectangle => pen.rect((2., 6.), (22., 18.), 2.),
        Icon::Ellipse => {
            painter.add(egui::Shape::ellipse_stroke(
                center,
                Vec2::new(8.3, 6.7),
                pen.stroke(),
            ));
        }
        Icon::Highlighter => {
            pen.path(&[(9., 11.), (3., 17.), (3., 20.), (12., 20.), (15., 17.)])
                .finish();
            pen.path(&[(22., 12.), (17.4, 16.6)])
                .curve((16.6, 17.4), (15.4, 17.4), (14.6, 16.6))
                .line((9.4, 11.4))
                .curve((8.6, 10.6), (8.6, 9.4), (9.4, 8.6))
                .line((14., 4.))
                .finish();
            pen.path(&[(3., 23.), (12., 23.)]).finish();
        }
        Icon::Redact => {
            pen.rect((2., 5.), (22., 19.), 2.);
            // Solid bars communicate opaque masking rather than a shape tool.
            for y in [9., 14.] {
                painter.rect_filled(
                    Rect::from_min_max(pen.point((5., y - 1.)), pen.point((19., y + 1.))),
                    0.5,
                    color,
                );
            }
        }
        Icon::Text => {
            pen.path(&[(12., 4.), (12., 20.)]).finish();
            pen.path(&[(4., 7.), (4., 5.)])
                .curve((4., 4.45), (4.45, 4.), (5., 4.))
                .line((19., 4.))
                .curve((19.55, 4.), (20., 4.45), (20., 5.))
                .line((20., 7.))
                .finish();
            pen.path(&[(9., 20.), (15., 20.)]).finish();
        }
        Icon::Pin => {
            painter.circle_stroke(center, 8.3, pen.stroke());
            pen.path(&[(10., 9.), (12., 7.5), (12., 16.5)]).finish();
            pen.path(&[(9.5, 16.5), (14.5, 16.5)]).finish();
        }
        Icon::Undo | Icon::Redo => {
            let flip = |(x, y)| {
                if matches!(icon, Icon::Redo) {
                    (24. - x, y)
                } else {
                    (x, y)
                }
            };
            pen.path(&[flip((9., 14.)), flip((4., 9.)), flip((9., 4.))])
                .finish();
            pen.path(&[flip((4., 9.)), flip((14.5, 9.))])
                .curve(flip((17.54, 9.)), flip((20., 11.46)), flip((20., 14.5)))
                .curve(flip((20., 17.54)), flip((17.54, 20.)), flip((14.5, 20.)))
                .line(flip((11., 20.)))
                .finish();
        }
        Icon::Copy => {
            pen.rect((8., 8.), (22., 22.), 2.);
            pen.path(&[(4., 16.)])
                .curve((2.9, 16.), (2., 15.1), (2., 14.))
                .line((2., 4.))
                .curve((2., 2.9), (2.9, 2.), (4., 2.))
                .line((14., 2.))
                .curve((15.1, 2.), (16., 2.9), (16., 4.))
                .finish();
        }
        Icon::Save => {
            pen.path(&[(12., 15.), (12., 3.)]).finish();
            pen.path(&[(21., 15.), (21., 19.)])
                .curve((21., 20.1), (20.1, 21.), (19., 21.))
                .line((5., 21.))
                .curve((3.9, 21.), (3., 20.1), (3., 19.))
                .line((3., 15.))
                .finish();
            pen.path(&[(7., 10.), (12., 15.), (17., 10.)]).finish();
        }
        Icon::Close => {
            pen.path(&[(18., 6.), (6., 18.)]).finish();
            pen.path(&[(6., 6.), (18., 18.)]).finish();
        }
    }
}

struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    color: Color32,
}
impl Pen<'_> {
    fn point(&self, (x, y): (f32, f32)) -> Pos2 {
        self.origin + Vec2::new(x, y) * (20. / 24.)
    }
    fn stroke(&self) -> Stroke {
        Stroke::new(20.0_f32 / 12., self.color)
    }
    fn rect(&self, min: (f32, f32), max: (f32, f32), radius: f32) {
        self.painter.rect_stroke(
            Rect::from_min_max(self.point(min), self.point(max)),
            radius * (20. / 24.),
            self.stroke(),
            StrokeKind::Middle,
        );
    }
    fn path(&self, points: &[(f32, f32)]) -> Path<'_, '_> {
        Path {
            pen: self,
            points: points.iter().map(|&p| self.point(p)).collect(),
        }
    }
}
struct Path<'a, 'b> {
    pen: &'a Pen<'b>,
    points: Vec<Pos2>,
}
impl Path<'_, '_> {
    fn line(mut self, point: (f32, f32)) -> Self {
        self.points.push(self.pen.point(point));
        self
    }
    fn curve(mut self, a: (f32, f32), b: (f32, f32), end: (f32, f32)) -> Self {
        let start = *self.points.last().expect("curve starts on a path");
        let [a, b, end] = [a, b, end].map(|p| self.pen.point(p));
        // Sample in logical coordinates; egui antialiases the resulting vector at the output scale.
        for step in 1..=12 {
            let t = step as f32 / 12.;
            let u = 1. - t;
            self.points.push(
                (start.to_vec2() * u.powi(3)
                    + a.to_vec2() * (3. * u * u * t)
                    + b.to_vec2() * (3. * u * t * t)
                    + end.to_vec2() * t.powi(3))
                .to_pos2(),
            );
        }
        self
    }
    fn finish(self) {
        let stroke = self.pen.stroke();
        // Explicit round caps and joins keep diagonals and corners consistent with Lucide.
        for &point in &self.points {
            self.pen
                .painter
                .circle_filled(point, stroke.width / 2., stroke.color);
        }
        self.pen.painter.add(egui::Shape::line(self.points, stroke));
    }
}
