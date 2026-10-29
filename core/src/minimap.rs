use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::time::{Duration, Instant};

use crate::actors::{self, ActorInspector, ActorLook, Base};
use crate::assets::{Assets, Load};
use crate::badges::{BadgeKey, BakeStep, Badges};
use crate::game::Player;
use crate::host;
use crate::icons::{self, IconClassFinder, IconTable};
use crate::keys::Keys;
use crate::layers::{self, Layers};
use crate::markers::Locations;
use crate::menu::{Menu, MenuContent};
use crate::objects::{MapObject, MapObjects};
use crate::profile;
use crate::settings::{CATEGORIES, LANDMARK_KINDS, Settings, format_keys};
use crate::terrain::Terrain;
use crate::widgets::{self, Widgets};
use crate::ue::{self, Anchors, Function, LinearColor, Name, Obj, SoftObjectPtr, Vec2, Vec3, Weak};
use crate::worldmap::{self, Region};

const ARROW_SIZE: f64 = 26.0;
const IMAGE_ICON_SIZE: f64 = 22.0;
const BADGE_ICON_SIZE: f64 = 24.0;
const CHEVRON_SIZE: Vec2 = Vec2 { x: 12.0, y: 6.0 };
const ELEVATION_THRESHOLD: f64 = 500.0;
const BAKES_PER_CULL: usize = 1;
const GAME_ICON_SCALE: f64 = 0.4;
// The boss widget's 64 px frame has padding around the portrait, so it needs more scale than a badge.
const BOSS_ICON_SCALE: f64 = 0.6;
const MIN_VIEW: f64 = 6_000.0;
const MAX_VIEW: f64 = 160_000.0;
const ZOOM_FACTOR: f64 = 1.25;
const MAX_MEGAZOOM_VIEW: f64 = 400_000.0;
// Auto zoom: how far each step widens the view, and the speeds (m/s) that move up or down a step. Down is
// lower than up so the zoom does not flicker at a boundary, and a step holds for a moment before the next.
const AUTO_ZOOM_STEPS: [f64; 3] = [1.0, 1.5, 2.2];
const AUTO_ZOOM_UP: [f64; 2] = [9.0, 18.0];
const AUTO_ZOOM_DOWN: [f64; 2] = [6.0, 13.0];
const AUTO_ZOOM_DWELL: Duration = Duration::from_millis(1500);
const SPEED_SMOOTHING: f64 = 0.5;
// Seconds for a zoom change to get about two thirds of the way there.
const ZOOM_SMOOTHING: f64 = 0.12;
const ZOOM_SNAP: f64 = 0.002;
// With icons following the zoom, they are their normal size at this view (the default zoom).
const ICON_ZOOM_REFERENCE: f64 = 22_000.0;
const ICON_ZOOM_LIMITS: (f64, f64) = (0.35, 1.8);
// Icons are rescaled only once the wanted size is this far off, so a zoom animation costs a few passes.
const ICON_ZOOM_STEP: f64 = 0.04;
const MAX_SPEED: f64 = 150.0;
const CULL_RADIUS: f64 = 0.75;
const CREATES_PER_CULL: usize = 6;
const DESCRIBES_PER_CULL: usize = 12;
const RETRY_AFTER: Duration = Duration::from_secs(5);
const SLOW_CHECK_EVERY: Duration = Duration::from_secs(1);
const CULL_EVERY: Duration = Duration::from_millis(250);
const SAVE_AFTER: Duration = Duration::from_secs(2);
const BOSS_RETRY: Duration = Duration::from_secs(10);
const FIELD_BOSS: u8 = 16;
const OBJECT_SCAN_EVERY: Duration = Duration::from_secs(3);

const PLAYER_ARROW: &str = "/Game/Pal/Texture/UI/InGame/T_icon_map_player.T_icon_map_player";
const FOG_MATERIAL: &str = "/Game/Pal/Material/UI/Menu/MI_UI_Menu_MapCloud.MI_UI_Menu_MapCloud";

const CLIP_TO_BOUNDS: u8 = 1;
const COLLAPSED: u8 = 1;
const HIT_TEST_INVISIBLE: u8 = 3;

const Z_ARROW: i32 = 10;
// Order inside each marker layer, bottom to top.
const Z_PAL: i32 = 1;
const Z_SHINY_PAL: i32 = 2;
const Z_BOSS_PAL: i32 = 3;
const Z_OTHER: i32 = 4;
const LOCAL_PLAYER_RADIUS: f64 = 400.0;
// Inside a dungeon, markers further above or below the player than this belong to the overworld.
const STAGE_HEIGHT: f64 = 5_000.0;
const Z_LANDMARK_BACKING: i32 = 5;
const Z_LANDMARK: i32 = 6;
// The backing's width against a plain icon's. The game's icon widgets have empty padding around their picture,
// so they get a tighter one.
const BACKING_SIZE: f64 = 1.45;
const GAME_ICON_BACKING_SIZE: f64 = 1.15;
const Z_PIN: i32 = 11;
const PIN_COUNT: usize = 2;
const PIN_RANGE: f64 = 200_000.0;
const PIN_ARROW_SIZE: f64 = 12.0;
const PIN_GAP: f64 = 2.0;
const PIN_ICON_SIZE: f64 = 20.0;
const FAST_TRAVEL: u8 = 3;
const WARPS: [u8; 2] = [24, 25];
// Pinned to the edge: places the player has unlocked or owns, so nothing still under the fog is given away.
const PINNED_KINDS: [u8; 6] = [FAST_TRAVEL, 4, 21, 22, 24, 25];
const HINT_TEXT_SIZE: i32 = 11;
const HINT_COLOR: LinearColor = LinearColor { r: 0.88, g: 0.91, b: 0.95, a: 0.72 };
const HINT_GAP: f64 = 4.0;
const SPECIAL_SCALE: f64 = 1.25;

const BOSS_ICON_CLASS: &str = "/Game/Pal/Blueprint/UI/UserInterface/Map/WBP_Map_IconBoss.WBP_Map_IconBoss_C";
const BOSS_ACTOR_NEAR_SPAWNER: f64 = 6_000.0;

struct BossIcon {
    class: Weak,
    set_boss: Function,
    update_state: Function,
}

impl BossIcon {
    fn resolve(class: Obj) -> Result<Self, String> {
        Ok(Self {
            set_boss: Function::resolve(
                class,
                "SetBoss",
                &[
                    "CharacterID",
                    "Level",
                    "Spawner Id",
                    "OriginalCharacterId",
                    "OriginalSpawnerID",
                    "Location",
                    "MapName",
                ],
            )?,
            update_state: Function::resolve(class, "UpdateBossIconState", &[])?,
            class: Weak::new(class),
        })
    }
}

#[derive(Clone, Copy)]
struct BossSpawner {
    character: Name,
    level: i32,
    spawner: Name,
    at: Vec3,
}

enum IconSetup {
    Setup(Function),
    LocationData(Function),
    Missing,
}

/// How the game's icon widgets show a location's state, when they can: teleporters and bases have a locked
/// and an unlocked look (`SetEnable`), towers redraw themselves (`UpdateIcon`).
struct IconState {
    set_enable: Option<Function>,
    update: Option<Function>,
}

struct Ui {
    blueprint_library: Obj,
    utility: Obj,
    layout_library: Obj,
    user_widget_class: Obj,
    canvas_class: Obj,
    image_class: Obj,
    widget_tree_offset: usize,
    root_widget_offset: usize,
    boss_character_offset: usize,
    boss_location_offset: usize,
    boss_spawner_offset: usize,
    boss_level_offset: usize,
    master_tables: Obj,
    boss_table: Function,
    is_boss_row: Function,
    boss_flags: HashMap<Name, bool>,
    boss_widget: Option<BossIcon>,
    boss_icon_broken: bool,
    world_map_data_class: Obj,
    region_names: Vec<(Name, &'static Region)>,
    icon_setups: HashMap<Weak, IconSetup>,
    icon_states: HashMap<Weak, IconState>,
    portraits: HashMap<Name, Option<String>>,
    database: Option<Weak>,
    create: Function,
    add_to_viewport: Function,
    remove_from_parent: Function,
    set_visibility: Function,
    set_clipping: Function,
    set_angle: Function,
    set_render_scale: Function,
    set_render_opacity: Function,
    desired_size: Function,
    add_child: Function,
    slot_position: Function,
    slot_size: Function,
    slot_alignment: Function,
    slot_anchors: Function,
    slot_auto_size: Function,
    slot_z_order: Function,
    image_texture: Function,
    icon_hidden: Function,
    world_map_data: Function,
    current_render_target: Function,
    current_map_name: Function,
    get_database: Function,
    character_icon: Function,
    viewport_scale: Function,
    fast_travel_enabled: Function,
    warp_point_class: Obj,
    warp_unlocked: Function,
}

impl Ui {
    fn resolve() -> Result<Self, String> {
        let widget = ue::find("/Script/UMG.Widget")?;
        let user_widget_class = ue::find("/Script/UMG.UserWidget")?;
        let canvas_class = ue::find("/Script/UMG.CanvasPanel")?;
        let slot_class = ue::find("/Script/UMG.CanvasPanelSlot")?;
        let image_class = ue::find("/Script/UMG.Image")?;
        let tree_class = ue::find("/Script/UMG.WidgetTree")?;
        let blueprint_library = ue::find("/Script/UMG.Default__WidgetBlueprintLibrary")?;
        let utility = ue::find("/Script/Pal.Default__PalUtility")?;
        let world_map_data_class = ue::find("/Script/Pal.PalWorldMapUIData")?;
        let map_icon_class = ue::find("/Script/Pal.PalUIWorldMapIcon")?;
        let database_class = ue::find("/Script/Pal.PalDatabaseCharacterParameter")?;
        let master_tables = ue::find("/Script/Pal.Default__PalMasterDataTablesUtility")?;
        let boss_data = ue::find("/Script/Pal.PalUIBossSpawnerLoactionData")?;
        let layout_library = ue::find("/Script/UMG.Default__WidgetLayoutLibrary")?;
        let location_point_class = ue::find("/Script/Pal.PalLocationPoint")?;
        let warp_point_class = ue::find("/Script/Pal.PalLocationPointStaticWarpPoint")?;
        Ok(Self {
            fast_travel_enabled: Function::resolve(location_point_class, "IsEnableFastTravel", &[])?,
            warp_unlocked: Function::resolve(warp_point_class, "IsUnlocked", &[])?,
            warp_point_class,
            viewport_scale: Function::resolve(layout_library, "GetViewportScale", &["WorldContextObject"])?,
            layout_library,
            widget_tree_offset: ue::property_offset(user_widget_class, "WidgetTree")?,
            root_widget_offset: ue::property_offset(tree_class, "RootWidget")?,
            boss_character_offset: ue::property_offset(boss_data, "CharacterID")?,
            boss_location_offset: ue::property_offset(boss_data, "Location")?,
            boss_spawner_offset: ue::property_offset(boss_data, "SpawnerID")?,
            boss_level_offset: ue::property_offset(boss_data, "Level")?,
            boss_table: Function::resolve(master_tables, "GetBossSpawnerUIDataTable", &["WorldContextObject"])?,
            is_boss_row: Function::resolve(database_class, "GetIsBoss", &["RowName"])?,
            master_tables,
            boss_flags: HashMap::new(),
            boss_widget: None,
            boss_icon_broken: false,
            region_names: worldmap::REGIONS.iter().map(|region| (Name::new(region.name), *region)).collect(),
            icon_setups: HashMap::new(),
            icon_states: HashMap::new(),
            portraits: HashMap::new(),
            database: None,
            create: Function::resolve(blueprint_library, "Create", &["WorldContextObject", "WidgetType", "OwningPlayer"])?,
            add_to_viewport: Function::resolve(user_widget_class, "AddToViewport", &["ZOrder"])?,
            remove_from_parent: Function::resolve(widget, "RemoveFromParent", &[])?,
            set_visibility: Function::resolve(widget, "SetVisibility", &["InVisibility"])?,
            set_clipping: Function::resolve(widget, "SetClipping", &["InClipping"])?,
            set_angle: Function::resolve(widget, "SetRenderTransformAngle", &["Angle"])?,
            set_render_scale: Function::resolve(widget, "SetRenderScale", &["Scale"])?,
            set_render_opacity: Function::resolve(widget, "SetRenderOpacity", &["InOpacity"])?,
            desired_size: Function::resolve(widget, "GetDesiredSize", &[])?,
            add_child: Function::resolve(canvas_class, "AddChildToCanvas", &["Content"])?,
            slot_position: Function::resolve(slot_class, "SetPosition", &["InPosition"])?,
            slot_size: Function::resolve(slot_class, "SetSize", &["InSize"])?,
            slot_alignment: Function::resolve(slot_class, "SetAlignment", &["InAlignment"])?,
            slot_anchors: Function::resolve(slot_class, "SetAnchors", &["InAnchors"])?,
            slot_auto_size: Function::resolve(slot_class, "SetAutoSize", &["InbAutoSize"])?,
            slot_z_order: Function::resolve(slot_class, "SetZOrder", &["InZOrder"])?,
            image_texture: Function::resolve(image_class, "SetBrushFromTexture", &["Texture", "bMatchSize"])?,
            icon_hidden: Function::resolve(map_icon_class, "IsActiveAnyHideFlag", &[])?,
            world_map_data: Function::resolve(utility, "GetLocalWorldMapData", &["WorldContextObject"])?,
            current_render_target: Function::resolve(world_map_data_class, "GetCurrentRenderTarget", &[])?,
            current_map_name: Function::resolve(world_map_data_class, "GetCurrentPlayMapName", &[])?,
            get_database: Function::resolve(utility, "GetDatabaseCharacterParameter", &["WorldContextObject"])?,
            character_icon: Function::resolve(database_class, "GetCharacterIconTexture", &["CharacterID"])?,
            world_map_data_class,
            blueprint_library,
            utility,
            user_widget_class,
            canvas_class,
            image_class,
        })
    }

    fn attach(&mut self, canvas: Obj, child: Obj, z_order: i32) -> Result<Obj, String> {
        let slot: *mut c_void = self.add_child.call(canvas, |a| a.set_obj(0, Some(child))).returned();
        let slot = Obj::from_raw(slot).ok_or("AddChildToCanvas returned no slot")?;
        self.slot_z_order.call(slot, |a| a.set(0, z_order));
        Ok(slot)
    }

    fn add(&mut self, canvas: Obj, child: Obj, position: Vec2, size: Vec2, z_order: i32) -> Result<Obj, String> {
        let slot = self.attach(canvas, child, z_order)?;
        self.slot_position.call(slot, |a| a.set(0, position));
        self.slot_size.call(slot, |a| a.set(0, size));
        Ok(slot)
    }

    fn image(&mut self, tree: Obj, canvas: Obj, position: Vec2, size: Vec2, z_order: i32) -> Result<(Obj, Obj), String> {
        let image = ue::construct(self.image_class, tree)?;
        let slot = self.add(canvas, image, position, size, z_order)?;
        Ok((image, slot))
    }

    fn set_texture(&mut self, image: Obj, texture: Obj) {
        self.image_texture.call(image, |a| {
            a.set_obj(0, Some(texture));
            a.set(1, false);
        });
    }

    fn set_shown(&mut self, widget: Obj, shown: bool) {
        self.set_visibility
            .call(widget, |a| a.set(0, if shown { HIT_TEST_INVISIBLE } else { COLLAPSED }));
    }

    fn center(&mut self, slot: Obj) {
        self.slot_alignment.call(slot, |a| a.set(0, Vec2 { x: 0.5, y: 0.5 }));
    }

    fn user_widget(&mut self, class: Obj, world: Obj, controller: Obj) -> Result<Obj, String> {
        let widget: *mut c_void = self
            .create
            .call(self.blueprint_library, |a| {
                a.set_obj(0, Some(world));
                a.set_obj(1, Some(class));
                a.set_obj(2, Some(controller));
            })
            .returned();
        Obj::from_raw(widget).ok_or_else(|| format!("could not create a {}", class.name()))
    }

    fn world_map_data(&mut self, world: Obj) -> Option<Obj> {
        let data: *mut c_void = self
            .world_map_data
            .call(self.utility, |a| a.set_obj(0, Some(world)))
            .returned();
        Obj::from_raw(data)
    }

    fn fog_texture(&mut self, data: Obj) -> Option<Obj> {
        let texture: *mut c_void = self.current_render_target.call(data, |_| {}).returned();
        Obj::from_raw(texture)
    }

    fn map_name(&mut self, data: Obj) -> Name {
        self.current_map_name.call(data, |_| {}).returned()
    }

    fn region_named(&self, name: Name) -> Option<&'static Region> {
        self.region_names.iter().find(|(known, _)| *known == name).map(|(_, region)| *region)
    }

    fn current_region(&mut self, world: Obj) -> &'static Region {
        self.world_map_data(world)
            .map(|data| self.map_name(data))
            .and_then(|name| self.region_named(name))
            .unwrap_or(&worldmap::MAIN)
    }

    fn setup_icon(&mut self, class: Obj, widget: Obj, location: Obj, point: Obj, id: ue::Guid) {
        let setup = self.icon_setups.entry(Weak::new(class)).or_insert_with(|| {
            if let Ok(function) = Function::resolve(class, "Setup", &["LocationPoint", "LocationId"]) {
                IconSetup::Setup(function)
            } else if let Ok(function) = Function::resolve(class, "SetLocationData", &["InLocation"]) {
                IconSetup::LocationData(function)
            } else {
                host::warning(&format!("{} has no setup function", class.name()));
                IconSetup::Missing
            }
        });
        match setup {
            IconSetup::Setup(function) => {
                function.call(widget, |a| {
                    a.set_obj(0, Some(point));
                    a.set(1, id);
                });
            }
            IconSetup::LocationData(function) => {
                function.call(widget, |a| a.set_obj(0, Some(location)));
            }
            IconSetup::Missing => {}
        }
    }

    /// Brings a game icon widget's look in line with its location: locked or unlocked, by the game's own check.
    fn refresh_icon(&mut self, class: Obj, widget: Obj, point: Obj) {
        let enabled = self.fast_travel_enabled.call(point, |_| {}).returned::<u8>() != 0;
        let state = self.icon_states.entry(Weak::new(class)).or_insert_with(|| IconState {
            set_enable: Function::resolve(class, "SetEnable", &["Enable"]).ok(),
            update: Function::resolve(class, "UpdateIcon", &[]).ok(),
        });
        if let Some(update) = state.update.as_mut() {
            update.call(widget, |_| {});
        }
        if let Some(set_enable) = state.set_enable.as_mut() {
            set_enable.call(widget, |a| a.set(0, enabled));
        }
    }

    fn is_icon_hidden(&mut self, widget: Obj) -> bool {
        self.icon_hidden.call(widget, |_| {}).returned::<u8>() != 0
    }

    fn database(&mut self, world: Obj) -> Option<Obj> {
        if let Some(database) = self.database.and_then(Weak::get) {
            return Some(database);
        }
        let database: *mut c_void = self
            .get_database
            .call(self.utility, |a| a.set_obj(0, Some(world)))
            .returned();
        let database = Obj::from_raw(database)?;
        self.database = Some(Weak::new(database));
        Some(database)
    }

    /// Whether the game's character table marks this character as a boss.
    fn is_boss(&mut self, world: Obj, character: Name) -> bool {
        if let Some(&known) = self.boss_flags.get(&character) {
            return known;
        }
        let Some(database) = self.database(world) else { return false };
        let boss = self.is_boss_row.call(database, |a| a.set(0, character)).returned::<u8>() != 0;
        self.boss_flags.insert(character, boss);
        boss
    }

    /// The full map's own boss icon widget, loaded on first use.
    fn boss_icon(&mut self, assets: &mut Assets, world: Obj) -> Load<&mut BossIcon> {
        if self.boss_icon_broken {
            return Load::Failed;
        }
        let class = match assets.get(world, BOSS_ICON_CLASS) {
            Load::Ready(class) => class,
            Load::Waiting => return Load::Waiting,
            Load::Failed => return Load::Failed,
        };
        if self.boss_widget.as_ref().is_none_or(|icon| icon.class.get() != Some(class)) {
            match BossIcon::resolve(class) {
                Ok(icon) => self.boss_widget = Some(icon),
                Err(reason) => {
                    host::warning(&format!("boss icons fall back to badges: {reason}"));
                    self.boss_icon_broken = true;
                    return Load::Failed;
                }
            }
        }
        self.boss_widget.as_mut().map_or(Load::Failed, Load::Ready)
    }

    fn region_name_at(&self, x: f64, y: f64) -> Name {
        self.region_names
            .iter()
            .find(|(_, region)| region.contains(x, y))
            .or(self.region_names.first())
            .map(|(name, _)| *name)
            .unwrap_or(Name::new(worldmap::MAIN.name))
    }

    /// Path of the character's portrait texture.
    fn portrait(&mut self, world: Obj, character: Name) -> Option<String> {
        if let Some(known) = self.portraits.get(&character) {
            return known.clone();
        }
        let database = self.database(world)?;
        let lookup = |id: Name, ui: &mut Self| -> Option<String> {
            let soft: SoftObjectPtr = ui.character_icon.call(database, |a| a.set(0, id)).returned();
            let path = unsafe { ue::soft_path(soft.as_ptr()) };
            (!path.is_empty() && path != "None").then_some(path)
        };
        let text = character.text();
        let found = lookup(character, self).or_else(|| {
            let base = text.strip_prefix("BOSS_").or_else(|| text.strip_prefix("Boss_"))?;
            lookup(Name::new(base), self)
        });
        if found.is_none() {
            host::warning(&format!("no portrait for {text}"));
        }
        self.portraits.insert(character, found.clone());
        found
    }

    /// Whether the player can use this fast travel point or warp, by the game's own check. Other kinds pass.
    fn unlocked(&mut self, kind: u8, point: Obj) -> bool {
        match kind {
            FAST_TRAVEL => self.fast_travel_enabled.call(point, |_| {}).returned::<u8>() != 0,
            _ if WARPS.contains(&kind) => {
                ue::is_a(point, self.warp_point_class) && self.warp_unlocked.call(point, |_| {}).returned::<u8>() != 0
            }
            _ => true,
        }
    }

    /// UI units per screen pixel.
    fn pixel_size(&mut self, world: Obj) -> f64 {
        let scale: f32 = self
            .viewport_scale
            .call(self.layout_library, |a| a.set_obj(0, Some(world)))
            .returned();
        if scale > 0.1 { 1.0 / f64::from(scale) } else { 1.0 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum MarkerKey {
    Location(Weak),
    Boss(Name),
    Actor(Weak),
    Model(Weak),
}

enum Look {
    GameIcon(Weak),
    Texture(String),
    Character(BadgeKey),
    Boss(BossSpawner),
    Undecided,
    Nothing,
}

struct PendingTexture {
    key: BadgeKey,
    path: String,
}

struct Visual {
    widget: Weak,
    slot: Weak,
    game_icon: bool,
    half_height: f64,
    /// Render scale before the zoom's share: the game's icon widgets are drawn shrunk, images at 1.
    base_scale: f64,
    landmark: bool,
    /// The dark disc behind a landmark: its image and slot.
    backing: Option<(Weak, Weak)>,
    pending: Option<PendingTexture>,
}

impl Visual {
    /// How much bigger than its setup this icon is drawn now. Landmarks never shrink with the zoom.
    fn factor(&self, look: IconLook) -> f64 {
        if self.landmark { look.zoom.max(1.0) * look.landmark } else { look.zoom }
    }
}

struct Elevation {
    widget: Weak,
    slot: Weak,
    up: Option<bool>,
    placed: Option<(Vec2, IconLook)>,
}

/// How every icon is drawn on top of its own setup: the zoom's share of its scale, and the turn that undoes
/// the map's so it stays upright.
#[derive(Clone, Copy, PartialEq, Debug)]
struct IconLook {
    zoom: f64,
    angle: f32,
    landmark: f64,
}

/// Places to find again, from the game's own location list: not pals, items or other world actors.
fn is_landmark(view: &MarkerView) -> bool {
    matches!(view.key, MarkerKey::Location(_) | MarkerKey::Boss(_)) && LANDMARK_KINDS.contains(&view.kind)
}

struct MarkerView {
    key: MarkerKey,
    kind: u8,
    look: Look,
    actor_base: Option<Base>,
    above_fog: bool,
    moves: bool,
    world: Option<(f64, f64)>,
    height: f64,
    present: bool,
    pool_revision: Option<u32>,
    /// Static item id, for item pickups, which are switched on and off per item.
    pickup: Option<String>,
    visual: Option<Visual>,
    elevation: Option<Elevation>,
    failed: bool,
    shown: bool,
    placed: Option<Vec2>,
}

impl MarkerView {
    fn new(key: MarkerKey, kind: u8, look: Look, above_fog: bool, moves: bool, at: Option<Vec3>) -> Self {
        let mut view = Self {
            key,
            kind,
            look,
            actor_base: None,
            above_fog,
            moves,
            world: None,
            height: f64::NAN,
            present: true,
            pool_revision: None,
            pickup: None,
            visual: None,
            elevation: None,
            failed: false,
            shown: false,
            placed: None,
        };
        view.set_position(at);
        view
    }

    fn set_position(&mut self, at: Option<Vec3>) {
        self.world = at.map(|at| (at.x, at.y));
        self.height = at.map_or(f64::NAN, |at| at.z);
    }

    fn drawable(&self) -> bool {
        !self.failed && !matches!(self.look, Look::Nothing)
    }

    fn drop_visual(&mut self, ui: &mut Ui) {
        if let Some(visual) = self.visual.take() {
            let backing = visual.backing.and_then(|(image, _)| image.get());
            for widget in [visual.widget.get(), backing].into_iter().flatten() {
                ui.remove_from_parent.call(widget, |_| {});
            }
        }
        if let Some(widget) = self.elevation.take().and_then(|elevation| elevation.widget.get()) {
            ui.remove_from_parent.call(widget, |_| {});
        }
        self.shown = false;
        self.placed = None;
    }
}

struct Live {
    widget: Weak,
    tree: Weak,
    frame_slot: Weak,
    arrow_slot: Weak,
    hint: Weak,
    hint_slot: Weak,
    layers: Layers,
    arrow: Weak,
    /// Degrees the map is turned by, and the matching turn applied to every icon so it stays upright.
    map_angle: f32,
    icon_angle: f32,
    /// The zoom's share of every icon's scale, and the landmark size, as last applied.
    icon_zoom: f64,
    landmark_scale: f64,
    backing_texture: Option<Weak>,
    pins: Vec<EdgePin>,
    cone: Option<(bool, f32)>,
    region: &'static Region,
    unknown_map: Option<Name>,
    size: f64,
    pixel: f64,
    layer_size: Vec2,
    layer_position: Option<Vec2>,
    arrow_angle: Option<f32>,
    shown: bool,
    needs_layout: bool,
    needs_cull: bool,
    bosses_loaded: bool,
    next_boss_try: Instant,
    boss_spawners: HashMap<Name, Vec<(f64, f64)>>,
    markers: HashMap<MarkerKey, MarkerView>,
    moving_shown: Vec<MarkerKey>,
    sized_classes: HashSet<Weak>,
    view_units: f64,
    model_kinds: HashSet<u8>,
    next_object_scan: Instant,
    size_reports: Vec<(Weak, Weak)>,
    next_slow: Instant,
    next_cull: Instant,
}

impl Live {
    fn layer_point(&self, x: f64, y: f64) -> Vec2 {
        let (u, v) = self.region.to_uv(x, y);
        Vec2 {
            x: u * self.layer_size.x,
            y: v * self.layer_size.y,
        }
    }

    /// Where a world point is on the map layer, as a fraction of its size: what markers are anchored to.
    fn layer_uv(&self, x: f64, y: f64) -> Vec2 {
        let (u, v) = self.region.to_uv(x, y);
        Vec2 { x: u, y: v }
    }

    fn icon_look(&self) -> IconLook {
        IconLook { zoom: self.icon_zoom, angle: self.icon_angle, landmark: self.landmark_scale }
    }

    /// A tolerance in layer units, as a fraction of the layer.
    fn uv_tolerance(&self, units: f64) -> f64 {
        units / self.layer_size.x.max(1.0)
    }
}

/// Puts a marker's icon and chevron at the render scale its base and the zoom call for.
fn scale_marker(ui: &mut Ui, view: &mut MarkerView, look: IconLook) {
    let Some(visual) = view.visual.as_ref() else { return };
    let zoom = visual.factor(look);
    if let Some(widget) = visual.widget.get() {
        let scale = visual.base_scale * zoom;
        ui.set_render_scale.call(widget, |a| a.set(0, Vec2 { x: scale, y: scale }));
    }
    if let Some(backing) = visual.backing.and_then(|(image, _)| image.get()) {
        ui.set_render_scale.call(backing, |a| a.set(0, Vec2 { x: zoom, y: zoom }));
    }
    if let Some(elevation) = view.elevation.as_mut() {
        if let Some(widget) = elevation.widget.get() {
            ui.set_render_scale.call(widget, |a| a.set(0, Vec2 { x: zoom, y: zoom }));
        }
    }
    place_elevation(ui, view, look);
}

/// One of the nearest landmarks off the minimap, shown at its edge with an arrow pointing to it.
struct EdgePin {
    icon: Weak,
    icon_slot: Weak,
    arrow: Weak,
    arrow_slot: Weak,
    target: Option<(MarkerKey, (f64, f64))>,
    placed: Option<(Vec2, f32)>,
    shown: bool,
}

impl EdgePin {
    fn build(ui: &mut Ui, tree: Obj, frame: Obj, arrow_texture: Option<Obj>) -> Result<Self, String> {
        let (icon, icon_slot) = ui.image(tree, frame, Vec2::default(), Vec2 { x: PIN_ICON_SIZE, y: PIN_ICON_SIZE }, Z_PIN)?;
        ui.center(icon_slot);
        ui.set_shown(icon, false);
        let arrow_extent = Vec2 { x: PIN_ARROW_SIZE, y: PIN_ARROW_SIZE };
        let (arrow, arrow_slot) = ui.image(tree, frame, Vec2::default(), arrow_extent, Z_PIN)?;
        ui.center(arrow_slot);
        if let Some(texture) = arrow_texture {
            ui.set_texture(arrow, texture);
        }
        ui.set_shown(arrow, false);
        Ok(Self {
            icon: Weak::new(icon),
            icon_slot: Weak::new(icon_slot),
            arrow: Weak::new(arrow),
            arrow_slot: Weak::new(arrow_slot),
            target: None,
            placed: None,
            shown: false,
        })
    }

    fn show(&mut self, ui: &mut Ui, shown: bool) {
        if shown == self.shown {
            return;
        }
        self.shown = shown;
        for widget in [self.icon.get(), self.arrow.get()].into_iter().flatten() {
            ui.set_shown(widget, shown);
        }
    }
}

/// Picks the nearest unlocked fast travel points, warps, bases and quests that are off the minimap. Run with
/// the cull, four times a second.
fn choose_pins(ui: &mut Ui, live: &mut Live, locations: Option<&mut Locations>, icons: Option<&IconTable>, assets: &mut Assets, context: &CullContext) {
    let settings = context.settings;
    let (Some(locations), true, false) = (locations, settings.edge_pins, context.player.in_stage) else {
        for pin in &mut live.pins {
            pin.target = None;
            pin.show(ui, false);
        }
        return;
    };
    let (px, py) = (context.player.location.x, context.player.location.y);
    let here = live.region.contains(px, py);
    let size = live.layers.size();
    let mut candidates: Vec<(f64, MarkerKey, u8, (f64, f64))> = live
        .markers
        .values()
        .filter(|view| matches!(view.key, MarkerKey::Location(_)) && PINNED_KINDS.contains(&view.kind))
        .filter(|view| view.present && settings.shows_kind(view.kind))
        .filter_map(|view| {
            let (x, y) = view.world?;
            let distance = (x - px).hypot(y - py);
            if live.region.contains(x, y) != here || distance > PIN_RANGE {
                return None;
            }
            let at = live.layers.frame_point(live.layer_point(x, y));
            let on_minimap = (0.0..=size).contains(&at.x) && (0.0..=size).contains(&at.y) && live.layers.inside(at);
            (!on_minimap).then_some((distance, view.key, view.kind, (x, y)))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut chosen = Vec::with_capacity(PIN_COUNT);
    for (_, key, kind, at) in candidates {
        if chosen.len() == PIN_COUNT {
            break;
        }
        let MarkerKey::Location(location) = key else { continue };
        let Some(location) = location.get() else { continue };
        let Some(point) = locations.point(location) else { continue };
        if !ui.unlocked(kind, point) {
            continue;
        }
        let Some(path) = locations.describe(location, kind, icons).icon else { continue };
        let Load::Ready(texture) = assets.get(context.player.world, &path) else { continue };
        chosen.push((key, at, texture));
    }

    let extent = PIN_ICON_SIZE * settings.icon_scale * settings.landmark_scale;
    for (index, pin) in live.pins.iter_mut().enumerate() {
        let Some(&(key, at, texture)) = chosen.get(index) else {
            pin.target = None;
            pin.show(ui, false);
            continue;
        };
        if pin.target.map(|(known, _)| known) != Some(key) {
            if let (Some(icon), Some(slot)) = (pin.icon.get(), pin.icon_slot.get()) {
                ui.set_texture(icon, texture);
                ui.slot_size.call(slot, |a| a.set(0, Vec2 { x: extent, y: extent }));
            }
            pin.placed = None;
        }
        pin.target = Some((key, at));
    }
}

/// Keeps each pin on the edge, in line with its landmark. Runs every frame, but only calls the engine when a
/// pin has moved.
fn place_pins(ui: &mut Ui, live: &mut Live, settings: &Settings) {
    let half = live.layers.size() / 2.0;
    let icon_half = PIN_ICON_SIZE * settings.icon_scale * settings.landmark_scale / 2.0;
    for index in 0..live.pins.len() {
        let Some((_, (x, y))) = live.pins[index].target else { continue };
        let at = live.layers.frame_point(live.layer_point(x, y));
        let dir = Vec2 { x: at.x - half, y: at.y - half };
        let arrow_at = live.layers.edge_point(dir, PIN_ARROW_SIZE / 2.0);
        let icon_at = live.layers.edge_point(dir, PIN_ARROW_SIZE + PIN_GAP + icon_half);
        let angle = dir.x.atan2(-dir.y).to_degrees() as f32;
        let pin = &mut live.pins[index];
        let moved = pin.placed.is_none_or(|(placed, placed_angle)| {
            (placed.x - icon_at.x).abs() > 0.3 || (placed.y - icon_at.y).abs() > 0.3 || angle_moved(placed_angle, angle, 1.0)
        });
        if moved {
            if let (Some(icon_slot), Some(arrow_slot), Some(arrow)) = (pin.icon_slot.get(), pin.arrow_slot.get(), pin.arrow.get()) {
                ui.slot_position.call(icon_slot, |a| a.set(0, icon_at));
                ui.slot_position.call(arrow_slot, |a| a.set(0, arrow_at));
                ui.set_angle.call(arrow, |a| a.set(0, angle));
            }
            pin.placed = Some((icon_at, angle));
        }
        pin.show(ui, true);
    }
}

fn refresh_region(ui: &mut Ui, w: &mut Widgets, live: &mut Live, data: Obj, assets: &mut Assets, world: Obj) {
    let name = ui.map_name(data);
    let Some(region) = ui.region_named(name) else {
        if live.unknown_map != Some(name) {
            live.unknown_map = Some(name);
            host::warning(&format!("no minimap region for map {}", name.text()));
        }
        return;
    };
    if region.name == live.region.name {
        return;
    }
    let Load::Ready(texture) = assets.get(world, region.texture) else { return };
    keep_full_resolution(texture);
    live.layers.set_map_texture(w, texture);
    live.region = region;
    live.needs_layout = true;
    host::info(&format!("map region {}", region.name));
}

/// Has the engine keep a map texture's full resolution loaded. It streams, and a UI image never asks for its
/// sharp levels, so without this the minimap shows a soft low-resolution copy of what the game's full map shows.
fn keep_full_resolution(texture: Obj) {
    let flags = ["bForceMiplevelsToBeResident", "bGlobalForceMipLevelsToBeResident", "bIgnoreStreamingMipBias"];
    let missing: Vec<&str> = flags.into_iter().filter(|flag| !ue::set_bool_property(texture, flag, true)).collect();
    if missing.is_empty() {
        host::info(&format!("{} kept at full resolution", texture.name()));
    } else {
        host::warning(&format!("{} may stay soft: no {}", texture.name(), missing.join(", ")));
    }
}

fn refresh_fog(ui: &mut Ui, w: &mut Widgets, live: &mut Live, data: Obj) {
    if !live.layers.has_fog() {
        return;
    }
    let Some(texture) = ui.fog_texture(data) else { return };
    if live.layers.set_fog_texture(w, texture) {
        host::info(&format!("fog of war attached ({})", texture.name()));
    }
}

/// Field boss spawners, from the boss spawner UI table (the full map's own source), falling back
/// to the world map data's copy of it.
fn load_bosses(ui: &mut Ui, live: &mut Live, world: Obj, data: Obj) {
    live.next_boss_try = Instant::now() + BOSS_RETRY;
    let read_row = |ui: &Ui, row: *mut c_void| unsafe {
        BossSpawner {
            spawner: ue::read_at(row, ui.boss_spawner_offset),
            character: ue::read_at(row, ui.boss_character_offset),
            level: ue::read_at(row, ui.boss_level_offset),
            at: ue::read_at(row, ui.boss_location_offset),
        }
    };

    let mut spawners = Vec::new();
    let mut source = "boss spawner table";
    let table: *mut c_void = ui
        .boss_table
        .call(ui.master_tables, |a| a.set_obj(0, Some(world)))
        .returned();
    let mut rows = Vec::new();
    if let Some(table) = Obj::from_raw(table) {
        if ue::data_table_rows(table, &mut rows).is_ok() {
            spawners.extend(rows.iter().map(|&row| read_row(ui, row)));
        }
    }
    if spawners.is_empty() {
        source = "world map data";
        let mut pairs = Vec::new();
        if let Ok(value_offset) = ue::map_entries(ui.world_map_data_class, "AllBossSpawnerUIDataMap", data.raw(), &mut pairs) {
            spawners.extend(
                pairs
                    .iter()
                    .map(|&pair| read_row(ui, unsafe { pair.cast::<u8>().add(value_offset).cast() })),
            );
        }
    }
    if spawners.is_empty() {
        return;
    }

    live.bosses_loaded = true;
    for boss in &spawners {
        live.boss_spawners.entry(boss.character).or_default().push((boss.at.x, boss.at.y));
        let key = MarkerKey::Boss(boss.spawner);
        live.markers.insert(key, MarkerView::new(key, FIELD_BOSS, Look::Boss(*boss), false, false, Some(boss.at)));
    }
    live.needs_cull = true;
    host::info(&format!("field bosses: {} from the {source}", spawners.len()));
}

/// Resizes the map for a new zoom. Markers are anchored to the map, so none of them is touched. Which ones are
/// in range is checked again once the zoom has settled, and by the regular cull meanwhile.
fn apply_layout(w: &mut Widgets, live: &mut Live, view_units: f64, settled: bool) {
    live.needs_layout = false;
    let size = live.region.image_size(live.size, view_units);
    live.layer_size = size;
    live.layers.set_layer_size(w, size);
    live.layer_position = None;
    if settled {
        live.needs_cull = true;
    }
}

struct Sources<'a> {
    locations: Option<&'a mut Locations>,
    actors: Option<&'a mut ActorInspector>,
    icons: Option<&'a IconTable>,
    classes: Option<&'a IconClassFinder>,
    badges: Option<&'a mut Badges>,
    objects: Option<&'a MapObjects>,
}

/// Markers for the map objects the host knows about anywhere in the world. Returns true when it found an
/// item pickup type the settings did not have yet.
#[allow(clippy::too_many_arguments)]
fn scan_map_objects(
    ui: &mut Ui,
    live: &mut Live,
    objects: &mut MapObjects,
    icons: &IconTable,
    settings: &mut Settings,
    world: Obj,
    list: &mut Vec<MapObject>,
) -> bool {
    let total = match objects.scan(world, icons, list) {
        Ok(total) => total,
        Err(reason) => {
            host::warning(&format!("map objects unavailable: {reason}"));
            return false;
        }
    };
    let mut seen = HashSet::with_capacity(list.len());
    let mut added = 0;
    let mut learned = false;
    for object in list.iter() {
        let key = MarkerKey::Model(Weak::new(object.model));
        seen.insert(key);
        if live.markers.contains_key(&key) {
            continue;
        }
        if let Some(item) = &object.item {
            learned |= settings.learn_pickup(item);
        }
        let look = Look::Texture(object.icon.clone());
        let mut view = MarkerView::new(key, object.kind, look, false, false, Some(object.at));
        view.pickup = object.item.clone();
        live.markers.insert(key, view);
        added += 1;
    }
    let before = live.markers.len();
    live.markers.retain(|key, view| {
        let keep = !matches!(key, MarkerKey::Model(_)) || seen.contains(key);
        if !keep {
            view.drop_visual(ui);
        }
        keep
    });
    let removed = before - live.markers.len();
    if added > 0 || removed > 0 {
        live.needs_cull = true;
        host::info(&format!(
            "map objects: {} with markers out of {total} models, {added} new, {removed} gone",
            list.len()
        ));
    }
    live.model_kinds = list.iter().map(|object| object.kind).collect();
    learned
}

fn location_look(kind: u8, location: Obj, locations: &mut Locations, sources_icons: Option<&IconTable>, classes: Option<&IconClassFinder>) -> Look {
    if Locations::is_player(kind) {
        return Look::Nothing;
    }
    if let Some(class) = classes.and_then(|classes| classes.for_kind(kind)) {
        return Look::GameIcon(Weak::new(class));
    }
    match locations.describe(location, kind, sources_icons).icon {
        Some(path) => Look::Texture(path),
        None => Look::Nothing,
    }
}

fn scan_locations(ui: &mut Ui, live: &mut Live, sources: &mut Sources, world: Obj, found: &mut Vec<Obj>) {
    let Some(locations) = sources.locations.as_deref_mut() else { return };
    if let Err(reason) = locations.current(world, found) {
        host::warning(&format!("markers unavailable: {reason}"));
        return;
    }

    let mut seen = HashSet::with_capacity(found.len());
    let mut added: Vec<(u8, &'static str)> = Vec::new();
    for &location in found.iter() {
        let key = MarkerKey::Location(Weak::new(location));
        seen.insert(key);
        let present = locations.shown_on_map(location);
        if let Some(view) = live.markers.get_mut(&key) {
            if view.present != present {
                view.present = present;
                live.needs_cull = true;
            }
            continue;
        }

        let kind = locations.kind(location);
        let info = locations.describe(location, kind, None);
        let look = location_look(kind, location, locations, sources.icons, sources.classes);
        added.push((
            kind,
            match look {
                Look::GameIcon(_) => "game icon",
                Look::Texture(_) => "image",
                _ => "hidden",
            },
        ));
        let mut view = MarkerView::new(key, kind, look, info.above_fog, info.moves, locations.position(location));
        view.present = present;
        live.markers.insert(key, view);
        live.needs_cull = true;
    }
    if !added.is_empty() {
        log_added(&added);
    }

    live.markers.retain(|key, view| {
        let keep = !matches!(key, MarkerKey::Location(_)) || seen.contains(key);
        if !keep {
            view.drop_visual(ui);
        }
        keep
    });
}

fn upgrade_location_looks(ui: &mut Ui, live: &mut Live, classes: &IconClassFinder) {
    for view in live.markers.values_mut() {
        if !matches!(view.key, MarkerKey::Location(_)) || matches!(view.look, Look::GameIcon(_)) {
            continue;
        }
        if let Some(class) = classes.for_kind(view.kind) {
            view.drop_visual(ui);
            view.look = Look::GameIcon(Weak::new(class));
            view.failed = false;
        }
    }
    live.needs_cull = true;
}

fn log_added(added: &[(u8, &str)]) {
    let mut counts: Vec<((u8, &str), usize)> = Vec::new();
    for &entry in added {
        match counts.iter_mut().find(|(known, _)| *known == entry) {
            Some((_, count)) => *count += 1,
            None => counts.push((entry, 1)),
        }
    }
    counts.sort_unstable();
    let summary: Vec<String> = counts
        .iter()
        .map(|((kind, how), count)| format!("type {kind} x{count} ({how})"))
        .collect();
    host::info(&format!("markers added: {}", summary.join(", ")));
}

struct CullContext<'a> {
    player: &'a Player,
    settings: &'a Settings,
    view_units: f64,
}

#[derive(Default)]
struct Budget {
    creates: usize,
    describes: usize,
    bakes: usize,
}

/// `Ok(None)` while an asset the marker needs is still loading.
fn create_visual(
    ui: &mut Ui,
    live: &mut Live,
    view: &MarkerView,
    sources: &mut Sources,
    assets: &mut Assets,
    context: &CullContext,
) -> Result<Option<Visual>, String> {
    let (over_fog, z_order) = marker_place(view, context.settings);
    let (Some(tree), Some(layer)) = (live.tree.get(), live.layers.canvas(over_fog)) else {
        return Err("minimap is gone".into());
    };
    let size = IMAGE_ICON_SIZE * context.settings.icon_scale;
    let extent = Vec2 { x: size, y: size };

    match &view.look {
        Look::GameIcon(class) => {
            let class = class.get().ok_or("icon class is gone")?;
            let MarkerKey::Location(location) = view.key else { return Err("game icon without a location".into()) };
            let location = location.get().ok_or("location is gone")?;
            let locations = sources.locations.as_deref_mut().ok_or("no location reader")?;
            let point = locations.point(location).ok_or("marker has no location point")?;
            let id = locations.id(location);
            let widget = ui.user_widget(class, context.player.world, context.player.controller)?;
            let slot = ui.attach(layer, widget, z_order)?;
            ui.slot_auto_size.call(slot, |a| a.set(0, true));
            ui.center(slot);
            let scale = GAME_ICON_SCALE * context.settings.icon_scale;
            ui.set_render_scale.call(widget, |a| a.set(0, Vec2 { x: scale, y: scale }));
            ui.setup_icon(class, widget, location, point, id);
            ui.refresh_icon(class, widget, point);
            ui.set_shown(widget, false);
            let class_id = Weak::new(class);
            if live.sized_classes.insert(class_id) {
                live.size_reports.push((Weak::new(widget), class_id));
            }
            Ok(Some(Visual {
                widget: Weak::new(widget),
                slot: Weak::new(slot),
                game_icon: true,
                half_height: size / 2.0,
                base_scale: scale,
                landmark: false,
                backing: None,
                pending: None,
            }))
        }
        Look::Texture(path) => {
            let texture = match assets.get(context.player.world, path) {
                Load::Ready(texture) => texture,
                Load::Waiting => return Ok(None),
                Load::Failed => return Err(format!("{path} unavailable")),
            };
            let (image, slot) = ui.image(tree, layer, Vec2::default(), extent, z_order)?;
            ui.center(slot);
            ui.set_texture(image, texture);
            ui.set_shown(image, false);
            Ok(Some(Visual {
                widget: Weak::new(image),
                slot: Weak::new(slot),
                game_icon: false,
                half_height: size / 2.0,
                base_scale: 1.0,
                landmark: false,
                backing: None,
                pending: None,
            }))
        }
        Look::Character(key) => badge_visual(ui, tree, layer, z_order, *key, sources, context).map(Some),
        Look::Boss(boss) => {
            let boss = *boss;
            let world = context.player.world;
            let map_name = ui.region_name_at(boss.at.x, boss.at.y);
            let class = match ui.boss_icon(assets, world) {
                Load::Ready(icon) => icon.class.get(),
                Load::Waiting => return Ok(None),
                Load::Failed => None,
            };
            let Some(class) = class else {
                let key = BadgeKey { character: boss.character, boss: true, shiny: false };
                return badge_visual(ui, tree, layer, z_order, key, sources, context).map(Some);
            };
            let widget = ui.user_widget(class, world, context.player.controller)?;
            let slot = ui.attach(layer, widget, z_order)?;
            ui.slot_auto_size.call(slot, |a| a.set(0, true));
            ui.center(slot);
            let scale = BOSS_ICON_SCALE * context.settings.icon_scale;
            ui.set_render_scale.call(widget, |a| a.set(0, Vec2 { x: scale, y: scale }));
            if let Load::Ready(icon) = ui.boss_icon(assets, world) {
                icon.set_boss.call(widget, |a| {
                    a.set(0, boss.character);
                    a.set(1, boss.level);
                    a.set(2, boss.spawner);
                    a.set(3, boss.character);
                    a.set(4, boss.spawner);
                    a.set(5, boss.at);
                    a.set(6, map_name);
                });
            }
            ui.set_shown(widget, false);
            let class_id = Weak::new(class);
            if live.sized_classes.insert(class_id) {
                live.size_reports.push((Weak::new(widget), class_id));
            }
            Ok(Some(Visual {
                widget: Weak::new(widget),
                slot: Weak::new(slot),
                game_icon: true,
                half_height: size / 2.0,
                base_scale: scale,
                landmark: false,
                backing: None,
                pending: None,
            }))
        }
        Look::Undecided | Look::Nothing => Err("nothing to draw".into()),
    }
}

/// A round portrait badge: from the cache when it exists, otherwise pending until it is made.
fn badge_visual(
    ui: &mut Ui,
    tree: Obj,
    layer: Obj,
    z_order: i32,
    key: BadgeKey,
    sources: &mut Sources,
    context: &CullContext,
) -> Result<Visual, String> {
    let world = context.player.world;
    let scale = if key.shiny || key.boss { SPECIAL_SCALE } else { 1.0 };
    let badge_size = BADGE_ICON_SIZE * context.settings.icon_scale * scale;
    let (image, slot) = ui.image(tree, layer, Vec2::default(), Vec2 { x: badge_size, y: badge_size }, z_order)?;
    ui.center(slot);
    ui.set_shown(image, false);
    let visual = |pending| Visual {
        widget: Weak::new(image),
        slot: Weak::new(slot),
        game_icon: false,
        half_height: badge_size / 2.0,
        base_scale: 1.0,
        landmark: false,
        backing: None,
        pending,
    };
    if let Some(badge) = sources.badges.as_deref_mut().and_then(|badges| badges.cached(world, key)) {
        ui.set_texture(image, badge);
        return Ok(visual(None));
    }
    let path = ui
        .portrait(world, key.character)
        .ok_or_else(|| format!("no portrait for {}", key.character.text()))?;
    Ok(visual(Some(PendingTexture { key, path })))
}

fn describe_actor(ui: &mut Ui, view: &mut MarkerView, actors: &mut ActorInspector, icons: Option<&IconTable>, player: &Player) {
    let (MarkerKey::Actor(actor), Some(base)) = (view.key, view.actor_base) else { return };
    let Some(actor) = actor.get() else {
        view.look = Look::Nothing;
        return;
    };
    let described = actors.describe(actor, base, icons);
    view.kind = described.kind;
    view.above_fog = described.kind == actors::PLAYER;
    view.look = match described.look {
        ActorLook::Texture(path) => Look::Texture(path),
        ActorLook::Character { id, shiny } => {
            let boss = ui.is_boss(player.world, id);
            // Captured alphas keep their boss row but belong under OwnPals.
            if described.kind == actors::WILD_PAL {
                if boss {
                    view.kind = FIELD_BOSS;
                } else if shiny {
                    view.kind = actors::SHINY_PAL;
                }
            }
            Look::Character(BadgeKey { character: id, boss, shiny })
        }
        ActorLook::Hidden => Look::Nothing,
        ActorLook::NotYet => Look::Undecided,
    };
}

/// Lets shown boss icons re-check whether their boss is defeated, so the check mark follows kills and respawns.
fn refresh_boss_states(ui: &mut Ui, live: &Live, assets: &mut Assets, world: Obj) {
    let widgets: Vec<Obj> = live
        .markers
        .values()
        .filter(|view| view.shown && matches!(view.look, Look::Boss(_)))
        .filter_map(|view| view.visual.as_ref().filter(|visual| visual.game_icon)?.widget.get())
        .collect();
    if widgets.is_empty() {
        return;
    }
    if let Load::Ready(icon) = ui.boss_icon(assets, world) {
        for widget in widgets {
            icon.update_state.call(widget, |_| {});
        }
    }
}

/// Lets shown game icons follow their location's state, so a teleporter switches to its unlocked look as soon
/// as it is unlocked.
fn refresh_icon_states(ui: &mut Ui, live: &Live, locations: &mut Locations) {
    for view in live.markers.values().filter(|view| view.shown) {
        let (Look::GameIcon(class), MarkerKey::Location(location)) = (&view.look, view.key) else { continue };
        let Some(widget) = view.visual.as_ref().and_then(|visual| visual.widget.get()) else { continue };
        let (Some(class), Some(location)) = (class.get(), location.get()) else { continue };
        if let Some(point) = locations.point(location) {
            ui.refresh_icon(class, widget, point);
        }
    }
}

/// A living boss whose spawner icon is already on the minimap is drawn by that icon instead.
fn covered_by_spawner(live: &Live, view: &MarkerView) -> bool {
    let (Look::Character(key), Some(_), Some((x, y))) = (&view.look, view.actor_base, view.world) else { return false };
    view.kind == FIELD_BOSS
        && live.boss_spawners.get(&key.character).is_some_and(|spots| {
            spots
                .iter()
                .any(|&(sx, sy)| (sx - x).hypot(sy - y) <= BOSS_ACTOR_NEAR_SPAWNER)
        })
}

fn cull_markers(
    ui: &mut Ui,
    live: &mut Live,
    sources: &mut Sources,
    assets: &mut Assets,
    context: &CullContext,
) {
    live.needs_cull = false;
    for (widget, class) in std::mem::take(&mut live.size_reports) {
        if let (Some(widget), Some(class)) = (widget.get(), class.get()) {
            let size: Vec2 = ui.desired_size.call(widget, |_| {}).returned();
            host::info(&format!("icon widget {}: {:.0}x{:.0} before scaling", class.name(), size.x, size.y));
        }
    }

    let radius = context.view_units * CULL_RADIUS;
    let limit = radius * radius;
    let (px, py) = (context.player.location.x, context.player.location.y);
    let region = live.region;
    let mut budget = Budget {
        creates: CREATES_PER_CULL,
        describes: DESCRIBES_PER_CULL,
        bakes: BAKES_PER_CULL,
    };
    live.moving_shown.clear();

    let keys: Vec<MarkerKey> = live.markers.keys().copied().collect();
    for key in keys {
        let Some(mut view) = live.markers.remove(&key) else { continue };
        cull_one(ui, live, &mut view, sources, assets, context, &mut budget, (px, py, limit, region));
        live.markers.insert(key, view);
    }
}

#[allow(clippy::too_many_arguments)]
fn cull_one(
    ui: &mut Ui,
    live: &mut Live,
    view: &mut MarkerView,
    sources: &mut Sources,
    assets: &mut Assets,
    context: &CullContext,
    budget: &mut Budget,
    (px, py, limit, region): (f64, f64, f64, &'static Region),
) {
    if let MarkerKey::Model(model) = view.key {
        match model.get() {
            Some(model) => {
                view.present = !(matches!(view.kind, actors::CHEST | actors::JUNK)
                    && sources.objects.is_some_and(|objects| objects.opened(model)));
            }
            None => view.present = false,
        }
    }
    if let (MarkerKey::Actor(actor), Some(base), Some(actors)) = (view.key, view.actor_base, sources.actors.as_deref_mut()) {
        // The world-wide model scan already covers these, near and far, once it has found any.
        let covered = match base {
            Base::Chest => live.model_kinds.contains(&actors::CHEST),
            Base::Rock => live.model_kinds.contains(&actors::ORE),
            Base::FruitTree => live.model_kinds.contains(&actors::SKILL_FRUIT),
            _ => false,
        };
        if covered {
            view.look = Look::Nothing;
            view.drop_visual(ui);
            return;
        }
        match actor.get() {
            Some(actor) => {
                if base.moves() {
                    view.set_position(actors.position(actor));
                } else if base == Base::Rock {
                    let revision = actors.pool_revision(actor);
                    if view.pool_revision != Some(revision) {
                        view.pool_revision = Some(revision);
                        view.set_position(actors.position(actor));
                    }
                }
            }
            None => {
                view.look = Look::Nothing;
                view.drop_visual(ui);
                return;
            }
        }
    } else if view.moves {
        if let (MarkerKey::Location(location), Some(locations)) = (view.key, sources.locations.as_deref_mut()) {
            if let Some(location) = location.get() {
                view.set_position(locations.position(location));
            }
        }
    }

    // Inside a dungeon the player is off the map art, and so is everything around them. Dungeons under the
    // overworld share its ground plan, so there only what is near the player's height belongs to it.
    let stage = context.player.in_stage;
    let level = (view.height - context.player.location.z).abs() <= STAGE_HEIGHT;
    let near = view.world.filter(|&(x, y)| {
        region.contains(x, y) == region.contains(px, py) && (!stage || level) && (x - px).powi(2) + (y - py).powi(2) <= limit
    });

    if near.is_some() && matches!(view.look, Look::Undecided) {
        if let (MarkerKey::Actor(actor), Some(Base::Rock), Some(actors)) = (view.key, view.actor_base, sources.actors.as_deref_mut())
            && actor.get().is_some_and(|actor| !actors.present(actor, Base::Rock))
        {
            return;
        }
        if budget.describes == 0 {
            live.needs_cull = true;
            return;
        }
        budget.describes -= 1;
        if let Some(actors) = sources.actors.as_deref_mut() {
            profile::time("describe actor", || describe_actor(ui, view, actors, sources.icons, context.player));
        }
    }
    if !view.drawable() {
        return;
    }

    if let (MarkerKey::Actor(actor), Some(base), Some(actors), true) =
        (view.key, view.actor_base, sources.actors.as_deref_mut(), near.is_some())
    {
        if let Some(actor) = actor.get() {
            let player = context.player;
            // A player character right where the camera's pawn is counts as you too: riding or gliding can
            // leave your own character as a different actor from the controller's pawn.
            let on_you = view
                .world
                .is_some_and(|(x, y)| (x - player.location.x).hypot(y - player.location.y) < LOCAL_PLAYER_RADIUS);
            let local = view.kind == actors::PLAYER && (on_you || actors.is_local_player(actor, player.pawn, player.controller));
            view.present = !local && actors.present(actor, base) && !covered_by_spawner(live, view);
        }
    }

    let shown_type = match &view.pickup {
        Some(item) => context.settings.shows_pickup(item),
        None => context.settings.shows_kind(view.kind),
    };
    // Outside a rounded shape's edge the map is not drawn, so neither are markers there.
    let in_shape = |&(x, y): &(f64, f64)| live.layers.inside(live.layers.frame_point(live.layer_point(x, y)));
    let target = near.filter(|at| view.present && shown_type && in_shape(at));
    if target.is_some() && view.visual.is_none() {
        if budget.creates == 0 {
            live.needs_cull = true;
            return;
        }
        match profile::time("create marker", || create_visual(ui, live, view, sources, assets, context)) {
            Ok(Some(mut visual)) => {
                budget.creates -= 1;
                if let (true, Some(widget)) = (live.icon_angle != 0.0, visual.widget.get()) {
                    ui.set_angle.call(widget, |a| a.set(0, live.icon_angle));
                }
                visual.landmark = is_landmark(view);
                if visual.landmark && context.settings.landmark_backing {
                    visual.backing = make_backing(ui, live, view, context.settings, visual.game_icon);
                }
                view.visual = Some(visual);
                scale_marker(ui, view, live.icon_look());
            }
            Ok(None) => return,
            Err(reason) => {
                host::warning(&format!("marker type {} not drawn: {reason}", view.kind));
                view.failed = true;
                return;
            }
        }
    }

    let handles = view
        .visual
        .as_ref()
        .and_then(|visual| Some((visual.widget.get()?, visual.slot.get()?, visual.game_icon)));
    let Some((widget, slot, game_icon)) = handles else {
        view.visual = None;
        view.shown = false;
        return;
    };
    if target.is_some() && view.visual.as_ref().is_some_and(|visual| visual.pending.is_some()) {
        profile::time("portrait", || {
            finish_pending_texture(ui, view, widget, sources, assets, budget, context.player.world);
        });
    }
    let waiting = view.visual.as_ref().is_some_and(|visual| visual.pending.is_some());

    if let Some((x, y)) = target {
        place_icon(ui, view, slot, live.layer_uv(x, y), live.uv_tolerance(0.05), live.icon_look());
    }
    let show = target.is_some() && !waiting && !(game_icon && ui.is_icon_hidden(widget));
    if show != view.shown {
        view.shown = show;
        ui.set_shown(widget, show);
        if let Some(backing) = view.visual.as_ref().and_then(|visual| visual.backing?.0.get()) {
            ui.set_shown(backing, show);
        }
    }
    update_elevation(ui, live, view, show, context, budget, sources.badges.as_deref_mut());
    if show && (view.moves || view.actor_base.is_some_and(Base::moves)) {
        live.moving_shown.push(view.key);
    }
}

/// Which marker layer an icon goes on (over the fog, or under it so unexplored areas cover it) and its order
/// there: pals at the bottom, then shiny pals, boss pals and every other marker. The game keeps a few markers
/// over its fog, and with markers shown in unexplored areas everything goes over it.
fn marker_place(view: &MarkerView, settings: &Settings) -> (bool, i32) {
    let z = match &view.look {
        _ if is_landmark(view) => Z_LANDMARK,
        Look::Character(key) if key.boss => Z_BOSS_PAL,
        Look::Character(key) if key.shiny => Z_SHINY_PAL,
        Look::Character(_) => Z_PAL,
        Look::Boss(_) => Z_BOSS_PAL,
        _ => Z_OTHER,
    };
    (settings.markers_in_fog || view.above_fog, z)
}

/// Anchors an icon at `at`, a fraction of the map layer, so it follows the layer through any resize.
fn place_icon(ui: &mut Ui, view: &mut MarkerView, slot: Obj, at: Vec2, tolerance: f64, look: IconLook) {
    if view
        .placed
        .is_none_or(|placed| (placed.x - at.x).abs() > tolerance || (placed.y - at.y).abs() > tolerance)
    {
        ui.slot_anchors.call(slot, |a| a.set(0, Anchors { min: at, max: at }));
        if let Some(backing) = view.visual.as_ref().and_then(|visual| visual.backing?.1.get()) {
            ui.slot_anchors.call(backing, |a| a.set(0, Anchors { min: at, max: at }));
        }
        view.placed = Some(at);
    }
    place_elevation(ui, view, look);
}

/// The dark disc behind a landmark icon, one layer under it.
fn make_backing(ui: &mut Ui, live: &Live, view: &MarkerView, settings: &Settings, game_icon: bool) -> Option<(Weak, Weak)> {
    let texture = live.backing_texture.and_then(Weak::get)?;
    let (over_fog, _) = marker_place(view, settings);
    let (tree, layer) = (live.tree.get()?, live.layers.canvas(over_fog)?);
    let size = IMAGE_ICON_SIZE * settings.icon_scale * if game_icon { GAME_ICON_BACKING_SIZE } else { BACKING_SIZE };
    let (image, slot) = ui.image(tree, layer, Vec2::default(), Vec2 { x: size, y: size }, Z_LANDMARK_BACKING).ok()?;
    ui.center(slot);
    ui.set_texture(image, texture);
    ui.set_shown(image, false);
    Some((Weak::new(image), Weak::new(slot)))
}

/// Keeps the chevron centred just under its icon on screen. It is anchored with the icon, and its offset is
/// turned by the icons' counter-turn, so the turning map carries it back to straight below.
fn place_elevation(ui: &mut Ui, view: &mut MarkerView, look: IconLook) {
    let (Some(icon_at), Some(half_height)) = (view.placed, view.visual.as_ref().map(|visual| visual.half_height)) else {
        return;
    };
    let Some(elevation) = view.elevation.as_mut() else { return };
    let Some(slot) = elevation.slot.get() else { return };
    if elevation.placed != Some((icon_at, look)) {
        let factor = view.visual.as_ref().map_or(look.zoom, |visual| visual.factor(look));
        let distance = (half_height + CHEVRON_SIZE.y * 0.5) * factor + 1.0;
        let (sin, cos) = f64::from(look.angle).to_radians().sin_cos();
        let below = Vec2 { x: -distance * sin, y: distance * cos };
        ui.slot_anchors.call(slot, |a| a.set(0, Anchors { min: icon_at, max: icon_at }));
        ui.slot_position.call(slot, |a| a.set(0, below));
        elevation.placed = Some((icon_at, look));
    }
}

#[allow(clippy::too_many_arguments)]
fn update_elevation(
    ui: &mut Ui,
    live: &mut Live,
    view: &mut MarkerView,
    shown: bool,
    context: &CullContext,
    budget: &mut Budget,
    badges: Option<&mut Badges>,
) {
    let dz = view.height - context.player.location.z;
    let wanted = (shown && dz.is_finite() && dz.abs() > ELEVATION_THRESHOLD).then_some(dz > 0.0);
    let Some(up) = wanted else {
        if let Some(elevation) = view.elevation.as_mut() {
            if elevation.up.take().is_some() {
                if let Some(widget) = elevation.widget.get() {
                    ui.set_shown(widget, false);
                }
            }
        }
        return;
    };
    let Some(badges) = badges else { return };
    let _scope = profile::scope("elevation arrow");

    if view.elevation.is_none() {
        let (over_fog, z_order) = marker_place(view, context.settings);
        let (Some(tree), Some(layer)) = (live.tree.get(), live.layers.canvas(over_fog)) else { return };
        if budget.creates == 0 {
            live.needs_cull = true;
            return;
        }
        budget.creates -= 1;
        let size = Vec2 {
            x: CHEVRON_SIZE.x * context.settings.icon_scale,
            y: CHEVRON_SIZE.y * context.settings.icon_scale,
        };
        let Ok((image, slot)) = ui.image(tree, layer, Vec2::default(), size, z_order) else { return };
        ui.center(slot);
        ui.set_shown(image, false);
        if live.icon_angle != 0.0 {
            ui.set_angle.call(image, |a| a.set(0, live.icon_angle));
        }
        if live.icon_zoom != 1.0 {
            let zoom = live.icon_zoom;
            ui.set_render_scale.call(image, |a| a.set(0, Vec2 { x: zoom, y: zoom }));
        }
        view.elevation = Some(Elevation {
            widget: Weak::new(image),
            slot: Weak::new(slot),
            up: None,
            placed: None,
        });
    }

    let Some(elevation) = view.elevation.as_mut() else { return };
    let Some(widget) = elevation.widget.get() else {
        view.elevation = None;
        return;
    };
    if elevation.up != Some(up) {
        let Some(texture) = badges.chevron(context.player.world, up) else { return };
        ui.set_texture(widget, texture);
        if elevation.up.is_none() {
            ui.set_shown(widget, true);
        }
        elevation.up = Some(up);
    }
    place_elevation(ui, view, live.icon_look());
}

/// Applies a portrait once it has loaded, as a badge when possible.
fn finish_pending_texture(
    ui: &mut Ui,
    view: &mut MarkerView,
    image: Obj,
    sources: &mut Sources,
    assets: &mut Assets,
    budget: &mut Budget,
    world: Obj,
) {
    let Some(pending) = view.visual.as_ref().and_then(|visual| visual.pending.as_ref()) else { return };
    let (key, path) = (pending.key, pending.path.clone());

    let made = sources.badges.as_deref_mut().and_then(|badges| badges.cached(world, key));
    let texture = match made {
        Some(badge) => Some(badge),
        None => match (assets.get(world, &path), sources.badges.as_deref_mut()) {
            (Load::Waiting, _) => return,
            (Load::Failed, _) => None,
            (Load::Ready(source), Some(badges)) if !badges.has_failed(key) => {
                if budget.bakes == 0 {
                    return;
                }
                budget.bakes -= 1;
                match badges.bake_step(world, key, source) {
                    BakeStep::Done(badge) => Some(badge),
                    BakeStep::Failed => Some(source),
                    BakeStep::Waiting => return,
                }
            }
            (Load::Ready(source), _) => Some(source),
        },
    };

    if let Some(texture) = texture {
        ui.set_texture(image, texture);
    } else {
        view.failed = true;
    }
    if let Some(visual) = view.visual.as_mut() {
        visual.pending = None;
    }
}

fn follow_moving(ui: &mut Ui, live: &mut Live, actors: &mut ActorInspector) {
    for index in 0..live.moving_shown.len() {
        let key = live.moving_shown[index];
        let MarkerKey::Actor(actor) = key else { continue };
        let Some(actor) = actor.get() else { continue };
        let Some(position) = actors.position(actor) else { continue };
        let at = live.layer_uv(position.x, position.y);
        let (tolerance, look) = (live.uv_tolerance(0.3), live.icon_look());
        let Some(view) = live.markers.get_mut(&key) else { continue };
        view.set_position(Some(position));
        let Some(slot) = view.visual.as_ref().and_then(|visual| visual.slot.get()) else { continue };
        place_icon(ui, view, slot, at, tolerance, look);
    }
}

pub struct Minimap {
    ui: Ui,
    settings: Settings,
    locations: Option<Locations>,
    actors: Option<ActorInspector>,
    icons: Option<IconTable>,
    classes: Option<IconClassFinder>,
    badges: Option<Badges>,
    objects: Option<MapObjects>,
    object_list: Vec<MapObject>,
    terrain: Option<Terrain>,
    sources_tried: bool,
    tracked: HashMap<Weak, Base>,
    assets: Assets,
    live: Option<Live>,
    next_attempt: Instant,
    found: Vec<Obj>,
    unsaved_since: Option<Instant>,
    menu: Menu,
    controller: Option<Weak>,
    megazoom: bool,
    speed: f64,
    last_sample: Option<(Vec3, Instant)>,
    zoom_level: usize,
    zoom_level_since: Instant,
    shown_view: f64,
    last_view_step: Option<Instant>,
    /// Whether the player was inside a dungeon at the last update, which picks the zoom setting.
    in_stage: bool,
}

impl Minimap {
    pub fn new(settings: Settings, engine: Obj) -> Result<Self, String> {
        let locations = Locations::new()
            .map_err(|reason| host::warning(&format!("map markers disabled: {reason}")))
            .ok();
        let actors = ActorInspector::new()
            .map_err(|reason| host::warning(&format!("world actors disabled: {reason}")))
            .ok();
        let badges = crate::settings::cache_folder()
            .ok_or_else(|| "mod folder not found".to_string())
            .and_then(Badges::new)
            .map_err(|reason| host::warning(&format!("pal badges and elevation arrows disabled: {reason}")))
            .ok();
        Ok(Self {
            ui: Ui::resolve()?,
            settings,
            locations,
            actors,
            icons: None,
            classes: None,
            badges,
            objects: MapObjects::new()
                .map_err(|reason| host::warning(&format!("world-wide map objects disabled: {reason}")))
                .ok(),
            object_list: Vec::new(),
            terrain: Terrain::new()
                .map_err(|reason| host::warning(&format!("rendered terrain disabled: {reason}")))
                .ok(),
            sources_tried: false,
            tracked: HashMap::new(),
            assets: Assets::new(engine)?,
            live: None,
            next_attempt: Instant::now(),
            found: Vec::new(),
            unsaved_since: None,
            menu: Menu::new()?,
            controller: None,
            megazoom: false,
            speed: 0.0,
            last_sample: None,
            zoom_level: 0,
            zoom_level_since: Instant::now(),
            shown_view: 0.0,
            last_view_step: None,
            in_stage: false,
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn toggle_menu(&mut self) {
        self.menu.toggle();
    }

    /// Whether every key should be watched, to capture a new binding in the menu.
    pub fn capturing_key(&self) -> bool {
        self.menu.capturing_key()
    }

    /// Picture for a marker category's row in the settings menu, from the first of its kinds that has one.
    fn category_icon(icons: Option<&IconTable>, category: usize) -> Option<String> {
        // The game's map draws these with textures the icon table does not list (names from PalMiniMap).
        let compass = |name: &str| format!("/Game/Pal/Texture/UI/InGame/{name}.{name}");
        match CATEGORIES[category].key {
            "DeathDrops" => return Some(compass("T_prt_map_death_1")),
            "TreasureMaps" => return Some(compass("T_icon_compass_TreasureMap_01")),
            _ => {}
        }
        CATEGORIES[category].kinds.iter().find_map(|&kind| {
            if kind >= actors::WILD_PAL {
                actors::kind_icon(kind)
            } else {
                icons?.for_kind(kind).map(str::to_owned)
            }
        })
    }

    /// Runs the settings menu, then applies whatever it changed.
    fn run_menu(&mut self, player: &Player, keys: &Keys) {
        let before = self.settings.clone();
        let Self { menu, settings, assets, icons, objects, ui, live, locations, .. } = self;
        let icons = icons.as_ref();
        let objects = objects.as_ref();
        let live = live.as_ref();
        let category_texture = |category: usize| Self::category_icon(icons, category);
        let mut category_widget =
            |category: usize| category_widget(ui, live, locations.as_mut(), player, category);
        let pickups = |settings: &Settings| pickup_rows(objects, settings);
        let mut content = MenuContent {
            category_texture: &category_texture,
            category_widget: &mut category_widget,
            pickups: &pickups,
        };
        if menu.update(player.world, player.controller, settings, keys, assets, &mut content) {
            self.apply_settings(&before, player.world);
        }
    }

    /// Applies settings that changed in the menu to the live minimap.
    fn apply_settings(&mut self, before: &Settings, world: Obj) {
        if self.settings == *before {
            return;
        }
        self.unsaved_since = Some(Instant::now());
        let settings = &self.settings;
        let Some(live) = self.live.as_mut() else { return };
        let ui = &mut self.ui;
        if settings.opacity != before.opacity
            && let Some(widget) = live.widget.get()
        {
            ui.set_render_opacity.call(widget, |a| a.set(0, settings.opacity as f32));
        }
        let layout = settings.size != before.size
            || settings.offset_x != before.offset_x
            || settings.offset_y != before.offset_y
            || settings.corner != before.corner
            || settings.roundness != before.roundness;
        if layout || settings.show_hint != before.show_hint || settings.menu_keys != before.menu_keys {
            place_frame(ui, self.menu.widgets(), live, settings, world);
        }
        if settings.view_cone != before.view_cone || settings.rotate_with_camera != before.rotate_with_camera {
            live.cone = None;
            live.layer_position = None;
        }
        if layout || settings.zoom_metres != before.zoom_metres {
            live.needs_layout = true;
        }
        // Both decide how markers are built: their size, and whether they sit under or over the fog.
        if settings.icon_scale != before.icon_scale
            || settings.markers_in_fog != before.markers_in_fog
            || settings.landmark_backing != before.landmark_backing
        {
            for view in live.markers.values_mut() {
                view.drop_visual(ui);
            }
            live.needs_cull = true;
        }
        if settings.categories != before.categories || settings.groups != before.groups || settings.pickups != before.pickups {
            live.needs_cull = true;
        }
    }

    /// Native classes whose spawns are worth reporting.
    pub fn actor_filter(&self) -> Vec<usize> {
        self.actors.as_ref().map(|actors| actors.classes().filter()).unwrap_or_default()
    }

    pub fn actor_began(&mut self, actor: Weak) {
        let Some(inspector) = self.actors.as_mut() else { return };
        let Some(object) = actor.get() else { return };
        if let Some(base) = inspector.classes().base(object)
            && inspector.wanted(object, base)
        {
            self.tracked.insert(actor, base);
        }
    }

    pub fn actor_ended(&mut self, actor: Weak) {
        if self.tracked.remove(&actor).is_none() {
            return;
        }
        if let Some(live) = self.live.as_mut() {
            if let Some(mut view) = live.markers.remove(&MarkerKey::Actor(actor)) {
                view.drop_visual(&mut self.ui);
            }
        }
    }

    /// World units across the minimap: the zoom setting, the megazoom range while that is on, or the zoom
    /// widened by the step the current speed calls for.
    fn view_units(&self) -> f64 {
        if self.megazoom {
            return (self.settings.megazoom_metres * 100.0).clamp(MIN_VIEW, MAX_MEGAZOOM_VIEW);
        }
        let base = self.zoom_units();
        if self.settings.auto_zoom { base * AUTO_ZOOM_STEPS[self.zoom_level] } else { base }
    }

    /// The zoom setting for where the player is, the overworld's or the dungeons', for stepping with the keys.
    fn zoom_units(&self) -> f64 {
        let metres = if self.in_stage { self.settings.dungeon_zoom_metres } else { self.settings.zoom_metres };
        (metres * 100.0).clamp(MIN_VIEW, MAX_VIEW)
    }

    pub fn zoom(&mut self, steps: i32) {
        let view = (self.zoom_units() * ZOOM_FACTOR.powi(-steps)).clamp(MIN_VIEW, MAX_VIEW);
        if (view - self.zoom_units()).abs() < 1.0 {
            return;
        }
        let metres = view / 100.0;
        if self.in_stage {
            self.settings.dungeon_zoom_metres = metres;
        } else {
            self.settings.zoom_metres = metres;
        }
        self.unsaved_since = Some(Instant::now());
        host::info(&format!("{}zoom: {metres:.0} m across", if self.in_stage { "dungeon " } else { "" }));
        if let Some(live) = self.live.as_mut() {
            live.needs_layout = true;
        }
    }

    pub fn toggle(&mut self) {
        self.settings.enabled = !self.settings.enabled;
        self.unsaved_since = Some(Instant::now());
        host::info(if self.settings.enabled { "minimap turned on" } else { "minimap turned off" });
    }

    /// Settings changed in game, once they have been stable long enough to be worth writing.
    pub fn settings_to_save(&mut self) -> Option<Settings> {
        let since = self.unsaved_since?;
        if since.elapsed() < SAVE_AFTER {
            return None;
        }
        self.unsaved_since = None;
        Some(self.settings.clone())
    }

    /// Starts background loads of what the build needs. True once none are still loading.
    fn build_assets_ready(&mut self, world: Obj, region: &Region) -> bool {
        let mut paths = if self.sources_tried { Vec::new() } else { icons::asset_paths() };
        paths.extend([region.texture, PLAYER_ARROW, FOG_MATERIAL].map(String::from));
        let mut ready = true;
        for path in &paths {
            if matches!(self.assets.get(world, path), Load::Waiting) {
                ready = false;
            }
        }
        ready
    }

    fn load_sources(&mut self) {
        if self.sources_tried {
            return;
        }
        self.sources_tried = true;
        match IconTable::load() {
            Ok(icons) => self.icons = Some(icons),
            Err(reason) => host::warning(&format!("marker icon table unavailable: {reason}")),
        }
        match IconClassFinder::new() {
            Ok(classes) => self.classes = Some(classes),
            Err(reason) => host::warning(&format!("map icon widgets unavailable, using plain icons: {reason}")),
        }
    }

    fn sync_actors(&mut self, player: &Player) {
        let (Some(live), Some(inspector)) = (self.live.as_mut(), self.actors.as_mut()) else { return };
        self.tracked.retain(|actor, _| actor.get().is_some());
        for (&actor, &base) in &self.tracked {
            let key = MarkerKey::Actor(actor);
            if live.markers.contains_key(&key) {
                continue;
            }
            let Some(object) = actor.get() else { continue };
            if object == player.pawn {
                continue;
            }
            let mut view = MarkerView::new(key, 0, Look::Undecided, false, base.moves(), inspector.position(object));
            view.actor_base = Some(base);
            live.markers.insert(key, view);
            live.needs_cull = true;
        }
        let tracked = &self.tracked;
        let ui = &mut self.ui;
        live.markers.retain(|key, view| {
            let keep = match key {
                MarkerKey::Actor(actor) => tracked.contains_key(actor),
                _ => true,
            };
            if !keep {
                view.drop_visual(ui);
            }
            keep
        });
    }

    pub fn update(&mut self, player: &Player, keys: &Keys) {
        self.controller = Some(Weak::new(player.controller));
        profile::time("settings menu", || self.run_menu(player, keys));
        if self.live.is_none() {
            if Instant::now() < self.next_attempt {
                return;
            }
            let _scope = profile::scope("build");
            let region = self.ui.current_region(player.world);
            if !profile::time("wait for assets", || self.build_assets_ready(player.world, region)) {
                return;
            }
            profile::time("icon sources", || self.load_sources());
            match self.build(player, region) {
                Ok(live) => {
                    host::info(&format!("minimap built ({})", live.region.name));
                    self.live = Some(live);
                    if let Some(terrain) = self.terrain.as_mut() {
                        terrain.forget_tiles();
                    }
                }
                Err(reason) => {
                    host::warning(&format!("minimap not built: {reason}"));
                    self.next_attempt = Instant::now() + RETRY_AFTER;
                    return;
                }
            }
        }

        let slow_due = self.live.as_ref().is_some_and(|live| Instant::now() >= live.next_slow);
        if slow_due {
            profile::time("actor sync", || self.sync_actors(player));
        }

        self.track_speed(player);
        self.in_stage = player.in_stage;
        let (view_units, settled) = self.animate_view();
        let Self {
            ui,
            settings,
            locations,
            actors,
            icons,
            classes,
            badges,
            objects,
            object_list,
            terrain,
            assets,
            live: live_slot,
            found,
            unsaved_since,
            menu,
            ..
        } = self;
        let w = menu.widgets();
        let Some(live) = live_slot.as_mut() else { return };
        let (Some(widget), Some(arrow)) = (live.widget.get(), live.arrow.get()) else {
            host::info("minimap widgets are gone, rebuilding");
            *live_slot = None;
            return;
        };

        let in_base = settings.hide_in_base && player.in_base;
        let show = settings.enabled && !player.screen_open && !player.fading && !in_base;
        if show != live.shown {
            live.shown = show;
            if settings.enabled {
                let why = if in_base {
                    "in your base"
                } else if player.fading {
                    "screen fading"
                } else {
                    "game screen open"
                };
                host::info(&if show { "showing minimap".to_string() } else { format!("{why}, hiding minimap") });
            }
            ui.set_shown(widget, show);
        }
        if !show {
            return;
        }

        let now = Instant::now();
        if now >= live.next_slow {
            let _scope = profile::scope("once a second");
            live.next_slow = now + SLOW_CHECK_EVERY;
            let pixel = profile::time("viewport scale", || ui.pixel_size(player.world));
            if (pixel - live.pixel).abs() > 1e-6 {
                live.pixel = pixel;
                live.layer_position = None;
            }
            if let Some(classes) = classes.as_mut() {
                let _scope = profile::scope("map icon classes");
                if classes.poll() {
                    upgrade_location_looks(ui, live, classes);
                }
            }
            if let Some(data) = profile::time("world map data", || ui.world_map_data(player.world)) {
                profile::time("map region", || refresh_region(ui, w, live, data, assets, player.world));
                profile::time("fog attach", || refresh_fog(ui, w, live, data));
                if !live.bosses_loaded && now >= live.next_boss_try {
                    profile::time("field bosses", || load_bosses(ui, live, player.world, data));
                }
            }
            let mut sources = Sources {
                locations: locations.as_mut(),
                actors: None,
                icons: icons.as_ref(),
                classes: classes.as_ref(),
                badges: None,
                objects: None,
            };
            profile::time("locations", || scan_locations(ui, live, &mut sources, player.world, found));
            profile::time("boss states", || refresh_boss_states(ui, live, assets, player.world));
            if let Some(locations) = locations.as_mut() {
                profile::time("icon states", || refresh_icon_states(ui, live, locations));
            }
            if now >= live.next_object_scan {
                live.next_object_scan = now + OBJECT_SCAN_EVERY;
                if let (Some(objects), Some(icons)) = (objects.as_mut(), icons.as_ref()) {
                    let learned = profile::time("map objects", || {
                        scan_map_objects(ui, live, objects, icons, settings, player.world, object_list)
                    });
                    if learned {
                        *unsaved_since = Some(now);
                    }
                }
            }
        }
        if view_units != live.view_units {
            live.view_units = view_units;
            live.needs_layout = true;
        }
        if live.needs_layout {
            profile::time("layout", || apply_layout(w, live, view_units, settled));
        }
        live.layers.set_fog_shown(w, settings.fog_of_war && !player.in_stage);
        // Before the cull: it checks markers against the map's shape at the map's current place.
        profile::time("move map", || move_map(ui, w, live, arrow, player, settings));
        place_pins(ui, live, settings);

        let wanted_zoom = if settings.icon_zoom {
            (ICON_ZOOM_REFERENCE / view_units)
                .powf(settings.icon_zoom_strength)
                .clamp(ICON_ZOOM_LIMITS.0, ICON_ZOOM_LIMITS.1)
        } else {
            1.0
        };
        let off_by = (wanted_zoom / live.icon_zoom - 1.0).abs();
        let landmark_resized = settings.landmark_scale != live.landmark_scale;
        if off_by > ICON_ZOOM_STEP || (settled && off_by > 1e-6) || landmark_resized {
            let _scope = profile::scope("icon zoom");
            live.icon_zoom = wanted_zoom;
            live.landmark_scale = settings.landmark_scale;
            let look = live.icon_look();
            for view in live.markers.values_mut() {
                scale_marker(ui, view, look);
            }
        }

        if live.needs_cull || now >= live.next_cull {
            let _scope = profile::scope("cull");
            live.next_cull = now + CULL_EVERY;
            let context = CullContext {
                player,
                settings,
                view_units,
            };
            let mut sources = Sources {
                locations: locations.as_mut(),
                actors: actors.as_mut(),
                icons: icons.as_ref(),
                classes: classes.as_ref(),
                badges: badges.as_mut(),
                objects: objects.as_ref(),
            };
            cull_markers(ui, live, &mut sources, assets, &context);
            let (locations, icons) = (sources.locations, sources.icons);
            profile::time("edge pins", || choose_pins(ui, live, locations, icons, assets, &context));
        } else if let Some(actors) = actors.as_mut() {
            profile::time("follow moving", || follow_moving(ui, live, actors));
        }
        if let Some(terrain) = terrain.as_mut() {
            let region = live.region;
            profile::time("terrain", || {
                terrain.update(w, &mut live.layers, region, player, settings, view_units);
            });
        }
    }

    /// The zoom on its way to the one asked for, eased so a change glides instead of jumping. True once there.
    fn animate_view(&mut self) -> (f64, bool) {
        let target = self.view_units();
        let now = Instant::now();
        let seconds = self.last_view_step.map_or(0.0, |at| now.duration_since(at).as_secs_f64()).min(0.1);
        self.last_view_step = Some(now);
        if self.shown_view <= 0.0 {
            self.shown_view = target;
        }
        // Eased in log space, so zooming in and out by the same factor take the same time.
        let blend = 1.0 - (-seconds / ZOOM_SMOOTHING).exp();
        let shown = (self.shown_view.ln() + (target.ln() - self.shown_view.ln()) * blend).exp();
        self.shown_view = if (shown / target - 1.0).abs() < ZOOM_SNAP { target } else { shown };
        (self.shown_view, self.shown_view == target)
    }

    pub fn toggle_megazoom(&mut self) {
        self.megazoom = !self.megazoom;
        host::info(if self.megazoom { "megazoom on" } else { "megazoom off" });
    }
}

/// Moves and turns the map under the player, keeps icons upright, and turns the arrow and view cone.
fn move_map(ui: &mut Ui, w: &mut Widgets, live: &mut Live, arrow: Obj, player: &Player, settings: &Settings) {
    {
        let map_angle = if settings.rotate_with_camera { (-player.camera_yaw).rem_euclid(360.0) as f32 } else { 0.0 };
        let half = live.size / 2.0;
        let (u, v) = live.region.to_uv(player.location.x, player.location.y);
        let mut position = Vec2 { x: half - u * live.layer_size.x, y: half - v * live.layer_size.y };
        // Whole screen pixels only, or Slate rounds the map and each icon differently every frame. A turning
        // map is never on whole pixels anyway.
        if map_angle == 0.0 {
            let pixel = live.pixel;
            let snap = |value: f64| (value / pixel).round() * pixel;
            position = Vec2 { x: snap(position.x), y: snap(position.y) };
        }
        if live.layer_position != Some(position) || angle_moved(live.map_angle, map_angle, 0.05) {
            live.layer_position = Some(position);
            live.map_angle = map_angle;
            live.layers.place_all(w, position, Vec2 { x: u, y: v }, map_angle);
        }

        // Icons turn back by the map's turn, so they stay upright.
        let icon_angle = (-map_angle).rem_euclid(360.0);
        if angle_moved(live.icon_angle, icon_angle, 0.5) {
            live.icon_angle = icon_angle;
            let look = live.icon_look();
            for view in live.markers.values_mut() {
                let visual = view.visual.as_ref().and_then(|visual| visual.widget.get());
                let elevation = view.elevation.as_ref().and_then(|elevation| elevation.widget.get());
                for widget in [visual, elevation].into_iter().flatten() {
                    ui.set_angle.call(widget, |a| a.set(0, icon_angle));
                }
                if elevation.is_some() {
                    place_elevation(ui, view, look);
                }
            }
        }

        let angle = (player.yaw as f32 + map_angle).rem_euclid(360.0);
        if live.arrow_angle.is_none_or(|last| angle_moved(last, angle, 0.05)) {
            live.arrow_angle = Some(angle);
            ui.set_angle.call(arrow, |a| a.set(0, angle));
        }

        let cone = (settings.view_cone, (player.camera_yaw as f32 + map_angle).rem_euclid(360.0));
        if live.cone.is_none_or(|(shown, angle)| shown != cone.0 || angle_moved(angle, cone.1, 0.5)) {
            live.cone = Some(cone);
            live.layers.cone(w, cone.0, cone.1);
        }
    }
}

impl Minimap {

    /// Ground speed, smoothed, for the zoom that follows it, and the zoom step that speed calls for.
    fn track_speed(&mut self, player: &Player) {
        let now = Instant::now();
        let here = player.location;
        if let Some((before, at)) = self.last_sample {
            let seconds = now.duration_since(at).as_secs_f64();
            if seconds > 0.0 {
                let metres = (here.x - before.x).hypot(here.y - before.y) / 100.0;
                // A teleport or respawn is not speed.
                let sample = if metres / seconds > MAX_SPEED { self.speed } else { metres / seconds };
                let blend = (seconds / SPEED_SMOOTHING).min(1.0);
                self.speed += (sample - self.speed) * blend;
            }
        }
        self.last_sample = Some((here, now));

        let level = self.zoom_level;
        let wanted = if level + 1 < AUTO_ZOOM_STEPS.len() && self.speed > AUTO_ZOOM_UP[level] {
            level + 1
        } else if level > 0 && self.speed < AUTO_ZOOM_DOWN[level - 1] {
            level - 1
        } else {
            level
        };
        if wanted != level && self.zoom_level_since.elapsed() >= AUTO_ZOOM_DWELL {
            self.zoom_level = wanted;
            self.zoom_level_since = now;
        }
    }
}

/// Whether two angles in degrees differ by more than `by`, the short way round.
fn angle_moved(from: f32, to: f32, by: f32) -> bool {
    let difference = (to - from).rem_euclid(360.0);
    difference.min(360.0 - difference) > by
}

impl Minimap {

    pub fn world_unload(&mut self) {
        self.menu.world_unload(self.controller.take().and_then(Weak::get));
        if let Some(objects) = self.objects.as_mut() {
            objects.world_unload();
        }
        if let Some(terrain) = self.terrain.as_mut() {
            terrain.world_unload();
        }
        if let Some(widget) = self.live.take().and_then(|live| live.widget.get()) {
            self.ui.remove_from_parent.call(widget, |_| {});
        }
        self.tracked.clear();
        self.assets.world_unload();
        self.shown_view = 0.0;
        self.last_view_step = None;
        self.ui.database = None;
        self.next_attempt = Instant::now();
    }

    fn build(&mut self, player: &Player, region: &'static Region) -> Result<Live, String> {
        let view_units = self.view_units();
        let world = player.world;
        let map_texture = self
            .assets
            .get(world, region.texture)
            .ready()
            .ok_or_else(|| format!("{} could not be loaded", region.texture))?;
        keep_full_resolution(map_texture);
        let arrow_texture = self
            .assets
            .get(world, PLAYER_ARROW)
            .ready()
            .ok_or_else(|| format!("{PLAYER_ARROW} could not be loaded"))?;
        let fog_material = self.assets.get(world, FOG_MATERIAL).ready();
        let settings = &self.settings;
        let ui = &mut self.ui;

        let user_widget_class = ui.user_widget_class;
        let widget = ui.user_widget(user_widget_class, player.world, player.controller)?;
        let tree = unsafe { widget.read_obj(ui.widget_tree_offset) }.ok_or("widget has no WidgetTree")?;

        let root = ue::construct(ui.canvas_class, tree)?;
        unsafe { tree.write(ui.root_widget_offset, root.raw()) };

        let size = settings.size;
        let full = Vec2 { x: size, y: size };
        let frame = ue::construct(ui.canvas_class, tree)?;
        let frame_slot = ui.add(root, frame, Vec2::default(), full, 0)?;
        ui.set_clipping.call(frame, |a| a.set(0, CLIP_TO_BOUNDS));

        let w = self.menu.widgets();
        let hint = w.text(tree, "", HINT_TEXT_SIZE, HINT_COLOR, false)?;
        w.center_text(hint);
        let hint_slot = w.canvas_add(root, hint, Vec2::default(), 1)?;

        if fog_material.is_none() {
            host::warning(&format!("fog of war unavailable: {FOG_MATERIAL} could not be loaded"));
        }
        let layer_size = region.image_size(size, view_units);
        let mut layers = Layers::build(w, world, tree, frame, size, settings.roundness, map_texture, fog_material, layer_size)?;
        let backing_texture = layers::backing_png().and_then(|png| w.import_png(world, &png));
        // Images only hold it while a landmark is drawn, so it is kept like a terrain tile.
        if let Some(texture) = backing_texture {
            layers.hold(w, texture);
        }
        let pin_arrow = layers::pin_arrow_png().and_then(|png| w.import_png(world, &png));
        let pins = (0..PIN_COUNT)
            .map(|_| EdgePin::build(ui, tree, frame, pin_arrow))
            .collect::<Result<Vec<_>, _>>()?;

        let center = Vec2 { x: size / 2.0, y: size / 2.0 };
        let (arrow, arrow_slot) = ui.image(tree, frame, center, Vec2 { x: ARROW_SIZE, y: ARROW_SIZE }, Z_ARROW)?;
        ui.center(arrow_slot);
        ui.set_texture(arrow, arrow_texture);

        ui.add_to_viewport.call(widget, |a| a.set(0, 0i32));
        let opacity = settings.opacity as f32;
        ui.set_render_opacity.call(widget, |a| a.set(0, opacity));
        ui.set_shown(widget, settings.enabled);

        let now = Instant::now();
        let mut live = Live {
            widget: Weak::new(widget),
            tree: Weak::new(tree),
            frame_slot: Weak::new(frame_slot),
            arrow_slot: Weak::new(arrow_slot),
            hint: Weak::new(hint),
            hint_slot: Weak::new(hint_slot),
            layers,
            arrow: Weak::new(arrow),
            map_angle: 0.0,
            icon_angle: 0.0,
            icon_zoom: 1.0,
            landmark_scale: 1.0,
            backing_texture: backing_texture.map(Weak::new),
            pins,
            cone: None,
            view_units,
            region,
            unknown_map: None,
            size,
            pixel: 1.0,
            layer_size,
            layer_position: None,
            arrow_angle: None,
            shown: settings.enabled,
            needs_layout: false,
            needs_cull: true,
            bosses_loaded: false,
            next_boss_try: now,
            boss_spawners: HashMap::new(),
            markers: HashMap::new(),
            moving_shown: Vec::new(),
            sized_classes: HashSet::new(),
            model_kinds: HashSet::new(),
            next_object_scan: now,
            size_reports: Vec::new(),
            next_slow: now,
            next_cull: now,
        };
        place_frame(ui, self.menu.widgets(), &mut live, settings, world);
        Ok(live)
    }
}

/// The game's own icon widget for a category, made and set up like one of its markers on the minimap, so
/// its row in the settings menu shows exactly what the minimap does. None when no marker of it is around.
fn category_widget(ui: &mut Ui, live: Option<&Live>, mut locations: Option<&mut Locations>, player: &Player, category: usize) -> Option<Obj> {
    let kinds = CATEGORIES[category].kinds;
    for view in live?.markers.values().filter(|view| kinds.contains(&view.kind)) {
        match (&view.look, view.key) {
            (Look::GameIcon(class), MarkerKey::Location(location)) => {
                let (Some(class), Some(location)) = (class.get(), location.get()) else { continue };
                let Some(locations) = locations.as_deref_mut() else { continue };
                let Some(point) = locations.point(location) else { continue };
                let id = locations.id(location);
                let widget = ui.user_widget(class, player.world, player.controller).ok()?;
                ui.setup_icon(class, widget, location, point, id);
                ui.refresh_icon(class, widget, point);
                return Some(widget);
            }
            (Look::Boss(boss), _) => {
                let class = ui.boss_widget.as_ref().and_then(|icon| icon.class.get())?;
                let widget = ui.user_widget(class, player.world, player.controller).ok()?;
                let map_name = ui.region_name_at(boss.at.x, boss.at.y);
                let icon = ui.boss_widget.as_mut()?;
                icon.set_boss.call(widget, |a| {
                    a.set(0, boss.character);
                    a.set(1, boss.level);
                    a.set(2, boss.spawner);
                    a.set(3, boss.character);
                    a.set(4, boss.spawner);
                    a.set(5, boss.at);
                    a.set(6, map_name);
                });
                return Some(widget);
            }
            _ => {}
        }
    }
    None
}

/// Item pickup types for the settings menu, by display name: index into `Settings::pickups`, name, icon.
fn pickup_rows(objects: Option<&MapObjects>, settings: &Settings) -> Vec<(usize, String, String)> {
    let Some(objects) = objects else { return Vec::new() };
    let types = objects.pickup_types();
    let mut rows: Vec<(usize, String, String)> = settings
        .pickups
        .iter()
        .enumerate()
        .filter_map(|(index, (item, _))| types.get(item).map(|kind| (index, kind.label.clone(), kind.icon.clone())))
        .collect();
    rows.sort_by(|a, b| a.1.cmp(&b.1));
    rows
}

/// Puts the minimap box in its corner at its size and shape, and the settings hint just outside it.
fn place_frame(ui: &mut Ui, w: &mut Widgets, live: &mut Live, settings: &Settings, world: Obj) {
    let size = settings.size;
    let full = Vec2 { x: size, y: size };
    let (anchor_x, anchor_y) = settings.corner.anchor();
    let anchor = Vec2 { x: anchor_x, y: anchor_y };
    let toward_center = |anchor: f64| if anchor == 0.0 { 1.0 } else { -1.0 };
    if let Some(slot) = live.frame_slot.get() {
        ui.slot_anchors.call(slot, |a| a.set(0, Anchors { min: anchor, max: anchor }));
        ui.slot_alignment.call(slot, |a| a.set(0, anchor));
        let position = Vec2 { x: toward_center(anchor_x) * settings.offset_x, y: toward_center(anchor_y) * settings.offset_y };
        ui.slot_position.call(slot, |a| a.set(0, position));
        ui.slot_size.call(slot, |a| a.set(0, full));
    }
    if let Err(reason) = live.layers.reshape(w, world, size, settings.roundness) {
        host::warning(&format!("minimap shape not changed: {reason}"));
    }
    live.cone = None;
    if let Some(slot) = live.arrow_slot.get() {
        ui.slot_position.call(slot, |a| a.set(0, Vec2 { x: size / 2.0, y: size / 2.0 }));
    }
    live.size = size;
    live.layer_position = None;
    live.needs_layout = true;

    let (Some(hint), Some(slot)) = (live.hint.get(), live.hint_slot.get()) else { return };
    let shown = settings.show_hint && !settings.menu_keys.is_empty();
    w.visibility(hint, if shown { widgets::HIT_TEST_INVISIBLE } else { widgets::COLLAPSED });
    w.set_text(hint, &format!("Press {} to change minimap settings", format_keys(&settings.menu_keys)));
    // Under the minimap in the top corners, above it in the bottom ones, so it stays on screen.
    let below = anchor_y == 0.0;
    w.align(slot, anchor, Vec2 { x: 0.5, y: if below { 0.0 } else { 1.0 } });
    let x = toward_center(anchor_x) * (settings.offset_x + size / 2.0);
    let y = toward_center(anchor_y) * (settings.offset_y + size + HINT_GAP);
    w.place(slot, Vec2 { x, y }, None);
}
