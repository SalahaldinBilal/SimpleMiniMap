use std::collections::HashMap;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use crate::game::Player;
use crate::host;
use crate::layers::{Layers, TileView};
use crate::profile;
use crate::settings::{Settings, TerrainUse};
use crate::ue::{self, Anchors, Function, LinearColor, Obj, Rotator, Vec2, Vec3, Weak};
use crate::widgets::Widgets;
use crate::worldmap::{self, Region};

const TILE_UNITS: f64 = 25_600.0;
const TILE_PIXELS: i32 = 512;
// Each level doubles the tile's width, so zoomed out a few coarse tiles cover the view instead of many fine ones.
const MAX_LEVEL: u8 = 4;
// Each tile is one 1 MB render target, kept for the session.
const MAX_TILES: usize = 64;
const MAX_SHOWN: usize = 49;
// What "when zoomed in close" means: 400 m across or less.
const CLOSE_VIEW: f64 = 40_000.0;
const SHOWN_RADIUS: f64 = 0.75;
const BAND_UNITS: f64 = 1_000.0;
const BAND_SLACK: f64 = 200.0;
const CAMERA_OVER_BAND: f64 = 300.0;
// The surface camera sits this far above the highest ground the survey found in the tile, plus a share of the
// tile's width for peaks between the survey's rays. Anything above the camera would be cut off.
const CAMERA_OVER_GROUND: f64 = 3_000.0;
const CAMERA_OVER_GROUND_SHARE: f64 = 0.04;
// Rays straight down across a tile, this many on each side, to check its ground is loaded and find its height.
const SURVEY_GRID: usize = 5;
const VIEW_DISTANCE_MARGIN: f64 = 10_000.0;
const CHECK_EVERY: Duration = Duration::from_millis(250);
const CAPTURE_GAP: Duration = Duration::from_millis(750);
const SETTLE: Duration = Duration::from_secs(2);
// After a load, a teleport or a dungeon entry the game keeps streaming the area in for a while, and a capture can
// miss ground that is not drawn yet. Tiles captured this soon after are captured once more later.
const WARMUP: Duration = Duration::from_secs(12);
const JUMP: f64 = 10_000.0;
const MAX_FAILURES: u32 = 3;
// Tiles surveyed per update, at 25 rays each.
const LOAD_CHECKS_PER_TICK: usize = 2;
const LOAD_CHECK_REACH: f64 = 200_000.0;
// The light lit tiles are drawn with instead of the game's sun, moon and sky: always the same, so tiles made at
// any time of day match. It comes from the north-west and high, the usual direction for shaded relief maps.
const MAP_LIGHT_ROTATION: Rotator = Rotator { pitch: -50.0, yaw: 135.0, roll: 0.0 };
// The fill comes from straight above at this share of the key light's brightness, so nothing is left unlit.
const FILL_LIGHT_ROTATION: Rotator = Rotator { pitch: -90.0, yaw: 0.0, roll: 0.0 };
const FILL_SHARE: f32 = 0.6;
// The key light's brightness, with the map look's tint and without it: what a per-world calibration settled on
// over grassland, every time. Fixed, so every tile matches wherever the session starts.
const MAP_LIGHT_INTENSITY: f32 = 15.0;
const NATURAL_LIGHT_INTENSITY: f32 = 7.4;

// Fitted to the game's map art: the map lights' color, then a curve on each captured tile. The curve's powers of
// x, from x to x⁴: it darkens grass and trees and keeps sand where it is, then levels off so bright sand and snow
// end near 0.8 instead of clipping to white.
const MAP_LIGHT_COLOR: LinearColor = LinearColor { r: 0.4, g: 0.5, b: 1.0, a: 1.0 };
const NATURAL_LIGHT_COLOR: LinearColor = LinearColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
const CURVE: [f32; 4] = [0.0295, 5.2278, -7.6441, 3.1868];
// Water is found by capturing the tile again with it hidden: wherever the two differ by more than 1/50, it is
// painted the game's map water color, fading out where shallow water barely changes the ground under it.
const WATER_MASK_GAIN: f32 = 50.0;
// The mask is blurred this many pixels each way, so shores fade over a few metres like the game's map, and the
// fading band gets the lighter shallow color.
const SHORE_BLUR_PIXELS: i32 = 3;
const WATER_COLOR: LinearColor = LinearColor { r: 0.0056, g: 0.091, b: 0.122, a: 1.0 };
const SHALLOW_WATER_COLOR: LinearColor = LinearColor { r: 0.0097, g: 0.156, b: 0.223, a: 1.0 };
const TEXTURE_CLAMP: u8 = 1;
const WATER_CLASSES: [&str; 2] = ["/Script/Water.WaterBody", "/Script/Water.WaterZone"];
const SIMPLE_WATER_CLASS: &str = "BP_SimpleWater_C";
const SIMPLE_WATER_TRIES: u32 = 3;
const SIMPLE_WATER_RETRY: Duration = Duration::from_secs(30);

const SCENE_COLOR_SCENE_DEPTH: u8 = 3;
const ORTHOGRAPHIC: u8 = 1;
const RGBA8_SRGB: u8 = 3;
const RGBA16F: u8 = 6;
const BLEND_OPAQUE: u8 = 0;
const BLEND_ADDITIVE: u8 = 3;
const BLEND_MODULATE: u8 = 4;
const AUTO_EXPOSURE_MANUAL: u8 = 2;
const GLOBAL_ILLUMINATION_NONE: u8 = 0;
const REFLECTIONS_NONE: u8 = 0;
const MOVABLE: u8 = 2;
const TRACE_VISIBILITY: u8 = 0;
const DRAW_DEBUG_NONE: u8 = 0;
// Meshes pick their detail level by distance to the camera, which differs a lot between tiles (the camera sits over
// each tile's highest ground). Treating everything as near keeps every tile on the detailed versions, whose
// ground textures the far versions drop. Captures are rare, so the extra detail costs little.
const LOD_DISTANCE_FACTOR: f32 = 0.1;
const LOOK_DOWN: Rotator = Rotator { pitch: -90.0, yaw: 0.0, roll: 0.0 };
const BLACK: LinearColor = LinearColor { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Transform {
    rotation: [f64; 4],
    translation: [f64; 3],
    pad0: f64,
    scale: [f64; 3],
    pad1: f64,
}

const IDENTITY: Transform = Transform {
    rotation: [0.0, 0.0, 0.0, 1.0],
    translation: [0.0; 3],
    pad0: 0.0,
    scale: [1.0; 3],
    pad1: 0.0,
};

/// Which height a tile was captured for: the open surface from high up, or a 10 m slice of height from just
/// above it, so cave and building ceilings are not in the picture.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Layer {
    Surface,
    Band(i32),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct TileKey {
    x: i32,
    y: i32,
    level: u8,
    layer: Layer,
}

fn level_units(level: u8) -> f64 {
    TILE_UNITS * f64::from(1u32 << level)
}

/// The coarsest level still at least half the view wide, so 3 or 4 tiles cover it.
fn level_for(view_units: f64) -> u8 {
    let mut level = 0;
    while level < MAX_LEVEL && level_units(level) * 2.0 < view_units {
        level += 1;
    }
    level
}

impl TileKey {
    fn units(self) -> f64 {
        level_units(self.level)
    }

    /// Distance from a world point to the tile's square, 0 inside it.
    fn distance(self, x: f64, y: f64) -> f64 {
        let units = self.units();
        let gap = |value: f64, index: i32| {
            let low = f64::from(index) * units;
            (low - value).max(value - (low + units)).max(0.0)
        };
        gap(x, self.x).hypot(gap(y, self.y))
    }
}

/// Parts of the game's sky actor (PPSkyCreator) that a lit capture leaves out: clouds and height fog (hidden),
/// rain and snow (never in the capture), and the sun, moon and sky light (dimmed to nothing, the map light
/// stands in for them).
#[derive(Clone, Copy, Default)]
struct Sky {
    cloud: Option<Weak>,
    fog: Option<Weak>,
    weather: Option<Weak>,
    lights: [Option<Weak>; 3],
}

/// What a lit capture changed for its moment, to put back right after.
struct SkyPause {
    hidden: Vec<Obj>,
    dimmed: Vec<(Obj, f32)>,
    map_lights: [Obj; 2],
}

/// The ground's height range across a tile, from the survey.
#[derive(Clone, Copy, Debug)]
struct Ground {
    lowest: f64,
    highest: f64,
}

/// The look fitted to the game's map art: a tinted map light, and a contrast curve drawn over each tile on the GPU.
struct MapStyle {
    set_light_color: Function,
    begin_draw: Function,
    end_draw: Function,
    draw_texture: Function,
    target_gamma_offset: usize,
    address_offsets: [usize; 2],
    resize_target: Function,
    clear_target: Function,
    clear_hidden: Function,
    all_actors: Function,
    /// The Water plugin's classes, when the game has them loaded.
    water_classes: Vec<Obj>,
    /// The game's own water class, a Blueprint, looked up once it is loaded.
    simple_water: Option<Weak>,
    simple_water_tries: u32,
    next_simple_water_try: Instant,
    water_report: String,
    /// A float target, so nothing clips between steps, and the widget holding it.
    scratch: Option<(Weak, Weak)>,
    /// The capture with water hidden, later the water mask, and the widget holding it.
    dry: Option<(Weak, Weak)>,
    /// The water mask squared, and the widget holding it.
    squared: Option<(Weak, Weak)>,
    broken: bool,
}

/// The targets the map look works in.
#[derive(Clone, Copy)]
enum Held {
    Scratch,
    Dry,
    Squared,
}

const fn gray(value: f32) -> LinearColor {
    LinearColor { r: value, g: value, b: value, a: 1.0 }
}

fn add(a: LinearColor, b: LinearColor, scale: f32) -> LinearColor {
    LinearColor { r: a.r + b.r * scale, g: a.g + b.g * scale, b: a.b + b.b * scale, a: 1.0 }
}

/// One canvas draw: a texture over the whole target, multiplied by a color, with a blend mode, its
/// texture read shifted by some pixels.
#[derive(Clone, Copy)]
struct Pass {
    texture: Obj,
    color: LinearColor,
    blend: u8,
    shift: Vec2,
}

fn pass(texture: Obj, color: LinearColor, blend: u8) -> Pass {
    Pass { texture, color, blend, shift: Vec2::default() }
}

/// A box blur along one axis: the first tap replaces what is in the target, the rest add to it.
fn blur_passes(texture: Obj, horizontal: bool) -> Vec<Pass> {
    let weight = gray(1.0 / (2 * SHORE_BLUR_PIXELS + 1) as f32);
    (-SHORE_BLUR_PIXELS..=SHORE_BLUR_PIXELS)
        .enumerate()
        .map(|(index, offset)| {
            let step = f64::from(offset);
            Pass {
                texture,
                color: weight,
                blend: if index == 0 { BLEND_OPAQUE } else { BLEND_ADDITIVE },
                shift: if horizontal { Vec2 { x: step, y: 0.0 } } else { Vec2 { x: 0.0, y: step } },
            }
        })
        .collect()
}

impl MapStyle {
    fn new(rendering: Obj, light_class: Obj, capture_base: Obj, statics: Obj) -> Result<Self, String> {
        let canvas = ue::find("/Script/Engine.Canvas")?;
        let water_classes: Vec<Obj> = WATER_CLASSES.iter().filter_map(|path| ue::find(path).ok()).collect();
        let target_class = ue::find("/Script/Engine.TextureRenderTarget2D")?;
        Ok(Self {
            address_offsets: [ue::property_offset(target_class, "AddressX")?, ue::property_offset(target_class, "AddressY")?],
            resize_target: Function::resolve(rendering, "ResizeRenderTarget2D", &["TextureRenderTarget", "Width", "Height"])?,
            squared: None,
            clear_target: Function::resolve(rendering, "ClearRenderTarget2D", &["WorldContextObject", "TextureRenderTarget", "ClearColor"])?,
            clear_hidden: Function::resolve(capture_base, "ClearHiddenComponents", &[])?,
            all_actors: Function::resolve(statics, "GetAllActorsOfClass", &["WorldContextObject", "actorClass", "OutActors"])?,
            water_classes,
            simple_water: None,
            simple_water_tries: 0,
            next_simple_water_try: Instant::now(),
            water_report: String::new(),
            dry: None,
            set_light_color: Function::resolve(light_class, "SetLightColor", &["NewLightColor", "bSRGB"])?,
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
            target_gamma_offset: ue::property_offset(ue::find("/Script/Engine.TextureRenderTarget")?, "TargetGamma")?,
            scratch: None,
            broken: false,
        })
    }

    fn held(&mut self, which: Held) -> &mut Option<(Weak, Weak)> {
        match which {
            Held::Scratch => &mut self.scratch,
            Held::Dry => &mut self.dry,
            Held::Squared => &mut self.squared,
        }
    }

    fn world_unload(&mut self) {
        self.scratch = None;
        self.dry = None;
        self.squared = None;
        self.simple_water_tries = 0;
        self.next_simple_water_try = Instant::now();
        self.water_report.clear();
    }

    /// Draws each pass over the whole of `target`.
    fn draw(&mut self, rendering: Obj, world: Obj, target: Obj, passes: &[Pass]) -> Result<(), String> {
        self.begin_draw.call(rendering, |a| {
            a.set_obj(0, Some(world));
            a.set_obj(1, Some(target));
        });
        let canvas = Obj::from_raw(self.begin_draw.param::<*mut c_void>(2));
        let context = self.begin_draw.param_bytes(4, 16);
        if let Some(canvas) = canvas {
            let size = f64::from(TILE_PIXELS);
            for pass in passes {
                self.draw_texture.call(canvas, |a| {
                    a.set_obj(0, Some(pass.texture));
                    a.set(1, Vec2::default());
                    a.set(2, Vec2 { x: size, y: size });
                    a.set(3, Vec2 { x: pass.shift.x / size, y: pass.shift.y / size });
                    a.set(4, Vec2 { x: 1.0, y: 1.0 });
                    a.set(5, pass.color);
                    a.set(6, pass.blend);
                    a.set(7, 0.0f32);
                    a.set(8, Vec2 { x: 0.5, y: 0.5 });
                });
            }
        }
        self.end_draw.call(rendering, |a| {
            a.set_obj(0, Some(world));
            a.set_bytes(1, &context);
        });
        canvas.map(|_| ()).ok_or_else(|| "BeginDrawCanvasToRenderTarget gave no canvas".to_string())
    }
}

struct Tile {
    texture: Weak,
    holder: Weak,
    outside: bool,
    last_seen: Instant,
    /// Captured while the area was still streaming in, so captured again once it has settled.
    early: bool,
}

/// Terrain rendered from above in tiles, where the game's map art cannot show where the player is: dungeons,
/// caves and, if asked for, the overworld. Each tile is captured once and kept, so the cost is one scene
/// render per new tile, spaced out, and nothing at all while no new tile is needed.
///
/// Lit tiles are drawn with a fixed light and a fixed exposure, with the game's sun, moon, sky light, clouds,
/// fog, weather, shadows and global illumination left out, so nothing in a tile depends on when it was made.
pub struct Terrain {
    rendering: Obj,
    system: Obj,
    capture_class: Obj,
    map_light_class: Obj,
    sky_light_class: Obj,
    add_component: Function,
    finish_component: Function,
    destroy_component: Function,
    place: Function,
    capture: Function,
    hide_actor: Function,
    create_target: Function,
    trace: Function,
    actor_of_class: Function,
    set_visibility: Function,
    is_visible: Function,
    hide_component: Function,
    set_mobility: Function,
    set_light_intensity: Function,
    set_sky_light_intensity: Function,
    statics: Obj,
    /// The sky actor's class and where its cloud, fog, weather, sun, moon and sky light sit, when the game has it.
    sky_class: Option<(Obj, [usize; 6])>,
    post_process: Obj,
    post_process_offset: usize,
    exposure_method_offset: usize,
    exposure_bias_offset: usize,
    global_illumination_offset: usize,
    reflection_offset: usize,
    intensity_offset: usize,
    impact_offset: usize,
    sky: Option<Sky>,
    style: Option<MapStyle>,
    /// Whether the kept tiles and the map lights have the map look, once known.
    styled: Option<bool>,
    source_offset: usize,
    projection_offset: usize,
    ortho_offset: usize,
    target_offset: usize,
    lod_offset: usize,
    view_distance_offset: usize,
    component: Option<(Weak, Weak)>,
    /// The key and fill lights, and the pawn they are on.
    map_lights: Option<([Weak; 2], Weak)>,
    failures: u32,
    broken: bool,
    tiles: HashMap<TileKey, Tile>,
    slots: Vec<Option<TileKey>>,
    layer: Option<Layer>,
    outside: bool,
    band: Option<i32>,
    next_check: Instant,
    next_capture: Instant,
    settle_until: Instant,
    warm_until: Instant,
    last_position: Option<Vec3>,
    region_name: &'static str,
}

impl Terrain {
    pub fn new() -> Result<Self, String> {
        let actor = ue::find("/Script/Engine.Actor")?;
        let actor_component = ue::find("/Script/Engine.ActorComponent")?;
        let scene_component = ue::find("/Script/Engine.SceneComponent")?;
        let capture_base = ue::find("/Script/Engine.SceneCaptureComponent")?;
        let capture_class = ue::find("/Script/Engine.SceneCaptureComponent2D")?;
        let light_base = ue::find("/Script/Engine.LightComponentBase")?;
        let light_class = ue::find("/Script/Engine.LightComponent")?;
        let map_light_class = ue::find("/Script/Engine.DirectionalLightComponent")?;
        let sky_light_class = ue::find("/Script/Engine.SkyLightComponent")?;
        let rendering = ue::find("/Script/Engine.Default__KismetRenderingLibrary")?;
        let system = ue::find("/Script/Engine.Default__KismetSystemLibrary")?;
        let statics = ue::find("/Script/Engine.Default__GameplayStatics")?;
        let sky_class = ue::find("/Script/PPSkyCreatorPlugin.PPSkyCreator").and_then(|class| {
            let offset = |name| ue::property_offset(class, name);
            Ok((
                class,
                [
                    offset("VolumetricCloud")?,
                    offset("ExponentialHeightFog")?,
                    offset("WeatherFX")?,
                    offset("Sunlight")?,
                    offset("MoonLight")?,
                    offset("SkyLight")?,
                ],
            ))
        });
        if let Err(reason) = &sky_class {
            host::warning(&format!("lit terrain cannot leave out the game's sky: {reason}"));
        }
        let post_process = ue::find("/Script/Engine.PostProcessSettings")?;
        let style = MapStyle::new(rendering, light_class, capture_base, statics)
            .inspect_err(|reason| host::warning(&format!("terrain tiles keep their natural colors: {reason}")))
            .ok();
        let now = Instant::now();
        Ok(Self {
            actor_of_class: Function::resolve(statics, "GetActorOfClass", &["WorldContextObject", "actorClass"])?,
            set_visibility: Function::resolve(scene_component, "SetVisibility", &["bNewVisibility", "bPropagateToChildren"])?,
            is_visible: Function::resolve(scene_component, "IsVisible", &[])?,
            hide_component: Function::resolve(capture_base, "HideComponent", &["InComponent"])?,
            set_mobility: Function::resolve(scene_component, "SetMobility", &["NewMobility"])?,
            set_light_intensity: Function::resolve(light_class, "SetIntensity", &["NewIntensity"])?,
            set_sky_light_intensity: Function::resolve(sky_light_class, "SetIntensity", &["NewIntensity"])?,
            intensity_offset: ue::property_offset(light_base, "Intensity")?,
            impact_offset: ue::property_offset(ue::find("/Script/Engine.HitResult")?, "ImpactPoint")?,
            sky_class: sky_class.ok(),
            post_process_offset: ue::property_offset(capture_class, "PostProcessSettings")?,
            exposure_method_offset: ue::property_offset(post_process, "AutoExposureMethod")?,
            exposure_bias_offset: ue::property_offset(post_process, "AutoExposureBias")?,
            global_illumination_offset: ue::property_offset(post_process, "DynamicGlobalIlluminationMethod")?,
            reflection_offset: ue::property_offset(post_process, "ReflectionMethod")?,
            statics,
            post_process,
            sky: None,
            style,
            styled: None,
            add_component: Function::resolve(
                actor,
                "AddComponentByClass",
                &["Class", "bManualAttachment", "RelativeTransform", "bDeferredFinish"],
            )?,
            finish_component: Function::resolve(actor, "FinishAddComponent", &["Component", "bManualAttachment", "RelativeTransform"])?,
            destroy_component: Function::resolve(actor_component, "K2_DestroyComponent", &["Object"])?,
            place: Function::resolve(
                scene_component,
                "K2_SetWorldLocationAndRotation",
                &["NewLocation", "NewRotation", "bSweep", "SweepHitResult", "bTeleport"],
            )?,
            capture: Function::resolve(capture_class, "CaptureScene", &[])?,
            hide_actor: Function::resolve(capture_base, "HideActorComponents", &["InActor", "bIncludeFromChildActors"])?,
            create_target: Function::resolve(
                rendering,
                "CreateRenderTarget2D",
                &["WorldContextObject", "Width", "Height", "Format", "ClearColor", "bAutoGenerateMipMaps"],
            )?,
            trace: Function::resolve(
                system,
                "LineTraceSingle",
                &[
                    "WorldContextObject",
                    "Start",
                    "End",
                    "TraceChannel",
                    "bTraceComplex",
                    "DrawDebugType",
                    "bIgnoreSelf",
                    "OutHit",
                ],
            )?,
            source_offset: ue::property_offset(capture_base, "CaptureSource")?,
            lod_offset: ue::property_offset(capture_base, "LODDistanceFactor")?,
            view_distance_offset: ue::property_offset(capture_base, "MaxViewDistanceOverride")?,
            projection_offset: ue::property_offset(capture_class, "ProjectionType")?,
            ortho_offset: ue::property_offset(capture_class, "OrthoWidth")?,
            target_offset: ue::property_offset(capture_class, "TextureTarget")?,
            rendering,
            system,
            capture_class,
            map_light_class,
            sky_light_class,
            component: None,
            map_lights: None,
            failures: 0,
            broken: false,
            tiles: HashMap::new(),
            slots: Vec::new(),
            layer: None,
            outside: false,
            band: None,
            next_check: now,
            next_capture: now,
            settle_until: now,
            warm_until: now,
            last_position: None,
            region_name: "",
        })
    }

    /// The minimap was rebuilt: its tile widgets, and with them the tiles, are gone.
    pub fn forget_tiles(&mut self) {
        self.tiles.clear();
        self.slots.clear();
        self.layer = None;
        self.next_check = Instant::now();
    }

    pub fn world_unload(&mut self) {
        self.forget_tiles();
        self.component = None;
        self.map_lights = None;
        self.sky = None;
        if let Some(style) = &mut self.style {
            style.world_unload();
        }
        self.failures = 0;
        self.broken = false;
        self.outside = false;
        self.band = None;
        self.last_position = None;
    }

    pub fn update(
        &mut self,
        w: &mut Widgets,
        layers: &mut Layers,
        region: &'static Region,
        player: &Player,
        settings: &Settings,
        view_units: f64,
    ) {
        let now = Instant::now();
        if now < self.next_check && region.name == self.region_name {
            return;
        }
        self.next_check = now + CHECK_EVERY;
        self.region_name = region.name;

        let styled = settings.map_colors && self.style.as_ref().is_some_and(|style| !style.broken);
        if self.styled.replace(styled).is_some_and(|before| before != styled) {
            // The kept tiles have the other look, and the lights have the other color and brightness.
            self.drop_tiles(w, layers, |_| true);
            self.color_map_lights();
            host::info(if styled { "terrain colored like the game's map" } else { "terrain in natural colors" });
        }

        let at = player.location;
        match self.last_position {
            None => self.warm_until = now + WARMUP,
            Some(before) if (at.x - before.x).hypot(at.y - before.y) > JUMP => {
                self.settle_until = now + SETTLE;
                self.warm_until = now + WARMUP;
            }
            Some(_) => {}
        }
        self.last_position = Some(at);

        // A dungeon: inside a stage (which may sit under the overworld), or off every map region.
        let outside = player.in_stage || !worldmap::REGIONS.iter().any(|known| known.contains(at.x, at.y));
        if outside && !self.outside {
            // Another dungeon can be put in the same place, so what was captured there before is stale.
            self.drop_tiles(w, layers, |tile| tile.outside);
            self.settle_until = now + SETTLE;
            self.warm_until = now + WARMUP;
        }
        self.outside = outside;

        let layer = self.wanted_layer(player, settings, view_units);
        if layer != self.layer {
            let why = match layer {
                None => "off".to_string(),
                Some(Layer::Surface) => "on (overworld)".to_string(),
                Some(Layer::Band(band)) => format!("on (dungeon, height band {band})"),
            };
            host::info(&format!("rendered terrain {why}"));
            self.layer = layer;
        }
        let Some(layer) = layer else {
            self.show(w, layers, region, &[]);
            return;
        };

        let level = level_for(view_units);
        let radius = view_units * SHOWN_RADIUS;
        let units = level_units(level);
        let cell = |value: f64| (value / units).floor() as i32;
        let mut wanted: Vec<(f64, TileKey)> = Vec::new();
        for x in cell(at.x - radius)..=cell(at.x + radius) {
            for y in cell(at.y - radius)..=cell(at.y + radius) {
                let key = TileKey { x, y, level, layer };
                let distance = key.distance(at.x, at.y);
                if distance <= radius {
                    wanted.push((distance, key));
                }
            }
        }
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));

        self.tiles.retain(|_, tile| tile.texture.get().is_some());
        let mut missing = None;
        if now >= self.next_capture && now >= self.settle_until {
            let settled = now >= self.warm_until;
            let candidates: Vec<TileKey> = wanted
                .iter()
                .map(|(_, key)| *key)
                .filter(|key| self.tiles.get(key).is_none_or(|tile| tile.early && settled))
                .take(LOAD_CHECKS_PER_TICK)
                .collect();
            missing = candidates
                .into_iter()
                .find_map(|key| profile::time("terrain survey", || self.survey(player.world, key)).map(|ground| (key, ground)));
        }
        if let Some((key, ground)) = missing {
            self.next_capture = now + CAPTURE_GAP;
            match profile::time("terrain capture", || self.capture_tile(w, layers, player, key, ground)) {
                Ok(()) => self.failures = 0,
                Err(reason) => {
                    self.failures += 1;
                    host::warning(&format!("terrain tile not rendered: {reason}"));
                    if self.failures >= MAX_FAILURES {
                        host::warning("rendered terrain turned off for this session, the game's map art is used instead");
                        self.broken = true;
                    }
                }
            }
        }

        // Tiles at this zoom's level first, then what other levels already have there, so a zoom change shows
        // the old tiles until the new ones are in.
        let mut shown: Vec<(u8, f64, TileKey)> = self
            .tiles
            .keys()
            .filter(|key| key.layer == layer)
            .filter_map(|&key| {
                let distance = key.distance(at.x, at.y);
                (distance <= radius).then_some((key.level.abs_diff(level), distance, key))
            })
            .collect();
        shown.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        shown.truncate(MAX_SHOWN);
        let shown: Vec<TileKey> = shown.into_iter().map(|(_, _, key)| key).collect();
        for key in &shown {
            if let Some(tile) = self.tiles.get_mut(key) {
                tile.last_seen = now;
            }
        }
        self.show(w, layers, region, &shown);
    }

    /// Dungeons are drawn in height bands, so their roofs are left out. The overworld is drawn from over its
    /// highest ground, so nothing is cut off.
    fn wanted_layer(&mut self, player: &Player, settings: &Settings, view_units: f64) -> Option<Layer> {
        if self.broken {
            return None;
        }
        let allowed = |terrain: TerrainUse| match terrain {
            TerrainUse::Off => false,
            TerrainUse::On => true,
            TerrainUse::Close => view_units <= CLOSE_VIEW,
        };
        if self.outside {
            return allowed(settings.dungeon_terrain).then(|| Layer::Band(self.band_for(player.location.z)));
        }
        allowed(settings.overworld_terrain).then_some(Layer::Surface)
    }

    /// The height band the player is in. Stays put within a couple of metres of its edges, so walking up and
    /// down a slope does not keep starting new tiles.
    fn band_for(&mut self, z: f64) -> i32 {
        if let Some(band) = self.band {
            let base = f64::from(band) * BAND_UNITS;
            if z >= base - BAND_SLACK && z < base + BAND_UNITS + BAND_SLACK {
                return band;
            }
        }
        let band = (z / BAND_UNITS).floor() as i32;
        self.band = Some(band);
        band
    }

    /// The tile's ground, from rays straight down across it on a grid, or None while any of them finds nothing:
    /// the game has not loaded all of it yet, and a capture then would keep holes for good. Rays start high
    /// above the tile's middle, so they find the top of whatever is there.
    fn survey(&mut self, world: Obj, key: TileKey) -> Option<Ground> {
        let units = key.units();
        let (low_x, low_y) = (f64::from(key.x) * units, f64::from(key.y) * units);
        let reference = self.last_position.map_or(0.0, |at| at.z);
        let mut ground: Option<Ground> = None;
        for row in 0..SURVEY_GRID {
            for column in 0..SURVEY_GRID {
                let share = |index: usize| (index as f64 + 0.5) / SURVEY_GRID as f64;
                let (x, y) = (low_x + share(row) * units, low_y + share(column) * units);
                let start = Vec3 { x, y, z: reference + LOAD_CHECK_REACH };
                let end = Vec3 { x, y, z: reference - LOAD_CHECK_REACH };
                let z = self.hit_height(world, start, end)?;
                ground = Some(match ground {
                    Some(known) => Ground { lowest: known.lowest.min(z), highest: known.highest.max(z) },
                    None => Ground { lowest: z, highest: z },
                });
            }
        }
        ground
    }

    /// Where a ray from `start` to `end` first hits something solid, as a height.
    fn hit_height(&mut self, world: Obj, start: Vec3, end: Vec3) -> Option<f64> {
        let system = self.system;
        let hit = self
            .trace
            .call(system, |a| {
                a.set_obj(0, Some(world));
                a.set(1, start);
                a.set(2, end);
                a.set(3, TRACE_VISIBILITY);
                a.set(4, false);
                a.set(5, DRAW_DEBUG_NONE);
                a.set(6, true);
            })
            .returned::<u8>()
            != 0;
        hit.then(|| self.trace.param_field::<Vec3>(7, self.impact_offset).z)
    }

    fn show(&mut self, w: &mut Widgets, layers: &mut Layers, region: &Region, keys: &[TileKey]) {
        for slot in &mut self.slots {
            if slot.is_some_and(|key| !keys.contains(&key)) {
                *slot = None;
            }
        }
        for &key in keys {
            if self.slots.contains(&Some(key)) {
                continue;
            }
            match self.slots.iter().position(Option::is_none) {
                Some(free) => self.slots[free] = Some(key),
                None => self.slots.push(Some(key)),
            }
        }
        let point = |x: f64, y: f64| {
            let (u, v) = region.to_uv(x, y);
            Vec2 { x: u, y: v }
        };
        let views: Vec<Option<TileView>> = self
            .slots
            .iter()
            .map(|slot| {
                let key = (*slot)?;
                let tile = self.tiles.get(&key)?;
                let units = key.units();
                let (low_x, low_y) = (f64::from(key.x) * units, f64::from(key.y) * units);
                // The image's top is the tile's north (+X) edge and its left the west (-Y) edge.
                let anchors = Anchors { min: point(low_x + units, low_y), max: point(low_x, low_y + units) };
                Some(TileView { texture: tile.texture, anchors, z: i32::from(key.level).wrapping_neg() })
            })
            .collect();
        layers.set_tiles(w, &views);
    }

    fn drop_tiles(&mut self, w: &mut Widgets, layers: &mut Layers, mut drop: impl FnMut(&Tile) -> bool) {
        let gone: Vec<TileKey> = self.tiles.iter().filter(|(_, tile)| drop(tile)).map(|(key, _)| *key).collect();
        for key in gone {
            if let Some(tile) = self.tiles.remove(&key) {
                layers.release(w, tile.holder);
            }
            for slot in &mut self.slots {
                if *slot == Some(key) {
                    *slot = None;
                }
            }
        }
    }

    /// Adds a component of `class` to the pawn, set up by `setup` before it is registered. Destroyed again when
    /// `setup` fails.
    fn add_component(&mut self, pawn: Obj, class: Obj, setup: impl FnOnce(&mut Self, Obj) -> Result<(), String>) -> Result<Obj, String> {
        let raw: *mut c_void = self
            .add_component
            .call(pawn, |a| {
                a.set_obj(0, Some(class));
                a.set(1, true);
                a.set(2, IDENTITY);
                a.set(3, true);
            })
            .returned();
        let component = Obj::from_raw(raw).ok_or_else(|| format!("AddComponentByClass made no {}", class.name()))?;
        if let Err(reason) = setup(self, component) {
            self.destroy_component.call(component, |a| a.set_obj(0, Some(component)));
            return Err(reason);
        }
        self.finish_component.call(pawn, |a| {
            a.set_obj(0, Some(component));
            a.set(1, true);
            a.set(2, IDENTITY);
        });
        Ok(component)
    }

    fn destroy(&mut self, made: Option<(Weak, Weak)>) {
        if let Some(old) = made.and_then(|(component, _)| component.get()) {
            self.destroy_component.call(old, |a| a.set_obj(0, Some(old)));
        }
    }

    /// The scene capture on the player's pawn, made when missing. It only ever renders when told to.
    fn component(&mut self, pawn: Obj) -> Result<Obj, String> {
        if let Some((component, owner)) = self.component
            && owner.get() == Some(pawn)
            && let Some(component) = component.get()
        {
            return Ok(component);
        }
        let old = self.component.take();
        self.destroy(old);

        let class = self.capture_class;
        let component = self.add_component(pawn, class, |terrain, component| {
            // Set before the component is registered, or it renders the scene every frame from the start.
            let manual = ue::set_bool_property(component, "bCaptureEveryFrame", false)
                && ue::set_bool_property(component, "bCaptureOnMovement", false)
                && ue::set_bool_property(component, "bAlwaysPersistRenderingState", true);
            if !manual {
                return Err("the scene capture could not be set to render only on request".into());
            }
            unsafe {
                component.write(terrain.projection_offset, ORTHOGRAPHIC);
                component.write(terrain.lod_offset, LOD_DISTANCE_FACTOR);
            }
            terrain.fix_post_process(component)
        })?;
        self.hide_actor.call(component, |a| {
            a.set_obj(0, Some(pawn));
            a.set(1, true);
        });
        self.component = Some((Weak::new(component), Weak::new(pawn)));
        host::info("terrain scene capture made");
        Ok(component)
    }

    /// A fixed exposure, and no global illumination or reflection method that keeps history between captures.
    fn fix_post_process(&mut self, component: Obj) -> Result<(), String> {
        let settings = unsafe { component.raw().cast::<u8>().add(self.post_process_offset) };
        let write = |offset: usize, value: u8| unsafe { settings.add(offset).write(value) };
        write(self.exposure_method_offset, AUTO_EXPOSURE_MANUAL);
        write(self.global_illumination_offset, GLOBAL_ILLUMINATION_NONE);
        write(self.reflection_offset, REFLECTIONS_NONE);
        unsafe { settings.add(self.exposure_bias_offset).cast::<f32>().write_unaligned(0.0) };
        let container = settings.cast::<c_void>();
        let flags = [
            ("bOverride_AutoExposureMethod", true),
            ("bOverride_AutoExposureBias", true),
            ("bOverride_AutoExposureApplyPhysicalCameraExposure", true),
            ("AutoExposureApplyPhysicalCameraExposure", false),
            ("bOverride_DynamicGlobalIlluminationMethod", true),
            ("bOverride_ReflectionMethod", true),
        ];
        for (flag, value) in flags {
            if !unsafe { ue::set_struct_bool(self.post_process, container, flag, value) } {
                host::warning(&format!("terrain capture post process has no {flag}"));
            }
        }
        Ok(())
    }

    /// The lights lit tiles are drawn with: the key light from the north-west, and a softer fill from straight
    /// above so slopes facing away still get some light. Off except for the moment of a capture.
    fn map_lights(&mut self, pawn: Obj) -> Result<[Obj; 2], String> {
        if let Some(([key, fill], owner)) = self.map_lights
            && owner.get() == Some(pawn)
            && let (Some(key), Some(fill)) = (key.get(), fill.get())
        {
            return Ok([key, fill]);
        }
        if let Some((old, owner)) = self.map_lights.take() {
            for light in old {
                self.destroy(Some((light, owner)));
            }
        }
        let key = self.make_map_light(pawn, MAP_LIGHT_ROTATION, 1.0)?;
        let fill = match self.make_map_light(pawn, FILL_LIGHT_ROTATION, FILL_SHARE) {
            Ok(fill) => fill,
            Err(reason) => {
                self.destroy(Some((Weak::new(key), Weak::new(pawn))));
                return Err(reason);
            }
        };
        self.map_lights = Some(([Weak::new(key), Weak::new(fill)], Weak::new(pawn)));
        Ok([key, fill])
    }

    /// The key light's brightness for the current look. The fill is a share of it.
    fn light_intensity(&self) -> f32 {
        if self.styled == Some(true) { MAP_LIGHT_INTENSITY } else { NATURAL_LIGHT_INTENSITY }
    }

    fn make_map_light(&mut self, pawn: Obj, rotation: Rotator, share: f32) -> Result<Obj, String> {
        let class = self.map_light_class;
        let intensity = self.light_intensity() * share;
        let light = self.add_component(pawn, class, |terrain, light| {
            let set = ue::set_bool_property(light, "bVisible", false)
                && ue::set_bool_property(light, "CastShadows", false)
                && ue::set_bool_property(light, "bAtmosphereSunLight", false);
            if !set {
                return Err("the map light could not be set up".into());
            }
            unsafe { light.write(terrain.intensity_offset, intensity) };
            terrain.set_mobility.call(light, |a| a.set(0, MOVABLE));
            Ok(())
        })?;
        self.place.call(light, |a| {
            a.set(0, Vec3::default());
            a.set(1, rotation);
            a.set(2, false);
            a.set(4, true);
        });
        self.color_light(light);
        Ok(light)
    }

    fn color_light(&mut self, light: Obj) {
        let color = if self.styled == Some(true) { MAP_LIGHT_COLOR } else { NATURAL_LIGHT_COLOR };
        if let Some(style) = &mut self.style {
            style.set_light_color.call(light, |a| {
                a.set(0, color);
                a.set(1, true);
            });
        }
    }

    /// Gives both map lights the current look's color and brightness.
    fn color_map_lights(&mut self) {
        let Some((lights, _)) = self.map_lights else { return };
        let intensity = self.light_intensity();
        for (light, share) in lights.into_iter().zip([1.0, FILL_SHARE]) {
            if let Some(light) = light.get() {
                self.color_light(light);
                self.set_light_intensity.call(light, |a| a.set(0, intensity * share));
            }
        }
    }

    /// Leaves the game's sky out of one capture and switches the map light on. This runs after the frame's own
    /// view was drawn, and `resume_sky` puts everything back before the next one, so only the capture ever sees
    /// the change. Light brightness changes are cheap updates, so the game's shadows are not rebuilt.
    fn pause_sky(&mut self, world: Obj, capture: Obj, map_lights: [Obj; 2]) -> SkyPause {
        let sky = self.sky(world);
        // Added only once by the engine, and the capture may have been made again for a new pawn.
        if let Some(weather) = sky.weather.and_then(Weak::get) {
            self.hide_component.call(capture, |a| a.set_obj(0, Some(weather)));
        }
        let hidden: Vec<Obj> = [sky.cloud, sky.fog]
            .into_iter()
            .flatten()
            .filter_map(Weak::get)
            .filter(|&part| self.is_visible.call(part, |_| {}).returned::<u8>() != 0)
            .collect();
        let dimmed: Vec<(Obj, f32)> = sky
            .lights
            .into_iter()
            .flatten()
            .filter_map(Weak::get)
            .map(|light| (light, unsafe { light.read::<f32>(self.intensity_offset) }))
            .filter(|&(_, intensity)| intensity > 0.0)
            .collect();
        for &part in &hidden {
            self.set_visibility.call(part, |a| a.set(0, false));
        }
        for &(light, _) in &dimmed {
            self.set_intensity(light, 0.0);
        }
        for light in map_lights {
            self.set_visibility.call(light, |a| a.set(0, true));
        }
        SkyPause { hidden, dimmed, map_lights }
    }

    fn resume_sky(&mut self, pause: SkyPause) {
        for light in pause.map_lights {
            self.set_visibility.call(light, |a| a.set(0, false));
        }
        for &(light, intensity) in &pause.dimmed {
            self.set_intensity(light, intensity);
        }
        for &part in &pause.hidden {
            self.set_visibility.call(part, |a| a.set(0, true));
        }
    }

    fn set_intensity(&mut self, light: Obj, intensity: f32) {
        let function = if ue::is_a(light, self.sky_light_class) { &mut self.set_sky_light_intensity } else { &mut self.set_light_intensity };
        function.call(light, |a| a.set(0, intensity));
    }

    /// The parts of the game's sky actor a capture leaves out. Looked up once per world.
    fn sky(&mut self, world: Obj) -> Sky {
        if self.sky.is_none() {
            let Some((class, [cloud, fog, weather, sun, moon, sky_light])) = self.sky_class else { return Sky::default() };
            let statics = self.statics;
            let actor: *mut c_void = self
                .actor_of_class
                .call(statics, |a| {
                    a.set_obj(0, Some(world));
                    a.set_obj(1, Some(class));
                })
                .returned();
            let part = |offset: usize| Obj::from_raw(actor).and_then(|actor| unsafe { actor.read_obj(offset) }).map(Weak::new);
            let sky = Sky {
                cloud: part(cloud),
                fog: part(fog),
                weather: part(weather),
                lights: [part(sun), part(moon), part(sky_light)],
            };
            let found = |part: Option<Weak>| if part.is_some() { "left out" } else { "none" };
            host::info(&format!(
                "sky for lit terrain: {}, clouds {}, height fog {}, rain and snow {}, sun {}, moon {}, sky light {}",
                if actor.is_null() { "no sky actor" } else { "sky actor found" },
                found(sky.cloud),
                found(sky.fog),
                found(sky.weather),
                found(sky.lights[0]),
                found(sky.lights[1]),
                found(sky.lights[2]),
            ));
            self.sky = Some(sky);
        }
        self.sky.unwrap_or_default()
    }

    fn new_target(&mut self, world: Obj, format: u8) -> Result<Obj, String> {
        self.new_target_sized(world, format, TILE_PIXELS)
    }

    fn new_target_sized(&mut self, world: Obj, format: u8, size: i32) -> Result<Obj, String> {
        let rendering = self.rendering;
        let raw: *mut c_void = self
            .create_target
            .call(rendering, |a| {
                a.set_obj(0, Some(world));
                a.set(1, size);
                a.set(2, size);
                a.set(3, format);
                a.set(4, BLACK);
                a.set(5, false);
            })
            .returned();
        let target = Obj::from_raw(raw).ok_or_else(|| "CreateRenderTarget2D made no render target".to_string())?;
        if let Some(style) = &self.style {
            // Canvas draws stay in linear light, so the curve means what it says.
            unsafe { target.write(style.target_gamma_offset, 1.0f32) };
        }
        Ok(target)
    }

    /// One of the map look's targets, made and kept on a widget when missing.
    fn held_target(&mut self, w: &mut Widgets, layers: &mut Layers, world: Obj, which: Held) -> Result<Obj, String> {
        let Some(style) = &mut self.style else { return Err("the map look is not available".into()) };
        if let Some(target) = style.held(which).and_then(|(target, holder)| holder.get().and(target.get())) {
            return Ok(target);
        }
        let format = match which {
            Held::Scratch => RGBA16F,
            Held::Dry | Held::Squared => RGBA8_SRGB,
        };
        // Made small, then clamped and resized: the sampler is only built with the resource, and the resize
        // rebuilds it. Clamped, the blur's shifted reads repeat the edge instead of the tile's other side.
        let target = self.new_target_sized(world, format, 8)?;
        let rendering = self.rendering;
        if let Some(style) = &mut self.style {
            for offset in style.address_offsets {
                unsafe { target.write(offset, TEXTURE_CLAMP) };
            }
            style.resize_target.call(rendering, |a| {
                a.set_obj(0, Some(target));
                a.set(1, TILE_PIXELS);
                a.set(2, TILE_PIXELS);
            });
        }
        let holder = layers.hold(w, target).ok_or("the minimap has nowhere to keep the map look's targets")?;
        if let Some(style) = &mut self.style {
            *style.held(which) = Some((Weak::new(target), holder));
        }
        Ok(target)
    }

    /// Every water actor in the world: the Water plugin's water bodies and zones, and the game's own water.
    fn water_actors(&mut self, world: Obj) -> Vec<Obj> {
        let statics = self.statics;
        let Some(style) = &mut self.style else { return Vec::new() };
        let now = Instant::now();
        if style.simple_water.and_then(Weak::get).is_none()
            && style.simple_water_tries < SIMPLE_WATER_TRIES
            && now >= style.next_simple_water_try
        {
            style.simple_water_tries += 1;
            style.next_simple_water_try = now + SIMPLE_WATER_RETRY;
            style.simple_water =
                profile::time("water class lookup", || ue::find_instance(SIMPLE_WATER_CLASS)).and_then(Obj::class).map(Weak::new);
        }
        let classes: Vec<Obj> = style.water_classes.iter().copied().chain(style.simple_water.and_then(Weak::get)).collect();
        let mut actors = Vec::new();
        let mut report = Vec::new();
        for class in classes {
            style.all_actors.call_reusing(statics, |a| {
                a.set_obj(0, Some(world));
                a.set_obj(1, Some(class));
            });
            let found = style.all_actors.param_objects(2);
            report.push(format!("{} {}", found.len(), class.name()));
            actors.extend(found);
        }
        let report = if report.is_empty() { "no water classes loaded".to_string() } else { report.join(", ") };
        if report != style.water_report {
            host::info(&format!("water hidden for the terrain water mask: {report}"));
            style.water_report = report;
        }
        actors
    }

    /// The map look over a captured tile, all on the GPU. With `dry` (the same capture with water hidden): a
    /// mask m of where the two differ, blurred. Then the curve, worked out from the highest power down with a
    /// white target as the constant. Then the water: T·(1 − m) + shallow·m + (deep − shallow)·m², deep inside the
    /// water and shallow along its soft edge.
    fn grade(&mut self, w: &mut Widgets, layers: &mut Layers, world: Obj, tile: Obj, dry: Option<Obj>) -> Result<(), String> {
        if self.styled != Some(true) {
            return Ok(());
        }
        let scratch = self.held_target(w, layers, world, Held::Scratch)?;
        let squared = self.held_target(w, layers, world, Held::Squared)?;
        let rendering = self.rendering;
        let Some(style) = &mut self.style else { return Ok(()) };
        if let Some(dry) = dry {
            style.draw(
                rendering,
                world,
                scratch,
                &[pass(tile, gray(WATER_MASK_GAIN), BLEND_OPAQUE), pass(dry, gray(-WATER_MASK_GAIN), BLEND_ADDITIVE)],
            )?;
            // The dry target clamps each write to 0..1, so this leaves min(1, gain·|wet − dry|) in it.
            style.draw(rendering, world, dry, &[pass(scratch, gray(1.0), BLEND_OPAQUE), pass(scratch, gray(-1.0), BLEND_ADDITIVE)])?;
            style.draw(rendering, world, scratch, &blur_passes(dry, true))?;
            style.draw(rendering, world, dry, &blur_passes(scratch, false))?;
        }
        // The squared target is free until the water step, so it stands in as white here.
        let white = squared;
        style.clear_target.call(rendering, |a| {
            a.set_obj(0, Some(world));
            a.set_obj(1, Some(white));
            a.set(2, gray(1.0));
        });
        let [linear, square, cube, fourth] = CURVE;
        style.draw(
            rendering,
            world,
            scratch,
            &[
                pass(white, gray(fourth), BLEND_OPAQUE),
                pass(tile, gray(1.0), BLEND_MODULATE),
                pass(white, gray(cube), BLEND_ADDITIVE),
                pass(tile, gray(1.0), BLEND_MODULATE),
                pass(white, gray(square), BLEND_ADDITIVE),
                pass(tile, gray(1.0), BLEND_MODULATE),
                pass(white, gray(linear), BLEND_ADDITIVE),
                pass(tile, gray(1.0), BLEND_MODULATE),
            ],
        )?;
        style.draw(rendering, world, tile, &[pass(scratch, gray(1.0), BLEND_OPAQUE)])?;
        if let Some(mask) = dry {
            style.draw(rendering, world, squared, &[pass(mask, gray(1.0), BLEND_OPAQUE), pass(mask, gray(1.0), BLEND_MODULATE)])?;
            style.clear_target.call(rendering, |a| {
                a.set_obj(0, Some(world));
                a.set_obj(1, Some(scratch));
                a.set(2, gray(1.0));
            });
            style.draw(
                rendering,
                world,
                scratch,
                &[
                    pass(mask, gray(-1.0), BLEND_ADDITIVE),
                    pass(tile, gray(1.0), BLEND_MODULATE),
                    pass(mask, SHALLOW_WATER_COLOR, BLEND_ADDITIVE),
                    pass(squared, add(WATER_COLOR, SHALLOW_WATER_COLOR, -1.0), BLEND_ADDITIVE),
                ],
            )?;
            style.draw(rendering, world, tile, &[pass(scratch, gray(1.0), BLEND_OPAQUE)])?;
        }
        Ok(())
    }

    /// Renders one tile's square from above into `into`. On the surface the camera goes just over the tile's
    /// highest ground, so nothing is cut off and meshes stay close enough for their detailed versions. Under
    /// something, it goes just over the player's height band, so the roof is left out. With `dry`, it renders
    /// the tile a second time into it with every water actor hidden. Returns how high the camera was over the
    /// tile's lowest ground, in metres, and whether the dry capture was made.
    fn render(&mut self, player: &Player, key: TileKey, ground: Ground, into: Obj, dry: Option<Obj>) -> Result<(f64, bool), String> {
        let component = self.component(player.pawn)?;
        let map_lights = self.map_lights(player.pawn)?;
        let units = key.units();
        let (camera_z, deepest) = match key.layer {
            Layer::Surface => (ground.highest + CAMERA_OVER_GROUND + units * CAMERA_OVER_GROUND_SHARE, ground.lowest),
            Layer::Band(band) => (f64::from(band + 1) * BAND_UNITS + CAMERA_OVER_BAND, player.location.z),
        };
        let view_distance = (camera_z - deepest).max(0.0) + units * 0.71 + VIEW_DISTANCE_MARGIN;
        unsafe {
            component.write(self.source_offset, SCENE_COLOR_SCENE_DEPTH);
            component.write(self.target_offset, into.raw());
            component.write(self.ortho_offset, units as f32);
            component.write(self.view_distance_offset, view_distance as f32);
        }
        let center = Vec3 {
            x: (f64::from(key.x) + 0.5) * units,
            y: (f64::from(key.y) + 0.5) * units,
            z: camera_z,
        };
        self.place.call(component, |a| {
            a.set(0, center);
            a.set(1, LOOK_DOWN);
            a.set(2, false);
            a.set(4, true);
        });
        let water = if dry.is_some() { self.water_actors(player.world) } else { Vec::new() };
        let pause = self.pause_sky(player.world, component, map_lights);
        self.capture.call(component, |_| {});
        let dry = dry.filter(|_| !water.is_empty());
        if let Some(dry) = dry {
            unsafe { component.write(self.target_offset, dry.raw()) };
            for &actor in &water {
                self.hide_actor.call(component, |a| {
                    a.set_obj(0, Some(actor));
                    a.set(1, true);
                });
            }
            self.capture.call(component, |_| {});
            // Back to hiding only the pawn. The weather is hidden again by the next capture's sky pause.
            if let Some(style) = &mut self.style {
                style.clear_hidden.call(component, |_| {});
            }
            self.hide_actor.call(component, |a| {
                a.set_obj(0, Some(player.pawn));
                a.set(1, true);
            });
        }
        self.resume_sky(pause);
        Ok(((camera_z - deepest) / 100.0, dry.is_some()))
    }

    fn capture_tile(&mut self, w: &mut Widgets, layers: &mut Layers, player: &Player, key: TileKey, ground: Ground) -> Result<(), String> {
        let started = Instant::now();
        let world = player.world;
        let target = self.new_target(world, RGBA8_SRGB)?;
        let holder = layers.hold(w, target).ok_or("the minimap has nowhere to keep the tile")?;
        let dry = if self.styled == Some(true) { self.held_target(w, layers, world, Held::Dry).ok() } else { None };
        let (camera_height, watered) = match self.render(player, key, ground, target, dry) {
            Ok(rendered) => rendered,
            Err(reason) => {
                layers.release(w, holder);
                return Err(reason);
            }
        };
        let dry = dry.filter(|_| watered);
        if let Err(reason) = profile::time("terrain grade", || self.grade(w, layers, world, target, dry)) {
            host::warning(&format!("terrain tiles keep their natural colors from here on: {reason}"));
            if let Some(style) = &mut self.style {
                style.broken = true;
            }
        }

        let early = Instant::now() < self.warm_until;
        let tile = Tile { texture: Weak::new(target), holder, outside: self.outside, last_seen: Instant::now(), early };
        let again = self.tiles.insert(key, tile).map(|old| layers.release(w, old.holder)).is_some();
        self.evict(w, layers);
        host::info(&format!(
            "terrain tile {},{} level {} ({:?}) rendered{}, camera {camera_height:.0} m over the ground's lowest point, {:.2} ms on the game thread, {} tiles kept{}",
            key.x,
            key.y,
            key.level,
            key.layer,
            if again { " again" } else { "" },
            started.elapsed().as_secs_f64() * 1000.0,
            self.tiles.len(),
            if early { ", early, will be captured again" } else { "" }
        ));
        Ok(())
    }

    fn evict(&mut self, w: &mut Widgets, layers: &mut Layers) {
        while self.tiles.len() > MAX_TILES {
            let oldest = self
                .tiles
                .iter()
                .filter(|(key, _)| !self.slots.contains(&Some(**key)))
                .min_by_key(|(_, tile)| tile.last_seen)
                .map(|(key, _)| *key);
            let Some(key) = oldest else { break };
            if let Some(tile) = self.tiles.remove(&key) {
                layers.release(w, tile.holder);
            }
        }
    }
}
