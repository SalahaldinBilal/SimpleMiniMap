use std::f64::consts::FRAC_PI_2;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ImageEncoder, Rgba, RgbaImage};

use crate::ue::{Anchors, LinearColor, Margin, Name, Obj, Vec2, Weak};
use crate::widgets::{self, Widgets};

const BACKDROP: LinearColor = LinearColor { r: 0.04, g: 0.06, b: 0.08, a: 0.85 };
const FOG_PARAMETER: &str = "TargetTexture";
const RIM_COLOR: [u8; 3] = [14, 19, 26];
const RIM_ALPHA: f32 = 235.0;
const RIM_PIXELS_PER_UNIT: f64 = 2.0;
const RIM_MAX_PIXELS: f64 = 1024.0;
const CONE_PIXELS: u32 = 256;
const CONE_HALF_ANGLE: f64 = 0.7;
const CONE_COLOR: [f32; 3] = [110.0, 210.0, 255.0];
const CONE_ALPHA: f32 = 90.0;
const CONE_SIZE: f64 = 0.9;
// Most strips one rounded corner band may use; each strip is its own clipping zone, drawn every frame.
const MAX_STRIPS_PER_BAND: usize = 40;
// The step between a strip and the curve aimed for, in frame units; the rim is sized to the step actually left.
const TARGET_STEP: f64 = 1.0;
const MIN_RIM: f64 = 2.0;

const Z_MAP: i32 = 0;
const Z_UNDER: i32 = 1;
const Z_FOG: i32 = 2;
const Z_ABOVE: i32 = 3;
const Z_CONE: i32 = 4;
const Z_RIM: i32 = 9;

/// One clipped band of the shape, holding its own copy of a full-size image.
struct Strip {
    canvas: Weak,
    image_slot: Weak,
    image: Weak,
    origin: Vec2,
    terrain: Option<StripTerrain>,
}

/// A map strip's copy of the rendered terrain tiles, on a canvas that moves and turns with its map image.
struct StripTerrain {
    canvas: Weak,
    slot: Weak,
    tiles: Vec<(Weak, Weak)>,
}

/// A rendered terrain tile, as fractions of the map layer, so it follows the layer's size by itself.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TileView {
    pub texture: Weak,
    pub anchors: Anchors,
    /// Draw order among tiles: finer tiles go over coarser ones.
    pub z: i32,
}

const WHITE: LinearColor = LinearColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
// Tiles overlap by this much so no hairline shows between them.
const TILE_OVERLAP: f64 = 0.5;

/// How many strips a rounded corner band of radius `radius` gets, and how far (frame units) a strip, as wide as
/// the curve at its middle, can stray from the curve on either side.
fn strip_plan(radius: f64) -> (usize, f64) {
    let count = ((radius * std::f64::consts::PI / (4.0 * TARGET_STEP)).ceil() as usize).clamp(3, MAX_STRIPS_PER_BAND);
    (count, radius * std::f64::consts::PI / (4.0 * count as f64))
}

/// Rectangles, in frame units, whose union is the minimap's shape: a rounded square cut into horizontal
/// strips that only bend in the corners. One rectangle when square.
fn strip_rects(size: f64, radius: f64, inset: f64, count: usize) -> Vec<(f64, f64, f64, f64)> {
    if radius < 0.5 {
        return vec![(0.0, 0.0, size, size)];
    }
    let side = size - 2.0 * inset;
    let r = (radius - inset).clamp(1.0, side / 2.0);
    let mut rects = Vec::new();
    if side - 2.0 * r > 0.5 {
        rects.push((inset, inset + r, side, side - 2.0 * r));
    }
    for k in 0..count {
        let t0 = k as f64 / count as f64 * FRAC_PI_2;
        let t1 = (k + 1) as f64 / count as f64 * FRAC_PI_2;
        let (top, bottom) = (r - r * t0.cos(), r - r * t1.cos());
        let dx = r - r * ((t0 + t1) / 2.0).sin();
        let (x, width, height) = (inset + dx, side - 2.0 * dx, bottom - top);
        if width < 0.5 || height < 0.1 {
            continue;
        }
        rects.push((x, inset + top, width, height));
        rects.push((x, size - inset - bottom, width, height));
    }
    rects
}

/// Signed distance from `p` to a rounded square centred at `center` (negative inside).
fn rounded_distance(p: (f64, f64), center: f64, half: f64, radius: f64) -> f64 {
    let qx = (p.0 - center).abs() - (half - radius);
    let qy = (p.1 - center).abs() - (half - radius);
    let outside = qx.max(0.0).hypot(qy.max(0.0));
    outside + qx.max(qy).min(0.0) - radius
}

fn coverage(distance_inside: f64) -> f32 {
    (distance_inside + 0.5).clamp(0.0, 1.0) as f32
}

fn encode(image: &RgbaImage) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::NoFilter)
        .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(bytes)
}

/// The ring over the shape's edge. Its inner edge is a true curve, and it is wider than the strips' steps.
fn rim_png(size: f64, radius: f64, rim: f64) -> Option<Vec<u8>> {
    let scale = (RIM_PIXELS_PER_UNIT).min(RIM_MAX_PIXELS / size);
    let pixels = (size * scale).round().max(8.0) as u32;
    let unit = size / f64::from(pixels);
    let half = size / 2.0;
    let image = RgbaImage::from_fn(pixels, pixels, |x, y| {
        let p = ((f64::from(x) + 0.5) * unit, (f64::from(y) + 0.5) * unit);
        let outer = rounded_distance(p, half, half, radius);
        let inner = rounded_distance(p, half, half - rim, (radius - rim).max(0.0));
        let alpha = coverage(-outer / unit) * coverage(inner / unit);
        Rgba([RIM_COLOR[0], RIM_COLOR[1], RIM_COLOR[2], (alpha * RIM_ALPHA) as u8])
    });
    encode(&image)
}

/// A soft wedge pointing up from the image centre.
fn cone_png() -> Option<Vec<u8>> {
    let half = f64::from(CONE_PIXELS) / 2.0;
    let image = RgbaImage::from_fn(CONE_PIXELS, CONE_PIXELS, |x, y| {
        let (dx, dy) = (f64::from(x) + 0.5 - half, half - (f64::from(y) + 0.5));
        let distance = dx.hypot(dy) / half;
        let off_axis = dx.atan2(dy).abs();
        let edge = ((CONE_HALF_ANGLE - off_axis) / 0.08).clamp(0.0, 1.0);
        let fade = (1.0 - distance).clamp(0.0, 1.0);
        let alpha = (edge * fade) as f32 * CONE_ALPHA;
        Rgba([CONE_COLOR[0] as u8, CONE_COLOR[1] as u8, CONE_COLOR[2] as u8, alpha as u8])
    });
    encode(&image)
}

pub const BACKING_PIXELS: u32 = 64;

/// A soft dark disc with a faint light edge, drawn behind landmark icons so they stand out on any ground.
pub fn backing_png() -> Option<Vec<u8>> {
    let half = f64::from(BACKING_PIXELS) / 2.0;
    let image = RgbaImage::from_fn(BACKING_PIXELS, BACKING_PIXELS, |x, y| {
        let r = (f64::from(x) + 0.5 - half).hypot(f64::from(y) + 0.5 - half) / half;
        let disc = ((1.0 - r) / 0.14).clamp(0.0, 1.0);
        let edge = (1.0 - ((r - 0.86).abs() / 0.05)).clamp(0.0, 1.0);
        let shade = 22.0 + 210.0 * edge;
        let alpha = (disc * 200.0).max(edge * 140.0);
        Rgba([shade as u8, (shade + 4.0) as u8, (shade + 10.0) as u8, alpha as u8])
    });
    encode(&image)
}

/// A small light triangle pointing up with a dark outline, for edge pins.
pub fn pin_arrow_png() -> Option<Vec<u8>> {
    const SIZE: u32 = 32;
    let image = RgbaImage::from_fn(SIZE, SIZE, |x, y| {
        let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        // Triangle tip at the top centre, base along y = 26.
        let (tip, base, half_width) = (6.0, 26.0, 11.0);
        let along = ((py - tip) / (base - tip)).clamp(0.0, 1.0);
        let inside_x = half_width * along - (px - 16.0).abs();
        let inside_y = (py - tip).min(base - py);
        let inside = inside_x.min(inside_y);
        let fill = (inside / 1.2).clamp(0.0, 1.0);
        let outline = ((inside + 2.0) / 1.2).clamp(0.0, 1.0);
        let shade = 245.0 * fill + 15.0 * (1.0 - fill);
        Rgba([shade as u8, shade as u8, (shade + 5.0).min(255.0) as u8, (outline * 255.0) as u8])
    });
    encode(&image)
}

/// What sits inside the minimap box: the map and fog art cut to the box's shape, the two marker layers
/// (under and over the fog), the rim that finishes a rounded shape, and the view cone.
pub struct Layers {
    tree: Weak,
    frame: Weak,
    map_strips: Vec<Strip>,
    fog_strips: Vec<Strip>,
    fog_materials: Vec<Weak>,
    under: Weak,
    under_slot: Weak,
    above: Weak,
    above_slot: Weak,
    rim: Weak,
    rim_slot: Weak,
    cone: Weak,
    cone_slot: Weak,
    store: Weak,
    fog_shown: bool,
    tiles: Vec<Option<TileView>>,
    terrain_shown: bool,
    map_texture: Weak,
    fog_material: Option<Weak>,
    fog_texture: Option<Weak>,
    fog_parameter: Name,
    size: f64,
    radius: f64,
    rim_width: f64,
    layer_size: Vec2,
    position: Vec2,
    pivot: Vec2,
    angle: f32,
}

impl Layers {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        w: &mut Widgets,
        world: Obj,
        tree: Obj,
        frame: Obj,
        size: f64,
        roundness: f64,
        map_texture: Obj,
        fog_material: Option<Obj>,
        layer_size: Vec2,
    ) -> Result<Self, String> {
        let under = w.make(w.canvas_class, tree)?;
        let under_slot = w.canvas_add(frame, under, Vec2::default(), Z_UNDER)?;
        let above = w.make(w.canvas_class, tree)?;
        let above_slot = w.canvas_add(frame, above, Vec2::default(), Z_ABOVE)?;
        let rim = w.image(tree, LinearColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 })?;
        let rim_slot = w.canvas_add(frame, rim, Vec2::default(), Z_RIM)?;
        w.visibility(rim, widgets::HIT_TEST_INVISIBLE);
        let cone = w.image(tree, LinearColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 })?;
        let cone_slot = w.canvas_add(frame, cone, Vec2 { x: 0.0, y: 0.0 }, Z_CONE)?;
        w.align(cone_slot, Vec2::default(), Vec2 { x: 0.5, y: 0.5 });
        if let Some(texture) = cone_png().and_then(|png| w.import_png(world, &png)) {
            w.image_texture(cone, texture);
        }
        w.visibility(cone, widgets::COLLAPSED);
        // Holds every kept terrain tile on an image that is never drawn, so the tiles are not garbage collected.
        let store = w.make(w.canvas_class, tree)?;
        w.canvas_add(frame, store, Vec2::default(), Z_MAP)?;
        w.visibility(store, widgets::COLLAPSED);
        let mut layers = Self {
            tree: Weak::new(tree),
            frame: Weak::new(frame),
            map_strips: Vec::new(),
            fog_strips: Vec::new(),
            fog_materials: Vec::new(),
            under: Weak::new(under),
            under_slot: Weak::new(under_slot),
            above: Weak::new(above),
            above_slot: Weak::new(above_slot),
            rim: Weak::new(rim),
            rim_slot: Weak::new(rim_slot),
            cone: Weak::new(cone),
            cone_slot: Weak::new(cone_slot),
            store: Weak::new(store),
            fog_shown: true,
            tiles: Vec::new(),
            terrain_shown: false,
            map_texture: Weak::new(map_texture),
            fog_material: fog_material.map(Weak::new),
            fog_texture: None,
            fog_parameter: Name::new(FOG_PARAMETER),
            size: 0.0,
            radius: -1.0,
            rim_width: 0.0,
            layer_size,
            position: Vec2::default(),
            pivot: Vec2::default(),
            angle: 0.0,
        };
        layers.reshape(w, world, size, roundness)?;
        layers.set_layer_size(w, layer_size);
        Ok(layers)
    }

    /// The canvas a marker goes on: under the fog, so unexplored areas cover it, or over it.
    pub fn canvas(&self, over_fog: bool) -> Option<Obj> {
        if over_fog { self.above.get() } else { self.under.get() }
    }

    /// Recuts the shape for a new size or roundness. Cheap when neither changed.
    pub fn reshape(&mut self, w: &mut Widgets, world: Obj, size: f64, roundness: f64) -> Result<(), String> {
        let radius = roundness.clamp(0.0, 100.0) / 100.0 * size / 2.0;
        if (size - self.size).abs() < 0.01 && (radius - self.radius).abs() < 0.01 {
            return Ok(());
        }
        let (Some(tree), Some(frame)) = (self.tree.get(), self.frame.get()) else { return Err("minimap is gone".into()) };
        for strip in self.map_strips.drain(..).chain(self.fog_strips.drain(..)) {
            if let Some(canvas) = strip.canvas.get() {
                w.remove(canvas);
            }
        }
        self.fog_materials.clear();
        self.size = size;
        self.radius = radius;
        // Strips are pulled in by one step so none pokes past the curve, and fall short of it by up to one
        // step, so the rim has to cover two.
        // Always drawn, square too, so it only thickens as the corners round instead of appearing.
        let (count, step) = strip_plan(radius);
        self.rim_width = if radius >= 0.5 { (2.0 * step + 0.5).max(MIN_RIM) } else { MIN_RIM };

        for (x, y, width, height) in strip_rects(size, radius, step, count) {
            let origin = Vec2 { x, y };
            let extent = Vec2 { x: width, y: height };
            let map = self.strip(w, tree, frame, origin, extent, Z_MAP, true)?;
            if let Some(texture) = self.map_texture.get() {
                w.image_texture(map.image.get().ok_or("strip image is gone")?, texture);
            }
            self.map_strips.push(map);
            if let Some(material) = self.fog_material.and_then(Weak::get) {
                let fog = self.strip(w, tree, frame, origin, extent, Z_FOG, false)?;
                let image = fog.image.get().ok_or("strip image is gone")?;
                if let Some(dynamic) = w.image_material(image, material) {
                    if let Some(texture) = self.fog_texture.and_then(Weak::get) {
                        w.texture_parameter(dynamic, self.fog_parameter, texture);
                    }
                    self.fog_materials.push(Weak::new(dynamic));
                }
                self.fog_strips.push(fog);
            }
        }

        let tiles = std::mem::take(&mut self.tiles);
        self.set_tiles(w, &tiles);

        let (Some(rim), Some(rim_slot)) = (self.rim.get(), self.rim_slot.get()) else { return Ok(()) };
        if let Some(texture) = rim_png(size, radius, self.rim_width).and_then(|png| w.import_png(world, &png)) {
            w.image_texture(rim, texture);
        }
        w.place(rim_slot, Vec2::default(), Some(Vec2 { x: size, y: size }));
        let layer_size = self.layer_size;
        self.set_layer_size(w, layer_size);
        let (position, pivot, angle) = (self.position, self.pivot, self.angle);
        // The new strips have not been turned yet.
        self.angle = f32::NAN;
        self.place_all(w, position, pivot, angle);
        Ok(())
    }

    /// Shows these terrain tiles, by slot, over the map art. Only slots that changed are touched.
    pub fn set_tiles(&mut self, w: &mut Widgets, views: &[Option<TileView>]) {
        for index in 0..views.len().max(self.tiles.len()) {
            let view = views.get(index).copied().flatten();
            if self.tiles.get(index).copied().flatten() != view {
                self.apply_tile(w, index, view);
            }
        }
        self.tiles = views.to_vec();
        let shown = views.iter().any(Option::is_some);
        if shown == self.terrain_shown {
            return;
        }
        self.terrain_shown = shown;
        let (position, pivot, angle) = (self.position, self.pivot, self.angle);
        for strip in &self.map_strips {
            let Some(terrain) = &strip.terrain else { continue };
            let Some(canvas) = terrain.canvas.get() else { continue };
            w.visibility(canvas, if shown { widgets::HIT_TEST_INVISIBLE } else { widgets::COLLAPSED });
            if !shown {
                continue;
            }
            if let Some(slot) = terrain.slot.get() {
                w.position(slot, Vec2 { x: position.x - strip.origin.x, y: position.y - strip.origin.y });
            }
            w.pivot(canvas, pivot);
            w.angle(canvas, angle);
        }
    }

    fn apply_tile(&mut self, w: &mut Widgets, index: usize, view: Option<TileView>) {
        let Some(tree) = self.tree.get() else { return };
        let texture = view.and_then(|view| view.texture.get());
        for strip in &mut self.map_strips {
            let Some(terrain) = strip.terrain.as_mut() else { continue };
            let Some(canvas) = terrain.canvas.get() else { continue };
            while texture.is_some() && terrain.tiles.len() <= index {
                let Ok(image) = w.image(tree, WHITE) else { return };
                let Ok(slot) = w.canvas_add(canvas, image, Vec2::default(), 0) else { return };
                w.visibility(image, widgets::COLLAPSED);
                terrain.tiles.push((Weak::new(image), Weak::new(slot)));
            }
            let Some(&(image, slot)) = terrain.tiles.get(index) else { continue };
            let (Some(image), Some(slot)) = (image.get(), slot.get()) else { continue };
            match (view, texture) {
                (Some(view), Some(texture)) => {
                    w.image_texture(image, texture);
                    w.stretch(slot, view.anchors, TILE_OVERLAP);
                    w.z_order(slot, view.z);
                    w.visibility(image, widgets::HIT_TEST_INVISIBLE);
                }
                _ => w.visibility(image, widgets::COLLAPSED),
            }
        }
    }

    /// Keeps a terrain tile alive until released.
    pub fn hold(&mut self, w: &mut Widgets, texture: Obj) -> Option<Weak> {
        let (tree, store) = (self.tree.get()?, self.store.get()?);
        let image = w.image(tree, WHITE).ok()?;
        w.canvas_add(store, image, Vec2::default(), 0).ok()?;
        w.image_texture(image, texture);
        Some(Weak::new(image))
    }

    pub fn release(&mut self, w: &mut Widgets, holder: Weak) {
        if let Some(image) = holder.get() {
            w.remove(image);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn strip(&self, w: &mut Widgets, tree: Obj, frame: Obj, origin: Vec2, extent: Vec2, z: i32, backdrop: bool) -> Result<Strip, String> {
        let canvas = w.make(w.canvas_class, tree)?;
        let slot = w.canvas_add(frame, canvas, Vec2::default(), z)?;
        w.place(slot, origin, Some(extent));
        w.clip(canvas);
        let shown = backdrop || self.fog_shown;
        w.visibility(canvas, if shown { widgets::HIT_TEST_INVISIBLE } else { widgets::COLLAPSED });
        if backdrop {
            let fill = w.border(tree, BACKDROP, Margin::default())?;
            w.canvas_fill(canvas, fill, 0)?;
        }
        let image = w.image(tree, WHITE)?;
        let image_slot = w.canvas_add(canvas, image, Vec2::default(), 1)?;
        let terrain = if backdrop {
            let terrain = w.make(w.canvas_class, tree)?;
            let slot = w.canvas_add(canvas, terrain, Vec2::default(), 2)?;
            w.visibility(terrain, if self.terrain_shown { widgets::HIT_TEST_INVISIBLE } else { widgets::COLLAPSED });
            Some(StripTerrain { canvas: Weak::new(terrain), slot: Weak::new(slot), tiles: Vec::new() })
        } else {
            None
        };
        Ok(Strip { canvas: Weak::new(canvas), image_slot: Weak::new(image_slot), image: Weak::new(image), origin, terrain })
    }

    pub fn set_map_texture(&mut self, w: &mut Widgets, texture: Obj) {
        self.map_texture = Weak::new(texture);
        for image in self.map_strips.iter().filter_map(|strip| strip.image.get()) {
            w.image_texture(image, texture);
        }
    }

    /// Points every fog copy at the game's fog render target. Returns true when it changed.
    pub fn set_fog_texture(&mut self, w: &mut Widgets, texture: Obj) -> bool {
        if self.fog_texture.and_then(Weak::get) == Some(texture) {
            return false;
        }
        self.fog_texture = Some(Weak::new(texture));
        for material in self.fog_materials.iter().filter_map(|material| material.get()) {
            w.texture_parameter(material, self.fog_parameter, texture);
        }
        true
    }

    /// Shows or hides the clouds over unexplored areas.
    pub fn set_fog_shown(&mut self, w: &mut Widgets, shown: bool) {
        if shown == self.fog_shown {
            return;
        }
        self.fog_shown = shown;
        for canvas in self.fog_strips.iter().filter_map(|strip| strip.canvas.get()) {
            w.visibility(canvas, if shown { widgets::HIT_TEST_INVISIBLE } else { widgets::COLLAPSED });
        }
    }

    pub fn has_fog(&self) -> bool {
        !self.fog_materials.is_empty()
    }

    /// Sizes only. Markers and tiles are anchored to fractions of the layer, so they follow on their own.
    pub fn set_layer_size(&mut self, w: &mut Widgets, size: Vec2) {
        self.layer_size = size;
        let strips = self.map_strips.iter().chain(&self.fog_strips);
        let terrain = self.map_strips.iter().filter_map(|strip| strip.terrain.as_ref()?.slot.get());
        let slots = strips
            .filter_map(|strip| strip.image_slot.get())
            .chain(terrain)
            .chain([self.under_slot.get(), self.above_slot.get()].into_iter().flatten());
        for slot in slots {
            w.size(slot, size);
        }
    }

    /// Moves and turns everything on the map: `position` is the map's top left in the frame, `pivot` the
    /// player's spot on the map as a fraction of its size, and the map turns `angle` degrees around it.
    pub fn place_all(&mut self, w: &mut Widgets, position: Vec2, pivot: Vec2, angle: f32) {
        // The pivot only matters while turned; skipping it otherwise saves two calls per strip per frame.
        let turned = angle != self.angle || (angle != 0.0 && pivot != self.pivot);
        self.position = position;
        self.pivot = pivot;
        self.angle = angle;
        for strip in self.map_strips.iter().chain(&self.fog_strips) {
            if let Some(slot) = strip.image_slot.get() {
                w.position(slot, Vec2 { x: position.x - strip.origin.x, y: position.y - strip.origin.y });
            }
            if let (true, Some(image)) = (turned, strip.image.get()) {
                w.pivot(image, pivot);
                w.angle(image, angle);
            }
            // Hidden terrain is placed when it is shown, so it costs nothing per frame until then.
            let Some(terrain) = strip.terrain.as_ref().filter(|_| self.terrain_shown) else { continue };
            if let Some(slot) = terrain.slot.get() {
                w.position(slot, Vec2 { x: position.x - strip.origin.x, y: position.y - strip.origin.y });
            }
            if let (true, Some(canvas)) = (turned, terrain.canvas.get()) {
                w.pivot(canvas, pivot);
                w.angle(canvas, angle);
            }
        }
        for (slot, canvas) in [(self.under_slot, self.under), (self.above_slot, self.above)] {
            if let Some(slot) = slot.get() {
                w.position(slot, position);
            }
            if let (true, Some(canvas)) = (turned, canvas.get()) {
                w.pivot(canvas, pivot);
                w.angle(canvas, angle);
            }
        }
    }

    /// Where a point on the map ends up in the frame, after the map's turn.
    pub fn frame_point(&self, on_map: Vec2) -> Vec2 {
        let center = Vec2 {
            x: self.position.x + self.pivot.x * self.layer_size.x,
            y: self.position.y + self.pivot.y * self.layer_size.y,
        };
        let (dx, dy) = (self.position.x + on_map.x - center.x, self.position.y + on_map.y - center.y);
        let (sin, cos) = f64::from(self.angle).to_radians().sin_cos();
        Vec2 { x: center.x + dx * cos - dy * sin, y: center.y + dx * sin + dy * cos }
    }

    /// Whether a frame point is inside the visible shape, rim excluded. Always true when square.
    pub fn inside(&self, point: Vec2) -> bool {
        if self.radius < 0.5 {
            return true;
        }
        let half = self.size / 2.0;
        rounded_distance((point.x, point.y), half, half - self.rim_width, (self.radius - self.rim_width).max(0.0)) <= 0.0
    }

    /// Where a line from the frame centre in direction `dir` leaves the visible shape, pulled in by `inset`.
    pub fn edge_point(&self, dir: Vec2, inset: f64) -> Vec2 {
        let half = self.size / 2.0;
        let length = dir.x.hypot(dir.y).max(1e-9);
        let (dx, dy) = (dir.x / length, dir.y / length);
        let (box_half, radius) = if self.radius < 0.5 {
            (half - MIN_RIM - inset, 0.0)
        } else {
            (half - self.rim_width - inset, (self.radius - self.rim_width - inset).max(0.0))
        };
        let (mut inner, mut outer) = (0.0, half * 1.5);
        for _ in 0..20 {
            let mid = (inner + outer) / 2.0;
            if rounded_distance((half + dx * mid, half + dy * mid), half, box_half, radius) <= 0.0 {
                inner = mid;
            } else {
                outer = mid;
            }
        }
        Vec2 { x: half + dx * inner, y: half + dy * inner }
    }

    pub fn size(&self) -> f64 {
        self.size
    }

    /// The camera's view cone at the frame centre, turned `angle` degrees, or hidden.
    pub fn cone(&mut self, w: &mut Widgets, shown: bool, angle: f32) {
        let (Some(cone), Some(slot)) = (self.cone.get(), self.cone_slot.get()) else { return };
        if !shown {
            w.visibility(cone, widgets::COLLAPSED);
            return;
        }
        let extent = self.size * CONE_SIZE;
        w.place(slot, Vec2 { x: self.size / 2.0, y: self.size / 2.0 }, Some(Vec2 { x: extent, y: extent }));
        w.angle(cone, angle);
        w.visibility(cone, widgets::HIT_TEST_INVISIBLE);
    }
}
