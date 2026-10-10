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
pub struct FontChoice {
    pub name: String,
    pub key: String,
    id: Option<fontdb::ID>,
}
pub struct Catalog {
    database: fontdb::Database,
    pub choices: Vec<FontChoice>,
}
impl Catalog {
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
            Self { database, choices }
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
