use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::path::PathBuf;
use std::time::Instant;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ImageEncoder, Rgba, RgbaImage};

use crate::host;
use crate::profile;
use crate::ue::{self, Function, LinearColor, Name, Obj, Vec2, Weak};

const STYLE: &str = "v1";
const SOURCE_SIZE: i32 = 128;
const BADGE_SIZE: u32 = 64;
const RING_WIDTH: f32 = 3.5;
const OUTLINE_WIDTH: f32 = 1.5;
const BACKGROUND: LinearColor = LinearColor { r: 0.025, g: 0.03, b: 0.045, a: 1.0 };
const RGBA8_SRGB: u8 = 3;
const BLEND_TRANSLUCENT: u8 = 2;
const DRAWN_TIMEOUT_SECS: u64 = 3;
const BAKE_ATTEMPTS: u32 = 3;

const RING: [f32; 3] = [255.0, 255.0, 255.0];
const BOSS_RING: [f32; 3] = [255.0, 190.0, 60.0];
const SHINY_RING: [f32; 3] = [255.0, 80.0, 220.0];

pub enum BakeStep {
    Done(Obj),
    Waiting,
    Failed,
}

/// Which badge: a species, with the shiny ring, the boss ring or the normal one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BadgeKey {
    pub character: Name,
    pub boss: bool,
    pub shiny: bool,
}

impl BadgeKey {
    fn ring(self) -> [f32; 3] {
        if self.shiny {
            SHINY_RING
        } else if self.boss {
            BOSS_RING
        } else {
            RING
        }
    }
}

struct Drawn {
    key: BadgeKey,
    at: Instant,
}

/// Round portrait badges and elevation chevrons, made once and cached as PNG next to the mod.
pub struct Badges {
    folder: PathBuf,
    rendering: Obj,
    create_target: Function,
    clear_target: Function,
    begin_draw: Function,
    end_draw: Function,
    draw_texture: Function,
    import: Function,
    target: Option<Weak>,
    drawn: Option<Drawn>,
    made: HashMap<BadgeKey, Weak>,
    not_on_disk: HashSet<BadgeKey>,
    failed: HashSet<BadgeKey>,
    attempts: HashMap<BadgeKey, u32>,
    chevrons: [Option<Weak>; 2],
}

impl Badges {
    pub fn new(folder: PathBuf) -> Result<Self, String> {
        let rendering = ue::find("/Script/Engine.Default__KismetRenderingLibrary")?;
        let canvas = ue::find("/Script/Engine.Canvas")?;
        std::fs::create_dir_all(&folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
        Ok(Self {
            create_target: Function::resolve(
                rendering,
                "CreateRenderTarget2D",
                &["WorldContextObject", "Width", "Height", "Format", "ClearColor", "bAutoGenerateMipMaps"],
            )?,
            clear_target: Function::resolve(rendering, "ClearRenderTarget2D", &["WorldContextObject", "TextureRenderTarget", "ClearColor"])?,
            begin_draw: Function::resolve(
                rendering,
                "BeginDrawCanvasToRenderTarget",
                &["WorldContextObject", "TextureRenderTarget", "Canvas", "Size", "Context"],
            )?,
            end_draw: Function::resolve(rendering, "EndDrawCanvasToRenderTarget", &["WorldContextObject", "Context"])?,
            draw_texture: Function::resolve(
                canvas,
                "K2_DrawTexture",
                &[
                    "RenderTexture",
                    "ScreenPosition",
                    "ScreenSize",
                    "CoordinatePosition",
                    "CoordinateSize",
                    "RenderColor",
                    "BlendMode",
                    "Rotation",
                    "PivotPoint",
                ],
            )?,
            import: Function::resolve(rendering, "ImportBufferAsTexture2D", &["WorldContextObject", "Buffer"])?,
            folder,
            rendering,
            target: None,
            drawn: None,
            made: HashMap::new(),
            not_on_disk: HashSet::new(),
            failed: HashSet::new(),
            attempts: HashMap::new(),
            chevrons: [None, None],
        })
    }

    fn badge_path(&self, key: BadgeKey) -> PathBuf {
        let safe: String = key
            .character
            .text()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        let kind = if key.boss { "boss" } else { "badge" };
        let shiny = if key.shiny { "-shiny" } else { "" };
        self.folder.join(format!("{kind}{shiny}-{STYLE}-{safe}.png"))
    }

    pub fn has_failed(&self, key: BadgeKey) -> bool {
        self.failed.contains(&key)
    }

    /// A badge already made this session or cached on disk.
    pub fn cached(&mut self, world: Obj, key: BadgeKey) -> Option<Obj> {
        if let Some(texture) = self.made.get(&key).and_then(|weak| weak.get()) {
            return Some(texture);
        }
        if self.not_on_disk.contains(&key) {
            return None;
        }
        let _scope = profile::scope("badge from disk");
        let Ok(bytes) = std::fs::read(self.badge_path(key)) else {
            self.not_on_disk.insert(key);
            return None;
        };
        let texture = self.import_png(world, &bytes)?;
        self.made.insert(key, Weak::new(texture));
        Some(texture)
    }

    /// One step of making a badge: the first call draws the portrait into the render target, a later
    /// call reads it back once the GPU has had a frame to finish, so the read does not stall on it.
    pub fn bake_step(&mut self, world: Obj, key: BadgeKey, portrait: Obj) -> BakeStep {
        if let Some(drawn) = &self.drawn {
            if drawn.key != key {
                if drawn.at.elapsed().as_secs() < DRAWN_TIMEOUT_SECS {
                    return BakeStep::Waiting;
                }
                self.drawn = None;
            }
        }
        if self.drawn.is_none() {
            if !profile::time("badge draw", || self.draw(world, portrait)) {
                return self.give_up(key);
            }
            self.drawn = Some(Drawn {
                key,
                at: Instant::now(),
            });
            return BakeStep::Waiting;
        }

        self.drawn = None;
        let _scope = profile::scope("badge finish");
        let started = Instant::now();
        let Some(pixels) = profile::time("read back", || self.read(world)) else {
            // The render target is only weakly held; a garbage collection between drawing and reading
            // takes it away. Drawing again makes a new one.
            let attempts = self.attempts.entry(key).or_insert(0);
            *attempts += 1;
            return if *attempts < BAKE_ATTEMPTS { BakeStep::Waiting } else { self.give_up(key) };
        };
        let read_ms = started.elapsed().as_secs_f64() * 1000.0;
        let badge = compose_badge(&pixels, SOURCE_SIZE as usize, key.ring());
        let Some(png) = encode_png(&badge) else { return self.give_up(key) };
        let path = self.badge_path(key);
        if let Err(error) = std::fs::write(&path, &png) {
            host::warning(&format!("could not cache {}: {error}", path.display()));
        }
        let Some(texture) = self.import_png(world, &png) else { return self.give_up(key) };
        host::info(&format!(
            "{}{} for {} made: read back in {read_ms:.1} ms, {:.1} ms total",
            if key.shiny { "shiny " } else { "" },
            if key.boss { "boss badge" } else { "badge" },
            key.character.text(),
            started.elapsed().as_secs_f64() * 1000.0
        ));
        self.made.insert(key, Weak::new(texture));
        self.not_on_disk.remove(&key);
        BakeStep::Done(texture)
    }

    fn give_up(&mut self, key: BadgeKey) -> BakeStep {
        host::warning(&format!("badge for {} could not be made", key.character.text()));
        self.failed.insert(key);
        BakeStep::Failed
    }

    pub fn chevron(&mut self, world: Obj, up: bool) -> Option<Obj> {
        let slot = usize::from(up);
        if let Some(texture) = self.chevrons[slot].and_then(Weak::get) {
            return Some(texture);
        }
        let _scope = profile::scope("chevron image");
        let path = self.folder.join(format!("chevron-{STYLE}-{}.png", if up { "up" } else { "down" }));
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                let bytes = encode_png(&compose_chevron(up))?;
                let _ = std::fs::write(&path, &bytes);
                bytes
            }
        };
        let texture = self.import_png(world, &bytes)?;
        self.chevrons[slot] = Some(Weak::new(texture));
        Some(texture)
    }

    fn import_png(&mut self, world: Obj, png: &[u8]) -> Option<Obj> {
        let texture: *mut c_void = self
            .import
            .call(self.rendering, |a| {
                a.set_obj(0, Some(world));
                a.set_byte_array(1, png);
            })
            .returned();
        Obj::from_raw(texture)
    }

    fn render_target(&mut self, world: Obj) -> Option<Obj> {
        if let Some(target) = self.target.and_then(Weak::get) {
            return Some(target);
        }
        let target: *mut c_void = self
            .create_target
            .call(self.rendering, |a| {
                a.set_obj(0, Some(world));
                a.set(1, SOURCE_SIZE);
                a.set(2, SOURCE_SIZE);
                a.set(3, RGBA8_SRGB);
                a.set(4, BACKGROUND);
                a.set(5, false);
            })
            .returned();
        let target = Obj::from_raw(target)?;
        self.target = Some(Weak::new(target));
        Some(target)
    }

    fn draw(&mut self, world: Obj, portrait: Obj) -> bool {
        let Some(target) = self.render_target(world) else { return false };
        self.clear_target.call(self.rendering, |a| {
            a.set_obj(0, Some(world));
            a.set_obj(1, Some(target));
            a.set(2, BACKGROUND);
        });
        self.begin_draw.call(self.rendering, |a| {
            a.set_obj(0, Some(world));
            a.set_obj(1, Some(target));
        });
        let canvas: *mut c_void = self.begin_draw.param(2);
        let context = self.begin_draw.param_bytes(4, 16);
        if let Some(canvas) = Obj::from_raw(canvas) {
            let size = f64::from(SOURCE_SIZE);
            self.draw_texture.call(canvas, |a| {
                a.set_obj(0, Some(portrait));
                a.set(1, Vec2::default());
                a.set(2, Vec2 { x: size, y: size });
                a.set(3, Vec2::default());
                a.set(4, Vec2 { x: 1.0, y: 1.0 });
                a.set(5, LinearColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
                a.set(6, BLEND_TRANSLUCENT);
                a.set(7, 0.0f32);
                a.set(8, Vec2 { x: 0.5, y: 0.5 });
            });
        }
        self.end_draw.call(self.rendering, |a| {
            a.set_obj(0, Some(world));
            a.set_bytes(1, &context);
        });
        Obj::from_raw(canvas).is_some()
    }

    fn read(&mut self, world: Obj) -> Option<Vec<u32>> {
        let target = self.target.and_then(Weak::get)?;
        let pixels_wanted = (SOURCE_SIZE * SOURCE_SIZE) as usize;
        let mut pixels = Vec::with_capacity(pixels_wanted);
        ue::read_render_target(world, target, &mut pixels).ok()?;
        (pixels.len() == pixels_wanted).then_some(pixels)
    }
}

fn encode_png(image: &RgbaImage) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::NoFilter)
        .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(bytes)
}

fn coverage(distance_inside: f32) -> f32 {
    (distance_inside + 0.5).clamp(0.0, 1.0)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Round crop of the portrait inside a white ring with a thin dark outline. `pixels` are BGRA.
fn compose_badge(pixels: &[u32], source_size: usize, ring_color: [f32; 3]) -> RgbaImage {
    let scale = source_size as f32 / BADGE_SIZE as f32;
    let center = BADGE_SIZE as f32 / 2.0;
    let outer = center - 0.5;
    let ring_outer = outer - OUTLINE_WIDTH;
    let ring_inner = ring_outer - RING_WIDTH;

    let sample = |x: f32, y: f32| -> [f32; 3] {
        let sx = ((x * scale) as usize).min(source_size - 1);
        let sy = ((y * scale) as usize).min(source_size - 1);
        let p = pixels[sy * source_size + sx];
        [((p >> 16) & 0xFF) as f32, ((p >> 8) & 0xFF) as f32, (p & 0xFF) as f32]
    };

    RgbaImage::from_fn(BADGE_SIZE, BADGE_SIZE, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let distance = (fx - center).hypot(fy - center);
        let alpha = coverage(outer - distance);
        if alpha <= 0.0 {
            return Rgba([0, 0, 0, 0]);
        }
        let portrait = sample(fx, fy);
        let ring = mix(portrait, ring_color, coverage(distance - ring_inner));
        let color = mix(ring, [12.0, 14.0, 20.0], coverage(distance - ring_outer));
        Rgba([color[0] as u8, color[1] as u8, color[2] as u8, (alpha * 255.0) as u8])
    })
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (abx, aby) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * abx + (p.1 - a.1) * aby) / (abx * abx + aby * aby)).clamp(0.0, 1.0);
    (p.0 - (a.0 + abx * t)).hypot(p.1 - (a.1 + aby * t))
}

/// A white chevron with a dark outline, pointing up or down.
fn compose_chevron(up: bool) -> RgbaImage {
    const WIDTH: u32 = 32;
    const HEIGHT: u32 = 16;
    let (top, bottom) = (4.0, 12.0);
    let (tip_y, arm_y) = if up { (top, bottom) } else { (bottom, top) };
    let left = (5.0, arm_y);
    let tip = (16.0, tip_y);
    let right = (27.0, arm_y);
    RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
        let p = (x as f32 + 0.5, y as f32 + 0.5);
        let distance = segment_distance(p, left, tip).min(segment_distance(p, tip, right));
        let stroke = coverage(1.6 - distance);
        let outline = coverage(3.0 - distance);
        let shade = (255.0 * stroke) as u8;
        let alpha = (255.0 * outline.max(stroke)) as u8;
        Rgba([shade, shade, shade, alpha])
    })
}
