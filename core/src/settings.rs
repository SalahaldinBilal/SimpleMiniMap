use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::host;

pub const MODIFIER_SHIFT: u8 = 1;
pub const MODIFIER_CONTROL: u8 = 2;
pub const MODIFIER_ALT: u8 = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyBind {
    pub key: u8,
    pub modifiers: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    /// Anchor and alignment of the minimap box, as fractions of the screen.
    pub fn anchor(self) -> (f64, f64) {
        match self {
            Corner::TopLeft => (0.0, 0.0),
            Corner::TopRight => (1.0, 0.0),
            Corner::BottomLeft => (0.0, 1.0),
            Corner::BottomRight => (1.0, 1.0),
        }
    }

    pub const ALL: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];

    fn name(self) -> &'static str {
        match self {
            Corner::TopLeft => "TopLeft",
            Corner::TopRight => "TopRight",
            Corner::BottomLeft => "BottomLeft",
            Corner::BottomRight => "BottomRight",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "Top left",
            Corner::TopRight => "Top right",
            Corner::BottomLeft => "Bottom left",
            Corner::BottomRight => "Bottom right",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Corner::ALL.into_iter().find(|corner| corner.name().eq_ignore_ascii_case(text))
    }
}

/// When the minimap shows terrain rendered from above instead of the game's map art.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerrainUse {
    Off,
    On,
    Close,
}

impl TerrainUse {
    pub const ALL: [TerrainUse; 3] = [TerrainUse::Off, TerrainUse::On, TerrainUse::Close];

    fn name(self) -> &'static str {
        match self {
            TerrainUse::Off => "Off",
            TerrainUse::On => "On",
            TerrainUse::Close => "Close",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TerrainUse::Off => "Off",
            TerrainUse::On => "On",
            TerrainUse::Close => "When zoomed in close",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        TerrainUse::ALL.into_iter().find(|mode| mode.name().eq_ignore_ascii_case(text))
    }
}

/// Groups of marker categories, each with its own master switch.
pub const GROUPS: [(&str, &str); 5] = [
    ("Map", "Map"),
    ("PalsAndPeople", "Pals and people"),
    ("Items", "Items"),
    ("Resources", "Resources"),
    ("Pickups", "Item pickups"),
];

/// The group whose switches are per item type, found while playing rather than fixed.
pub const PICKUPS_GROUP: usize = 4;

/// Item pickups hidden until switched on: too common to be worth a marker.
const PICKUPS_OFF_BY_DEFAULT: [&str; 8] = ["Wood", "Stone", "Pal_crystal_S", "Berries", "Egg", "Arrow", "Leather", "Wool"];

pub struct Category {
    pub key: &'static str,
    pub label: &'static str,
    pub group: Option<usize>,
    pub kinds: &'static [u8],
}

const fn category(key: &'static str, label: &'static str, group: usize, kinds: &'static [u8]) -> Category {
    Category { key, label, group: Some(group), kinds }
}

/// Marker categories that can be switched off, with the marker kinds they cover:
/// EPalLocationType values for map markers, 200 and up for world actors (see actors.rs).
pub const CATEGORIES: [Category; 29] = [
    category("FastTravel", "Fast travel", 0, &[3]),
    category("Bases", "Bases", 0, &[4]),
    category("Dungeons", "Dungeons", 0, &[9, 20]),
    category("Towers", "Towers", 0, &[11]),
    category("Bosses", "Bosses", 0, &[16, 17, 26]),
    category("Quests", "Quests", 0, &[21, 22]),
    category("Warps", "Warps", 0, &[24, 25]),
    category("OilRigs", "Oil rigs", 0, &[18]),
    category("SupplyDrops", "Supply drops", 0, &[19]),
    category("TreasureMaps", "Treasure maps", 0, &[23]),
    category("DeathDrops", "Death drops", 0, &[12, 13]),
    category("Pins", "Pins", 0, &[15]),
    category("RespawnPoints", "Respawn points", 0, &[10]),
    category("WildPals", "Wild pals", 1, &[200]),
    category("ShinyPals", "Shiny pals", 1, &[214]),
    category("OwnPals", "Own pals", 1, &[202]),
    category("Humans", "Humans", 1, &[201]),
    category("Players", "Players", 1, &[203]),
    category("Chests", "Chests", 2, &[204]),
    category("Eggs", "Eggs", 2, &[205]),
    category("Effigies", "Effigies", 2, &[206]),
    category("Notes", "Notes", 2, &[207]),
    category("Junk", "Junk", 2, &[212]),
    category("Ore", "Ore", 3, &[209]),
    category("SkillFruitTrees", "Skill fruit trees", 3, &[208]),
    category("Lotuses", "Lotuses", 3, &[210]),
    category("Forage", "Forage", 3, &[211]),
    category("FishingSpots", "Fishing spots", 3, &[213]),
    Category { key: "Other", label: "Other", group: None, kinds: &[] },
];

const CATEGORIES_OFF_BY_DEFAULT: [&str; 2] = ["RespawnPoints", "OwnPals"];
pub const DEFAULT_SIZE: f64 = 300.0;
pub const DEFAULT_OFFSET: f64 = 40.0;
pub const SIZE_RANGE: (f64, f64) = (100.0, 1200.0);
pub const ZOOM_RANGE: (f64, f64) = (60.0, 1600.0);
pub const OPACITY_RANGE: (f64, f64) = (0.1, 1.0);
pub const ICON_SCALE_RANGE: (f64, f64) = (0.3, 3.0);
pub const ROUNDNESS_RANGE: (f64, f64) = (0.0, 100.0);
pub const MEGAZOOM_RANGE: (f64, f64) = (400.0, 4000.0);
pub const ICON_ZOOM_RANGE: (f64, f64) = (0.1, 1.0);
pub const LANDMARK_SCALE_RANGE: (f64, f64) = (0.5, 3.0);

/// Marker kinds that are places to find again (fast travel, bases, dungeons, towers, bosses, quests, warps).
/// They keep their size when zoomed out, sit above other markers, and can get a backing.
pub const LANDMARK_KINDS: [u8; 12] = [3, 4, 9, 11, 16, 17, 20, 21, 22, 24, 25, 26];

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub corner: Corner,
    pub offset_x: f64,
    pub offset_y: f64,
    pub size: f64,
    pub opacity: f64,
    pub zoom_metres: f64,
    /// The zoom inside dungeons, kept apart from the overworld's.
    pub dungeon_zoom_metres: f64,
    pub icon_scale: f64,
    /// Icons shrink as the view widens and grow as it narrows, by `icon_zoom_strength` (1 follows the map fully).
    pub icon_zoom: bool,
    pub icon_zoom_strength: f64,
    /// Size of landmark icons on top of `icon_scale`.
    pub landmark_scale: f64,
    pub landmark_backing: bool,
    /// The nearest unlocked teleporters, warps, bases and quests off the minimap, pinned to its edge.
    pub edge_pins: bool,
    /// The game's clouds over unexplored areas, as on its own map.
    pub fog_of_war: bool,
    pub markers_in_fog: bool,
    pub show_hint: bool,
    /// How round the corners are: 0 is square, 100 a circle.
    pub roundness: f64,
    pub rotate_with_camera: bool,
    pub view_cone: bool,
    pub auto_zoom: bool,
    pub megazoom_metres: f64,
    pub hide_in_base: bool,
    pub overworld_terrain: TerrainUse,
    pub dungeon_terrain: TerrainUse,
    pub toggle_keys: Vec<KeyBind>,
    pub zoom_in_keys: Vec<KeyBind>,
    pub zoom_out_keys: Vec<KeyBind>,
    pub menu_keys: Vec<KeyBind>,
    pub megazoom_keys: Vec<KeyBind>,
    pub categories: Vec<bool>,
    pub groups: Vec<bool>,
    /// Item pickups by static item id, in the order they were first seen.
    pub pickups: Vec<(String, bool)>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            corner: Corner::TopLeft,
            offset_x: DEFAULT_OFFSET,
            offset_y: DEFAULT_OFFSET,
            size: DEFAULT_SIZE,
            opacity: 1.0,
            zoom_metres: 220.0,
            dungeon_zoom_metres: 120.0,
            icon_scale: 1.0,
            icon_zoom: true,
            icon_zoom_strength: 0.5,
            landmark_scale: 1.3,
            landmark_backing: true,
            edge_pins: true,
            fog_of_war: true,
            markers_in_fog: false,
            show_hint: true,
            roundness: 10.0,
            rotate_with_camera: false,
            view_cone: true,
            auto_zoom: false,
            megazoom_metres: 1500.0,
            hide_in_base: true,
            overworld_terrain: TerrainUse::On,
            dungeon_terrain: TerrainUse::On,
            toggle_keys: parse_keys("Alt+N"),
            zoom_in_keys: parse_keys("Alt+Equals, Alt+NumpadPlus"),
            zoom_out_keys: parse_keys("Alt+Minus, Alt+NumpadMinus"),
            menu_keys: parse_keys("Alt+J"),
            megazoom_keys: parse_keys("Alt+K"),
            categories: CATEGORIES
                .iter()
                .map(|category| !CATEGORIES_OFF_BY_DEFAULT.contains(&category.key))
                .collect(),
            groups: vec![true; GROUPS.len()],
            pickups: Vec::new(),
        }
    }
}

impl Settings {
    pub fn shows_kind(&self, kind: u8) -> bool {
        let index = CATEGORIES
            .iter()
            .position(|category| category.kinds.contains(&kind))
            .unwrap_or(CATEGORIES.len() - 1);
        self.categories[index] && CATEGORIES[index].group.is_none_or(|group| self.groups[group])
    }

    pub fn shows_pickup(&self, item: &str) -> bool {
        self.groups[PICKUPS_GROUP]
            && self
                .pickups
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(item))
                .is_none_or(|(_, shown)| *shown)
    }

    /// Adds an item pickup type the first time it is seen. Returns true when it was new. Item ids are matched
    /// without regard to case, as the game spells some differently in different places.
    pub fn learn_pickup(&mut self, item: &str) -> bool {
        if self.pickups.iter().any(|(known, _)| known.eq_ignore_ascii_case(item)) {
            return false;
        }
        let off = PICKUPS_OFF_BY_DEFAULT.iter().any(|known| known.eq_ignore_ascii_case(item));
        self.pickups.push((item.to_owned(), !off));
        true
    }

    /// Every key some binding uses, for the keyboard poll.
    pub fn bound_keys(&self) -> impl Iterator<Item = u8> + '_ {
        [&self.toggle_keys, &self.zoom_in_keys, &self.zoom_out_keys, &self.menu_keys, &self.megazoom_keys]
            .into_iter()
            .flatten()
            .map(|bind| bind.key)
    }

    pub fn load(path: &Path) -> Self {
        let mut settings = Self::default();
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => {
                host::info(&format!("no settings file yet, writing defaults to {}", path.display()));
                settings.save(path);
                return settings;
            }
        };

        let mut section = String::new();
        let mut seen = Vec::new();
        let mut only_shiny = false;
        for (number, raw) in text.lines().enumerate() {
            let Some((line_section, key, value)) = parse_line(raw, &mut section) else { continue };
            // Replaced by the ShinyPals category: "only shiny" is wild pals off, shiny pals on.
            if line_section == "markers" && key.eq_ignore_ascii_case("OnlyShinyWildPals") {
                only_shiny = parse_bool(value) == Some(true);
                continue;
            }
            if settings.apply(&line_section, key, value) {
                seen.push(format!("{line_section}.{}", key.to_ascii_lowercase()));
            } else {
                host::warning(&format!("settings line {} ignored: {raw}", number + 1));
            }
        }
        if only_shiny && let Some(index) = CATEGORIES.iter().position(|category| category.key == "WildPals") {
            settings.categories[index] = false;
        }
        if settings.missing_keys(&seen) {
            host::info("adding new options to the settings file");
            settings.save(path);
        }
        settings
    }

    fn missing_keys(&self, seen: &[String]) -> bool {
        let mut section = String::new();
        self.to_text()
            .lines()
            .filter_map(|raw| parse_line(raw, &mut section))
            .any(|(section, key, _)| !seen.contains(&format!("{section}.{}", key.to_ascii_lowercase())))
    }

    fn apply(&mut self, section: &str, key: &str, value: &str) -> bool {
        // Item ids keep their spelling, as they are written back to the file and shown in the menu.
        if section == "pickups" {
            let Some(shown) = parse_bool(value) else { return false };
            self.learn_pickup(key);
            if let Some(entry) = self.pickups.iter_mut().find(|(known, _)| known.eq_ignore_ascii_case(key)) {
                entry.1 = shown;
            }
            return true;
        }
        let key = key.to_ascii_lowercase();
        match (section, key.as_str()) {
            ("minimap", "enabled") => parse_bool(value).map(|v| self.enabled = v).is_some(),
            ("minimap", "showsettingshint") => parse_bool(value).map(|v| self.show_hint = v).is_some(),
            ("minimap", "corner") => Corner::parse(value).map(|v| self.corner = v).is_some(),
            ("minimap", "offsetx") => parse_number(value, 0.0, 4000.0).map(|v| self.offset_x = v).is_some(),
            ("minimap", "offsety") => parse_number(value, 0.0, 4000.0).map(|v| self.offset_y = v).is_some(),
            ("minimap", "size") => parse_number(value, SIZE_RANGE.0, SIZE_RANGE.1).map(|v| self.size = v).is_some(),
            ("minimap", "opacity") => parse_number(value, OPACITY_RANGE.0, OPACITY_RANGE.1).map(|v| self.opacity = v).is_some(),
            ("minimap", "zoom") => parse_number(value, ZOOM_RANGE.0, ZOOM_RANGE.1).map(|v| self.zoom_metres = v).is_some(),
            ("minimap", "dungeonzoom") => {
                parse_number(value, ZOOM_RANGE.0, ZOOM_RANGE.1).map(|v| self.dungeon_zoom_metres = v).is_some()
            }
            ("minimap", "iconscale") => {
                parse_number(value, ICON_SCALE_RANGE.0, ICON_SCALE_RANGE.1).map(|v| self.icon_scale = v).is_some()
            }
            ("minimap", "scaleiconswithzoom") => parse_bool(value).map(|v| self.icon_zoom = v).is_some(),
            ("minimap", "iconzoomstrength") => {
                parse_number(value, ICON_ZOOM_RANGE.0, ICON_ZOOM_RANGE.1).map(|v| self.icon_zoom_strength = v).is_some()
            }
            ("minimap", "landmarkscale") => {
                parse_number(value, LANDMARK_SCALE_RANGE.0, LANDMARK_SCALE_RANGE.1).map(|v| self.landmark_scale = v).is_some()
            }
            ("minimap", "landmarkbacking") => parse_bool(value).map(|v| self.landmark_backing = v).is_some(),
            ("minimap", "landmarkedgepins") => parse_bool(value).map(|v| self.edge_pins = v).is_some(),
            ("minimap", "fogofwar") => parse_bool(value).map(|v| self.fog_of_war = v).is_some(),
            ("minimap", "markersinfog") => parse_bool(value).map(|v| self.markers_in_fog = v).is_some(),
            ("minimap", "roundness") => {
                parse_number(value, ROUNDNESS_RANGE.0, ROUNDNESS_RANGE.1).map(|v| self.roundness = v).is_some()
            }
            ("minimap", "rotatewithcamera") => parse_bool(value).map(|v| self.rotate_with_camera = v).is_some(),
            ("minimap", "viewcone") => parse_bool(value).map(|v| self.view_cone = v).is_some(),
            ("minimap", "autozoom") => parse_bool(value).map(|v| self.auto_zoom = v).is_some(),
            ("minimap", "megazoom") => {
                parse_number(value, MEGAZOOM_RANGE.0, MEGAZOOM_RANGE.1).map(|v| self.megazoom_metres = v).is_some()
            }
            ("minimap", "hideinbase") => parse_bool(value).map(|v| self.hide_in_base = v).is_some(),
            ("minimap", "overworldterrain") => TerrainUse::parse(value).map(|v| self.overworld_terrain = v).is_some(),
            ("minimap", "dungeonterrain") => TerrainUse::parse(value).map(|v| self.dungeon_terrain = v).is_some(),
            ("keys", "megazoom") => set_keys(&mut self.megazoom_keys, value),
            ("keys", "toggle") => set_keys(&mut self.toggle_keys, value),
            ("keys", "zoomin") => set_keys(&mut self.zoom_in_keys, value),
            ("keys", "zoomout") => set_keys(&mut self.zoom_out_keys, value),
            ("keys", "menu") => set_keys(&mut self.menu_keys, value),
            ("markergroups", name) => match GROUPS.iter().position(|(known, _)| known.eq_ignore_ascii_case(name)) {
                Some(index) => parse_bool(value).map(|v| self.groups[index] = v).is_some(),
                None => false,
            },
            ("markers", name) => match CATEGORIES.iter().position(|category| category.key.eq_ignore_ascii_case(name)) {
                Some(index) => parse_bool(value).map(|v| self.categories[index] = v).is_some(),
                None => false,
            },
            _ => false,
        }
    }

    pub fn save(&self, path: &Path) {
        if let Err(error) = std::fs::write(path, self.to_text()) {
            host::warning(&format!("could not save settings to {}: {error}", path.display()));
        }
    }

    fn to_text(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "; SimpleMiniMap settings. Changes made in the game's settings menu are saved here automatically.");
        let _ = writeln!(text);
        let _ = writeln!(text, "[Minimap]");
        let _ = writeln!(text, "Enabled = {}", self.enabled);
        let _ = writeln!(text, "; the line under the minimap that names the settings key");
        let _ = writeln!(text, "ShowSettingsHint = {}", self.show_hint);
        let _ = writeln!(text, "; TopLeft, TopRight, BottomLeft or BottomRight");
        let _ = writeln!(text, "Corner = {}", self.corner.name());
        let _ = writeln!(text, "; distance from the corner, in UI units");
        let _ = writeln!(text, "OffsetX = {}", self.offset_x);
        let _ = writeln!(text, "OffsetY = {}", self.offset_y);
        let _ = writeln!(text, "; width and height of the minimap, in UI units");
        let _ = writeln!(text, "Size = {}", self.size);
        let _ = writeln!(text, "; 0.1 to 1.0");
        let _ = writeln!(text, "Opacity = {}", self.opacity);
        let _ = writeln!(text, "; metres of world shown across the minimap, 60 to 1600");
        let _ = writeln!(text, "Zoom = {:.0}", self.zoom_metres);
        let _ = writeln!(text, "; the same inside dungeons, boss rooms and arenas");
        let _ = writeln!(text, "DungeonZoom = {:.0}", self.dungeon_zoom_metres);
        let _ = writeln!(text, "IconScale = {}", self.icon_scale);
        let _ = writeln!(text, "; icons get smaller as you zoom out and bigger as you zoom in");
        let _ = writeln!(text, "ScaleIconsWithZoom = {}", self.icon_zoom);
        let _ = writeln!(text, "; how strongly, 0.1 to 1.0 (1.0 shrinks them as much as the map)");
        let _ = writeln!(text, "IconZoomStrength = {}", self.icon_zoom_strength);
        let _ = writeln!(text, "; landmarks are fast travel points, bases, dungeons, towers, bosses, quests and warps");
        let _ = writeln!(text, "; they never shrink when zoomed out. Their size, 0.5 to 3.0, on top of IconScale");
        let _ = writeln!(text, "LandmarkScale = {}", self.landmark_scale);
        let _ = writeln!(text, "; a dark round backing behind landmark icons");
        let _ = writeln!(text, "LandmarkBacking = {}", self.landmark_backing);
        let _ = writeln!(text, "; the nearest unlocked fast travel points, warps, bases and quests off the minimap, shown at its edge");
        let _ = writeln!(text, "LandmarkEdgePins = {}", self.edge_pins);
        let _ = writeln!(text, "; the game's clouds over areas you have not explored yet");
        let _ = writeln!(text, "FogOfWar = {}", self.fog_of_war);
        let _ = writeln!(text, "; true shows markers in areas you have not explored yet");
        let _ = writeln!(text, "MarkersInFog = {}", self.markers_in_fog);
        let _ = writeln!(text, "; 0 is square, 100 is a circle");
        let _ = writeln!(text, "Roundness = {}", self.roundness);
        let _ = writeln!(text, "RotateWithCamera = {}", self.rotate_with_camera);
        let _ = writeln!(text, "ViewCone = {}", self.view_cone);
        let _ = writeln!(text, "; zooms out a step or two while moving fast");
        let _ = writeln!(text, "AutoZoom = {}", self.auto_zoom);
        let _ = writeln!(text, "; metres across while the megazoom key is on, 400 to 4000");
        let _ = writeln!(text, "Megazoom = {:.0}", self.megazoom_metres);
        let _ = writeln!(text, "; hides the minimap while you are inside one of your bases");
        let _ = writeln!(text, "HideInBase = {}", self.hide_in_base);
        let _ = writeln!(text, "; terrain rendered from above instead of the game's map art: Off, On, or Close (zoomed in to 400 m or closer)");
        let _ = writeln!(text, "OverworldTerrain = {}", self.overworld_terrain.name());
        let _ = writeln!(text, "; the same for dungeons, boss rooms and arenas");
        let _ = writeln!(text, "DungeonTerrain = {}", self.dungeon_terrain.name());
        let _ = writeln!(text);
        let _ = writeln!(text, "[Keys]");
        let _ = writeln!(text, "; Combine Alt, Ctrl and Shift with a key, separate alternatives with commas.");
        let _ = writeln!(text, "; Keys: A-Z, 0-9, F1-F12, Home, End, Insert, Delete, PageUp, PageDown, Equals, Minus,");
        let _ = writeln!(text, "; NumpadPlus, NumpadMinus, Numpad0-Numpad9, Up, Down, Left, Right");
        let _ = writeln!(text, "Toggle = {}", format_keys(&self.toggle_keys));
        let _ = writeln!(text, "ZoomIn = {}", format_keys(&self.zoom_in_keys));
        let _ = writeln!(text, "ZoomOut = {}", format_keys(&self.zoom_out_keys));
        let _ = writeln!(text, "Menu = {}", format_keys(&self.menu_keys));
        let _ = writeln!(text, "Megazoom = {}", format_keys(&self.megazoom_keys));
        let _ = writeln!(text);
        let _ = writeln!(text, "[MarkerGroups]");
        let _ = writeln!(text, "; false hides every marker in the group, whatever [Markers] says");
        for ((key, _), shown) in GROUPS.iter().zip(&self.groups) {
            let _ = writeln!(text, "{key} = {shown}");
        }
        let _ = writeln!(text);
        let _ = writeln!(text, "[Markers]");
        for (category, shown) in CATEGORIES.iter().zip(&self.categories) {
            let _ = writeln!(text, "{} = {shown}", category.key);
        }
        let _ = writeln!(text);
        let _ = writeln!(text, "[Pickups]");
        let _ = writeln!(text, "; items lying in the world, by item id, added as they are found");
        for (item, shown) in &self.pickups {
            let _ = writeln!(text, "{item} = {shown}");
        }
        text
    }
}

/// Section, key and value of a settings line, tracking the current section.
fn parse_line<'a>(raw: &'a str, section: &mut String) -> Option<(String, &'a str, &'a str)> {
    let line = raw.split([';', '#']).next().unwrap_or("").trim();
    if line.is_empty() {
        return None;
    }
    if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
        *section = name.trim().to_ascii_lowercase();
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((section.clone(), key.trim(), value.trim()))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_number(value: &str, min: f64, max: f64) -> Option<f64> {
    value.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(min, max))
}

fn set_keys(target: &mut Vec<KeyBind>, value: &str) -> bool {
    let keys = parse_keys(value);
    let all_valid = keys.len() == value.split(',').filter(|part| !part.trim().is_empty()).count();
    *target = keys;
    all_valid
}

fn parse_keys(value: &str) -> Vec<KeyBind> {
    value.split(',').filter_map(|part| parse_key(part.trim())).collect()
}

fn parse_key(text: &str) -> Option<KeyBind> {
    let mut modifiers = 0;
    let mut key = None;
    for part in text.split('+').map(str::trim).filter(|part| !part.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "alt" => modifiers |= MODIFIER_ALT,
            "ctrl" | "control" => modifiers |= MODIFIER_CONTROL,
            "shift" => modifiers |= MODIFIER_SHIFT,
            name => key = Some(key_code(name)?),
        }
    }
    key.map(|key| KeyBind { key, modifiers })
}

const NAMED_KEYS: [(&str, u8); 18] = [
    ("Home", 0x24),
    ("End", 0x23),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("Equals", 0xBB),
    ("Minus", 0xBD),
    ("NumpadPlus", 0x6B),
    ("NumpadMinus", 0x6D),
    ("Up", 0x26),
    ("Down", 0x28),
    ("Left", 0x25),
    ("Right", 0x27),
    ("Comma", 0xBC),
    ("Period", 0xBE),
    ("Tab", 0x09),
    ("Space", 0x20),
];

fn key_code(name: &str) -> Option<u8> {
    if let Some(&(_, code)) = NAMED_KEYS.iter().find(|(known, _)| known.eq_ignore_ascii_case(name)) {
        return Some(code);
    }
    let bytes = name.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        return Some(bytes[0].to_ascii_uppercase());
    }
    if let Some(number) = name.strip_prefix("numpad").and_then(|n| n.parse::<u8>().ok()).filter(|n| *n <= 9) {
        return Some(0x60 + number);
    }
    if let Some(number) = name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n)) {
        return Some(0x6F + number);
    }
    None
}

fn key_name(code: u8) -> String {
    if let Some((name, _)) = NAMED_KEYS.iter().find(|(_, known)| *known == code) {
        return (*name).to_string();
    }
    match code {
        b'A'..=b'Z' | b'0'..=b'9' => (code as char).to_string(),
        0x60..=0x69 => format!("Numpad{}", code - 0x60),
        0x70..=0x7B => format!("F{}", code - 0x6F),
        _ => format!("Key{code}"),
    }
}

/// Every key a binding can use, for capturing a new binding from the keyboard.
pub fn bindable_keys() -> Vec<u8> {
    let mut keys: Vec<u8> = NAMED_KEYS.iter().map(|(_, code)| *code).collect();
    keys.extend(b'A'..=b'Z');
    keys.extend(b'0'..=b'9');
    keys.extend(0x60..=0x69);
    keys.extend(0x70..=0x7B);
    keys
}

pub fn format_keys(keys: &[KeyBind]) -> String {
    keys.iter()
        .map(|bind| {
            let mut parts = Vec::new();
            if bind.modifiers & MODIFIER_CONTROL != 0 {
                parts.push("Ctrl".to_string());
            }
            if bind.modifiers & MODIFIER_ALT != 0 {
                parts.push("Alt".to_string());
            }
            if bind.modifiers & MODIFIER_SHIFT != 0 {
                parts.push("Shift".to_string());
            }
            parts.push(key_name(bind.key));
            parts.join("+")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn mod_folder() -> Option<PathBuf> {
    module_path().and_then(|dll| Some(dll.parent()?.parent()?.to_path_buf()))
}

pub fn path() -> Option<PathBuf> {
    mod_folder().map(|folder| folder.join("settings.ini"))
}

pub fn cache_folder() -> Option<PathBuf> {
    mod_folder().map(|folder| folder.join("cache"))
}

fn module_path() -> Option<PathBuf> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut std::ffi::c_void) -> i32;
        fn GetModuleFileNameW(module: *mut std::ffi::c_void, buffer: *mut u16, size: u32) -> u32;
    }
    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;

    let mut module = std::ptr::null_mut();
    let anchor = module_path as *const u16;
    if unsafe { GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, anchor, &mut module) } == 0 {
        return None;
    }
    let mut buffer = vec![0u16; 1024];
    let len = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len >= buffer.len() {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}
