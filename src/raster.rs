//! Software egui mesh renderer. No graphics context or toolkit window is needed.
use egui::{
    ClippedPrimitive, TextureId,
    epaint::{ImageDelta, Primitive},
};
use image::RgbaImage;
use std::collections::HashMap;
#[derive(Default)]
pub struct Raster {
    textures: HashMap<TextureId, egui::ColorImage>,
}
impl Raster {
    pub fn update(&mut self, id: TextureId, delta: &ImageDelta) {
        let egui::ImageData::Color(image) = &delta.image;
        if let Some([x, y]) = delta.pos {
            if let Some(texture) = self.textures.get_mut(&id) {
                for yy in 0..image.size[1] {
                    for xx in 0..image.size[0] {
                        texture[(x + xx, y + yy)] = image[(xx, yy)];
                    }
                }
            }
        } else {
            self.textures.insert(id, (**image).clone());
        }
    }
    pub fn free(&mut self, id: TextureId) {
        self.textures.remove(&id);
    }
    pub fn draw(&self, image: &mut RgbaImage, primitives: &[ClippedPrimitive], scale: f32) {
        for primitive in primitives {
            let Primitive::Mesh(mesh) = &primitive.primitive else {
                continue;
            };
            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            let clip = primitive.clip_rect;
            for indices in mesh.indices.as_chunks::<3>().0 {
                let a = mesh.vertices[indices[0] as usize];
                let b = mesh.vertices[indices[1] as usize];
                let c = mesh.vertices[indices[2] as usize];
                if [a.color, b.color, c.color].iter().all(|c| c.a() == 0) {
                    continue;
                }
                let positions = [
                    a.pos.to_vec2() * scale,
                    b.pos.to_vec2() * scale,
                    c.pos.to_vec2() * scale,
                ];
                let [pa, pb, pc] = positions;
                let edge = |p: egui::Vec2, q: egui::Vec2, r: egui::Vec2| {
                    (q.x - p.x) * (r.y - p.y) - (q.y - p.y) * (r.x - p.x)
                };
                let area = edge(pa, pb, pc);
                if area.abs() < 0.0001 {
                    continue;
                }
                let left =
                    pa.x.min(pb.x)
                        .min(pc.x)
                        .floor()
                        .max(clip.min.x * scale)
                        .max(0.) as u32;
                let top =
                    pa.y.min(pb.y)
                        .min(pc.y)
                        .floor()
                        .max(clip.min.y * scale)
                        .max(0.) as u32;
                let right =
                    pa.x.max(pb.x)
                        .max(pc.x)
                        .ceil()
                        .min(clip.max.x * scale)
                        .min(image.width() as f32)
                        .max(0.) as u32;
                let bottom =
                    pa.y.max(pb.y)
                        .max(pc.y)
                        .ceil()
                        .min(clip.max.y * scale)
                        .min(image.height() as f32)
                        .max(0.) as u32;
                for y in top..bottom {
                    for x in left..right {
                        let p = egui::vec2(x as f32 + 0.5, y as f32 + 0.5);
                        let wa = edge(pb, pc, p) / area;
                        let wb = edge(pc, pa, p) / area;
                        let wc = 1. - wa - wb;
                        if wa < 0. || wb < 0. || wc < 0. {
                            continue;
                        }
                        let uv = a.uv.to_vec2() * wa + b.uv.to_vec2() * wb + c.uv.to_vec2() * wc;
                        let tx = (uv.x * texture.size[0] as f32)
                            .floor()
                            .clamp(0., (texture.size[0] - 1) as f32)
                            as usize;
                        let ty = (uv.y * texture.size[1] as f32)
                            .floor()
                            .clamp(0., (texture.size[1] - 1) as f32)
                            as usize;
                        let tex = texture[(tx, ty)].to_array();
                        let colors = [a.color.to_array(), b.color.to_array(), c.color.to_array()];
                        let mut source = [0u16; 4];
                        for i in 0..4 {
                            source[i] = ((colors[0][i] as f32 * wa
                                + colors[1][i] as f32 * wb
                                + colors[2][i] as f32 * wc)
                                * tex[i] as f32
                                / 255.)
                                .round() as u16;
                        }
                        let destination = image.get_pixel_mut(x, y);
                        for (i, channel) in source.iter().take(3).enumerate() {
                            destination.0[i] = (*channel
                                + u16::from(destination.0[i]) * (255 - source[3]) / 255)
                                .min(255) as u8;
                        }
                    }
                }
            }
        }
    }
}
