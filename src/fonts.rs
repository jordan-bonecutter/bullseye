//! Font discovery and shared caption font data. No external font helper is needed.
use ab_glyph::{Font, FontRef};
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
pub struct TextFont {
    pub key: String,
    pub name: String,
    data: Arc<[u8]>,
    index: u32,
}
impl Default for TextFont {
    fn default() -> Self {
        static FONT: OnceLock<TextFont> = OnceLock::new();
        FONT.get_or_init(|| {
            Self::bundled(
                "Ubuntu-Light",
                "Ubuntu Light",
                epaint_default_fonts::UBUNTU_LIGHT,
            )
        })
        .clone()
    }
}
impl TextFont {
    pub fn preview_width(&self, height: f32) -> f32 {
        label_width(self.face(), &self.name, height)
    }
    fn bundled(key: &str, name: &str, bytes: &[u8]) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            data: Arc::from(bytes),
            index: 0,
        }
    }
    pub fn face(&self) -> FontRef<'_> {
        FontRef::try_from_slice_and_index(&self.data, self.index)
            .expect("Caption fonts are validated when loaded")
    }
    pub fn font_id(&self, pixel_height: f32) -> egui::FontId {
        let face = self.face();
        egui::FontId::new(
            pixel_height * face.units_per_em().unwrap_or(face.height_unscaled())
                / face.height_unscaled(),
            egui::FontFamily::Name(self.key.clone().into()),
        )
    }
    /// Rasterize a small label once; font-picker scrolling never rebuilds the annotation font atlas.
    pub fn preview(&self, size: [usize; 2], height: f32, color: egui::Color32) -> egui::ColorImage {
        use ab_glyph::ScaleFont;
        let mut image = egui::ColorImage::filled(size, egui::Color32::TRANSPARENT);
        let face = self.face();
        let fallback = TextFont::default();
        let fallback_face = fallback.face();
        let scaled = face.as_scaled(height);
        let fallback_scaled = fallback_face.as_scaled(height);
        let mut x = height * 0.1;
        let [r, g, b, alpha] = color.to_srgba_unmultiplied();
        for c in self.name.chars() {
            if x >= size[0] as f32 {
                break;
            }
            let (font, id) = if scaled.glyph_id(c).0 != 0 {
                (&scaled, scaled.glyph_id(c))
            } else {
                (&fallback_scaled, fallback_scaled.glyph_id(c))
            };
            let glyph = id.with_scale_and_position(height, ab_glyph::point(x, font.ascent()));
            x += font.h_advance(id);
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|px, py, coverage| {
                    let px = px as i32 + bounds.min.x as i32;
                    let py = py as i32 + bounds.min.y as i32;
                    if px >= 0 && py >= 0 && (px as usize) < size[0] && (py as usize) < size[1] {
                        let pixel = &mut image.pixels[py as usize * size[0] + px as usize];
                        let source = (coverage * f32::from(alpha)).round() as u8;
                        let combined = u16::from(source)
                            + u16::from(pixel.a()) * (255 - u16::from(source)) / 255;
                        *pixel = egui::Color32::from_rgba_unmultiplied(r, g, b, combined as u8);
                    }
                });
            }
        }
        image
    }
    pub fn install(&self, ctx: &egui::Context, definitions: &mut egui::FontDefinitions) {
        if !definitions.font_data.contains_key(&self.key) {
            let mut data = egui::FontData::from_owned(self.data.to_vec());
            data.index = self.index;
            definitions
                .font_data
                .insert(self.key.clone(), Arc::new(data));
        }
        definitions.families.insert(
            egui::FontFamily::Name(self.key.clone().into()),
            vec![self.key.clone()],
        );
        ctx.set_fonts(definitions.clone());
    }
}

fn label_width(face: FontRef<'_>, name: &str, height: f32) -> f32 {
    use ab_glyph::ScaleFont;
    let fallback = TextFont::default();
    let fallback_face = fallback.face();
    let scaled = face.as_scaled(height);
    let fallback_scaled = fallback_face.as_scaled(height);
    let mut x = height * 0.1;
    let mut right = x;
    for c in name.chars() {
        let (font, id) = if scaled.glyph_id(c).0 != 0 {
            (&scaled, scaled.glyph_id(c))
        } else {
            (&fallback_scaled, fallback_scaled.glyph_id(c))
        };
        let glyph = id.with_scale_and_position(height, ab_glyph::point(x, font.ascent()));
        if let Some(outline) = font.outline_glyph(glyph) {
            right = right.max(outline.px_bounds().max.x);
        }
        x += font.h_advance(id);
    }
    (right.max(x) + height * 0.1).ceil()
}
pub struct FontChoice {
    pub name: String,
    pub key: String,
    id: Option<fontdb::ID>,
}
pub struct Catalog {
    database: fontdb::Database,
    pub choices: Vec<FontChoice>,
    picker_width: OnceLock<f32>,
}
impl Catalog {
    /// Measure names in their own faces once, without retaining every font's data.
    pub fn picker_width(&self) -> f32 {
        *self.picker_width.get_or_init(|| {
            self.choices.iter().fold(440_f32, |width, choice| {
                let label = match choice.id {
                    Some(id) => self
                        .database
                        .with_face_data(id, |data, index| {
                            FontRef::try_from_slice_and_index(data, index)
                                .ok()
                                .map(|face| label_width(face, &choice.name, 20.))
                        })
                        .flatten(),
                    None => self.load(choice).map(|font| font.preview_width(20.)),
                };
                width.max(label.unwrap_or(0.) + 32.)
            })
        })
    }
    pub fn system() -> &'static Self {
        static CATALOG: OnceLock<Catalog> = OnceLock::new();
        CATALOG.get_or_init(|| {
            let mut database = fontdb::Database::new();
            database.load_system_fonts();
            let mut installed: Vec<_> = database
                .faces()
                .map(|face| FontChoice {
                    name: face.post_script_name.clone(),
                    key: format!("system-font-{:?}", face.id),
                    id: Some(face.id),
                })
                .collect();
            installed.sort_by_cached_key(|a| a.name.to_lowercase());
            installed.dedup_by(|a, b| a.name == b.name);
            let mut choices = vec![
                FontChoice {
                    name: "Ubuntu Light".into(),
                    key: "Ubuntu-Light".into(),
                    id: None,
                },
                FontChoice {
                    name: "Hack".into(),
                    key: "Hack".into(),
                    id: None,
                },
            ];
            choices.extend(installed);
            Self {
                database,
                choices,
                picker_width: OnceLock::new(),
            }
        })
    }
    pub fn load(&self, choice: &FontChoice) -> Option<TextFont> {
        match choice.id {
            None if choice.key == "Hack" => Some(TextFont::bundled(
                "Hack",
                "Hack",
                epaint_default_fonts::HACK_REGULAR,
            )),
            None => Some(TextFont::default()),
            Some(id) => self.database.with_face_data(id, |data, index| {
                FontRef::try_from_slice_and_index(data, index).ok()?;
                Some(TextFont {
                    key: choice.key.clone(),
                    name: choice.name.clone(),
                    data: Arc::from(data),
                    index,
                })
            })?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_uses_each_font_and_keeps_its_background_transparent() {
        let ubuntu = TextFont::default();
        let mut hack = TextFont::bundled("Hack", "Hack", epaint_default_fonts::HACK_REGULAR);
        hack.name = ubuntu.name.clone();
        let first = ubuntu.preview([240, 24], 18., egui::Color32::WHITE);
        let second = hack.preview([240, 24], 18., egui::Color32::WHITE);
        assert_ne!(
            first.pixels, second.pixels,
            "Each label must use its own font face"
        );
        assert!(first.pixels.iter().any(|p| p.a() == 0));
        assert!(
            first.pixels.iter().any(|p| p.a() > 0 && p.a() < 255),
            "Glyph edges need antialiasing coverage"
        );
    }
}
