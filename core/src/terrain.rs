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
const MAP_LIGHT_START: f32 = 10.0;
const MAP_LIGHT_RANGE: (f32, f32) = (0.01, 500.0);
// Exposure is fixed, but what a fixed exposure comes out at is up to the game's post process setup. So the map
// light is scaled once per world from a test capture: these many warm-up captures, then one is read back and
// the light is set so this share of the ground's pixels is darker than the target brightness.
const CALIBRATION_WARMUP: u32 = 2;
const CALIBRATION_PERCENTILE: f32 = 0.7;
const CALIBRATION_TARGET: f32 = 0.2;
const CALIBRATION_ROUNDS: u32 = 4;
const CLIPPED: f32 = 0.95;

const SCENE_COLOR_SCENE_DEPTH: u8 = 3;
const ORTHOGRAPHIC: u8 = 1;
const RGBA8_SRGB: u8 = 3;
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

#[derive(Clone, Copy, PartialEq, Debug)]
enum Calibration {
    Warming { done: u32, round: u32 },
    Measuring { round: u32 },
    Done,
}

const CALIBRATION_START: Calibration = Calibration::Warming { done: 0, round: 0 };

/// The ground's height range across a tile, from the survey.
#[derive(Clone, Copy, Debug)]
struct Ground {
    lowest: f64,
    highest: f64,
}

struct Tile {
    texture: Weak,
    holder: Weak,
    outside: bool,
    last_seen: Instant,
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
    source_offset: usize,
    projection_offset: usize,
    ortho_offset: usize,
    target_offset: usize,
    lod_offset: usize,
    view_distance_offset: usize,
    component: Option<(Weak, Weak)>,
    /// The key and fill lights, and the pawn they are on.
    map_lights: Option<([Weak; 2], Weak)>,
    /// The key light's calibrated brightness. The fill is a fixed share of it.
    map_light_intensity: f32,
    calibration: Calibration,
    scratch: Option<Weak>,
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
            map_light_intensity: MAP_LIGHT_START,
            calibration: CALIBRATION_START,
            scratch: None,
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
        self.calibration = CALIBRATION_START;
        self.scratch = None;
        self.sky = None;
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

        let at = player.location;
        if self.last_position.is_some_and(|before| (at.x - before.x).hypot(at.y - before.y) > JUMP) {
            self.settle_until = now + SETTLE;
        }
        self.last_position = Some(at);

        // A dungeon: inside a stage (which may sit under the overworld), or off every map region.
        let outside = player.in_stage || !worldmap::REGIONS.iter().any(|known| known.contains(at.x, at.y));
        if outside && !self.outside {
            // Another dungeon can be put in the same place, so what was captured there before is stale.
            self.drop_tiles(w, layers, |tile| tile.outside);
            self.settle_until = now + SETTLE;
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
            let candidates: Vec<TileKey> =
                wanted.iter().map(|(_, key)| *key).filter(|key| !self.tiles.contains_key(key)).take(LOAD_CHECKS_PER_TICK).collect();
            missing = candidates
                .into_iter()
                .find_map(|key| profile::time("terrain survey", || self.survey(player.world, key)).map(|ground| (key, ground)));
        }
        if let Some((key, ground)) = missing {
            self.next_capture = now + CAPTURE_GAP;
            let result = if self.calibration != Calibration::Done {
                profile::time("terrain calibration", || self.calibrate(player, key, ground))
            } else {
                profile::time("terrain capture", || self.capture_tile(w, layers, player, key, ground))
            };
            match result {
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
        let key = self.make_map_light(pawn, MAP_LIGHT_ROTATION, self.map_light_intensity)?;
        let fill = match self.make_map_light(pawn, FILL_LIGHT_ROTATION, self.map_light_intensity * FILL_SHARE) {
            Ok(fill) => fill,
            Err(reason) => {
                self.destroy(Some((Weak::new(key), Weak::new(pawn))));
                return Err(reason);
            }
        };
        self.map_lights = Some(([Weak::new(key), Weak::new(fill)], Weak::new(pawn)));
        Ok([key, fill])
    }

    /// Sets both map lights to the calibrated brightness, keeping their ratio.
    fn set_map_light_intensity(&mut self, intensity: f32) {
        self.map_light_intensity = intensity;
        let Some((lights, _)) = self.map_lights else { return };
        for (light, share) in lights.into_iter().zip([1.0, FILL_SHARE]) {
            if let Some(light) = light.get() {
                self.set_light_intensity.call(light, |a| a.set(0, intensity * share));
            }
        }
    }

    fn make_map_light(&mut self, pawn: Obj, rotation: Rotator, intensity: f32) -> Result<Obj, String> {
        let class = self.map_light_class;
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
        Ok(light)
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

    fn new_target(&mut self, world: Obj) -> Result<Obj, String> {
        let rendering = self.rendering;
        let raw: *mut c_void = self
            .create_target
            .call(rendering, |a| {
                a.set_obj(0, Some(world));
                a.set(1, TILE_PIXELS);
                a.set(2, TILE_PIXELS);
                a.set(3, RGBA8_SRGB);
                a.set(4, BLACK);
                a.set(5, false);
            })
            .returned();
        Obj::from_raw(raw).ok_or_else(|| "CreateRenderTarget2D made no render target".to_string())
    }

    /// Where calibration captures land. The capture holds on to it while it is its target.
    fn scratch(&mut self, world: Obj) -> Result<Obj, String> {
        if let Some(scratch) = self.scratch.and_then(Weak::get) {
            return Ok(scratch);
        }
        let scratch = self.new_target(world)?;
        self.scratch = Some(Weak::new(scratch));
        Ok(scratch)
    }

    /// Renders one tile's square from above into `into`. On the surface the camera goes just over the tile's
    /// highest ground, so nothing is cut off and meshes stay close enough for their detailed versions. Under
    /// something, it goes just over the player's height band, so the roof is left out. Returns how high the
    /// camera was over the tile's lowest ground, in metres.
    fn render(&mut self, player: &Player, key: TileKey, ground: Ground, into: Obj) -> Result<f64, String> {
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
        let pause = self.pause_sky(player.world, component, map_lights);
        self.capture.call(component, |_| {});
        self.resume_sky(pause);
        Ok((camera_z - deepest) / 100.0)
    }

    /// One step of fitting the map light to the fixed exposure: a few warm-up captures, then the last one is read
    /// back once and the light scaled to bring the ground to a normal brightness. Repeated if the reading was
    /// clipped or far off. Only the reading stalls the game, once or twice per world.
    fn calibrate(&mut self, player: &Player, key: TileKey, ground: Ground) -> Result<(), String> {
        let world = player.world;
        let scratch = self.scratch(world)?;
        match self.calibration {
            Calibration::Warming { done, round } => {
                self.render(player, key, ground, scratch)?;
                self.calibration = if done + 1 >= CALIBRATION_WARMUP {
                    Calibration::Measuring { round }
                } else {
                    Calibration::Warming { done: done + 1, round }
                };
            }
            Calibration::Measuring { round } => {
                let started = Instant::now();
                let mut pixels = Vec::with_capacity((TILE_PIXELS * TILE_PIXELS) as usize);
                ue::read_render_target(world, scratch, &mut pixels)?;
                let read_ms = started.elapsed().as_secs_f64() * 1000.0;
                let Some(level) = ground_level(&pixels) else {
                    return Err("the calibration capture came out empty".into());
                };
                let gain = (CALIBRATION_TARGET / level.max(1e-4)).clamp(1.0 / 64.0, 64.0);
                let before = self.map_light_intensity;
                self.set_map_light_intensity((before * gain).clamp(MAP_LIGHT_RANGE.0, MAP_LIGHT_RANGE.1));
                let off = (gain - 1.0).abs() > 0.25 || level >= CLIPPED;
                let again = off && round + 1 < CALIBRATION_ROUNDS && self.map_light_intensity != before;
                host::info(&format!(
                    "terrain light calibration {}: ground at {level:.3}, light {before:.2} to {:.2}, read back in {read_ms:.1} ms{}",
                    round + 1,
                    self.map_light_intensity,
                    if again { ", checking again" } else { ", done" }
                ));
                self.calibration = if again { Calibration::Warming { done: CALIBRATION_WARMUP - 1, round: round + 1 } } else { Calibration::Done };
            }
            Calibration::Done => {}
        }
        Ok(())
    }

    fn capture_tile(&mut self, w: &mut Widgets, layers: &mut Layers, player: &Player, key: TileKey, ground: Ground) -> Result<(), String> {
        let started = Instant::now();
        let world = player.world;
        let target = self.new_target(world)?;
        let holder = layers.hold(w, target).ok_or("the minimap has nowhere to keep the tile")?;
        let camera_height = match self.render(player, key, ground, target) {
            Ok(height) => height,
            Err(reason) => {
                layers.release(w, holder);
                return Err(reason);
            }
        };

        self.tiles.insert(key, Tile { texture: Weak::new(target), holder, outside: self.outside, last_seen: Instant::now() });
        self.evict(w, layers);
        host::info(&format!(
            "terrain tile {},{} level {} ({:?}) rendered, camera {camera_height:.0} m over the ground's lowest point, {:.2} ms on the game thread, {} tiles kept",
            key.x,
            key.y,
            key.level,
            key.layer,
            started.elapsed().as_secs_f64() * 1000.0,
            self.tiles.len()
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

/// How bright the ground in a capture is: the linear brightness that this share of its non-black pixels stays
/// under. `pixels` are BGRA, stored sRGB-encoded. None when almost everything is black.
fn ground_level(pixels: &[u32]) -> Option<f32> {
    let decode = |channel: u32| {
        let value = channel as f32 / 255.0;
        if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    let mut levels: Vec<f32> = pixels
        .iter()
        .map(|&p| 0.2126 * decode((p >> 16) & 0xFF) + 0.7152 * decode((p >> 8) & 0xFF) + 0.0722 * decode(p & 0xFF))
        .filter(|&level| level > 0.002)
        .collect();
    if levels.len() < pixels.len() / 10 {
        return None;
    }
    levels.sort_by(f32::total_cmp);
    Some(levels[((levels.len() - 1) as f32 * CALIBRATION_PERCENTILE) as usize])
}
