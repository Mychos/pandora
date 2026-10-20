//! Headless egui harness for the layout tests: run frames without a window and
//! read either the painted shapes or the rendered pixels.

use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{pos2, Color32, ImageData, Pos2, Rect, TextureId, TexturesDelta};
use std::collections::HashMap;

pub type Atlas = HashMap<TextureId, (usize, usize, Vec<Color32>)>;

pub fn input(size: egui::Vec2, events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), size)),
        events,
        ..Default::default()
    }
}

pub fn frame(
    ctx: &egui::Context,
    size: egui::Vec2,
    events: Vec<egui::Event>,
    draw: impl FnMut(&mut egui::Ui),
) -> Vec<egui::epaint::ClippedShape> {
    let mut out = ctx.run_ui(input(size, events), draw);
    out.textures_delta.clear();
    out.shapes
}

/// Run `draw` until egui's fades have settled and write the pixels to
/// `/tmp/opencode/{name}.png`, so a screen can be eyeballed. `held` drags from one
/// point to another across the first frames.
pub fn png(
    ctx: &egui::Context,
    name: &str,
    w: u32,
    h: u32,
    held: Option<(Pos2, Pos2)>,
    mut draw: impl FnMut(&mut egui::Ui),
) {
    let mut atlas: Atlas = HashMap::new();
    let mut prims: Vec<ClippedPrimitive> = Vec::new();
    for frame in 0..15 {
        let mut input = input(egui::vec2(w as f32, h as f32), Vec::new());
        if let Some((from, to)) = held {
            match frame {
                0 => {
                    input.events.push(egui::Event::PointerMoved(from));
                    input.events.push(egui::Event::PointerButton {
                        pos: from,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    });
                }
                1 => input.events.push(egui::Event::PointerMoved(to)),
                _ => {}
            }
        }
        let mut out = ctx.run_ui(input, &mut draw);
        let mut delta = std::mem::take(&mut out.textures_delta);
        apply(&mut atlas, &delta);
        delta.clear();
        prims = ctx.tessellate(out.shapes, 1.0);
    }
    let bytes = raster(&prims, &atlas, w as usize, h as usize);
    image::RgbaImage::from_raw(w, h, bytes)
        .expect("the raster is w * h * 4")
        .save(format!("/tmp/opencode/{name}.png"))
        .expect("the shots folder is writable");
}

/// Every text run painted, in paint order.
pub fn words(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

/// The primary button going down (`pressed`) or up, at `at`.
pub fn press(at: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}

pub fn apply(atlas: &mut Atlas, delta: &TexturesDelta) {
    for (id, patches) in &delta.set {
        for patch in patches {
            let ImageData::Color(img) = &patch.image;
            let (w, h) = (img.size[0], img.size[1]);
            match patch.pos {
                None => {
                    atlas.insert(*id, (w, h, img.pixels.clone()));
                }
                Some([x, y]) => {
                    if let Some((tw, th, px)) = atlas.get_mut(id) {
                        for row in 0..h {
                            for col in 0..w {
                                let (dx, dy) = (x + col, y + row);
                                if dx < *tw && dy < *th {
                                    px[dy * *tw + dx] = img.pixels[row * w + col];
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn sample(atlas: &Atlas, id: TextureId, uv: Pos2) -> Color32 {
    let Some((w, h, px)) = atlas.get(&id) else {
        return Color32::WHITE;
    };
    let x = (uv.x * *w as f32) as usize;
    let y = (uv.y * *h as f32) as usize;
    px[y.min(*h - 1) * *w + x.min(*w - 1)]
}

fn mul(a: Color32, b: Color32) -> [f32; 4] {
    let f = |x: u8, y: u8| x as f32 * y as f32 / 255.0;
    [
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    ]
}

pub fn raster(prims: &[ClippedPrimitive], atlas: &Atlas, w: usize, h: usize) -> Vec<u8> {
    let mut buf = vec![0u8; w * h * 4];
    for cp in prims {
        let Primitive::Mesh(mesh) = &cp.primitive else {
            continue;
        };
        let clip = cp.clip_rect;
        for t in mesh.indices.as_chunks::<3>().0 {
            let v = [
                mesh.vertices[t[0] as usize],
                mesh.vertices[t[1] as usize],
                mesh.vertices[t[2] as usize],
            ];
            let area = (v[1].pos.x - v[0].pos.x) * (v[2].pos.y - v[0].pos.y)
                - (v[2].pos.x - v[0].pos.x) * (v[1].pos.y - v[0].pos.y);
            if area == 0.0 {
                continue;
            }
            let x0 = v
                .iter()
                .map(|p| p.pos.x)
                .fold(f32::MAX, f32::min)
                .max(clip.min.x)
                .max(0.0);
            let x1 = v
                .iter()
                .map(|p| p.pos.x)
                .fold(f32::MIN, f32::max)
                .min(clip.max.x)
                .min(w as f32 - 1.0);
            let y0 = v
                .iter()
                .map(|p| p.pos.y)
                .fold(f32::MAX, f32::min)
                .max(clip.min.y)
                .max(0.0);
            let y1 = v
                .iter()
                .map(|p| p.pos.y)
                .fold(f32::MIN, f32::max)
                .min(clip.max.y)
                .min(h as f32 - 1.0);
            for y in y0.ceil() as usize..=y1.floor() as usize {
                for x in x0.ceil() as usize..=x1.floor() as usize {
                    let p = pos2(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((v[1].pos.x - p.x) * (v[2].pos.y - p.y)
                        - (v[2].pos.x - p.x) * (v[1].pos.y - p.y))
                        / area;
                    let w1 = ((v[2].pos.x - p.x) * (v[0].pos.y - p.y)
                        - (v[0].pos.x - p.x) * (v[2].pos.y - p.y))
                        / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let uv = pos2(
                        w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x,
                        w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y,
                    );
                    let c = [
                        w0 * v[0].color.r() as f32
                            + w1 * v[1].color.r() as f32
                            + w2 * v[2].color.r() as f32,
                        w0 * v[0].color.g() as f32
                            + w1 * v[1].color.g() as f32
                            + w2 * v[2].color.g() as f32,
                        w0 * v[0].color.b() as f32
                            + w1 * v[1].color.b() as f32
                            + w2 * v[2].color.b() as f32,
                        w0 * v[0].color.a() as f32
                            + w1 * v[1].color.a() as f32
                            + w2 * v[2].color.a() as f32,
                    ];
                    let s = mul(
                        Color32::from_rgba_premultiplied(
                            c[0] as u8, c[1] as u8, c[2] as u8, c[3] as u8,
                        ),
                        sample(atlas, mesh.texture_id, uv),
                    );
                    let o = (y * w + x) * 4;
                    for ch in 0..4 {
                        let dst = buf[o + ch] as f32;
                        buf[o + ch] = (s[ch] + dst * (1.0 - s[3] / 255.0)).clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
    buf
}
