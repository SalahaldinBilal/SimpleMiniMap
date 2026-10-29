use std::collections::HashMap;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use crate::host;
use crate::ue::{self, Function, Name, Obj, Weak};

const TABLE: &str = "/Game/Pal/DataTable/UI/DT_LocationUIData.DT_LocationUIData";
const MAP_BASE_CLASS: &str = "/Game/Pal/Blueprint/UI/UserInterface/Map/WBP_Map_Base.WBP_Map_Base_C";
const MAP_BODY_CLASS: &str = "/Game/Pal/Blueprint/UI/UserInterface/Map/WBP_Map_Body.WBP_Map_Body_C";
const ICON_CANVASES: [&str; 3] = ["Canvas_ForIcon_Mask", "Canvas_ForIcon_NoMask", "Canvas_ForIcon_Priority"];
const WIDGET_FOLDER: &str = "/Game/Pal/Blueprint/UI/UserInterface/Map/";
// Icon classes the map was seen using, loaded up front so they work before the map is first opened.
const KNOWN_ICON_CLASSES: [(u8, &str); 6] = [
    (3, "WBP_Map_IconFTTower"),
    (4, "WBP_Map_IconCamp"),
    (11, "WBP_Map_IconTower"),
    (21, "WBP_Map_IconQuest"),
    (25, "WBP_Map_IconPortal"),
    (26, "WBP_Map_StandAloneBossIcon"),
];
const RETRY_EVERY: Duration = Duration::from_secs(10);

/// What `IconTable::load` and `IconClassFinder::new` load, so it can be loaded in the background first.
pub fn asset_paths() -> Vec<String> {
    let mut paths: Vec<String> = [TABLE, MAP_BASE_CLASS, MAP_BODY_CLASS].map(String::from).into();
    paths.extend(KNOWN_ICON_CLASSES.iter().map(|(_, name)| class_path(name)));
    paths
}

fn class_path(name: &str) -> String {
    format!("{WIDGET_FOLDER}{name}.{name}_C")
}

/// The game's own marker icons, from DT_LocationUIData.
pub struct IconTable {
    by_kind: HashMap<u8, String>,
    by_map_object: HashMap<Name, String>,
    by_relic: HashMap<u8, String>,
}

impl IconTable {
    pub fn load() -> Result<Self, String> {
        let table = ue::load(TABLE)?;
        let row_struct = ue::find("/Script/Pal.PalLocationUIData_TableRow")?;
        let ui_data = ue::find("/Script/Pal.PalLocationUIData")?;
        let icon_offset = ue::property_offset(ui_data, "Icon")?;

        let mut icons = Self {
            by_kind: HashMap::new(),
            by_map_object: HashMap::new(),
            by_relic: HashMap::new(),
        };
        let mut rows = Vec::new();
        let mut pairs = Vec::new();
        ue::data_table_rows(table, &mut rows)?;
        for &row in &rows {
            let value_offset = ue::map_entries(row_struct, "locationUIDataMap", row, &mut pairs)?;
            for &pair in &pairs {
                let kind: u8 = unsafe { ue::read_at(pair, 0) };
                if let Some(path) = soft_path_at(pair, value_offset + icon_offset) {
                    icons.by_kind.entry(kind).or_insert(path);
                }
            }

            let value_offset = ue::map_entries(row_struct, "mapObjectIconMap", row, &mut pairs)?;
            for &pair in &pairs {
                let name: Name = unsafe { ue::read_at(pair, 0) };
                if let Some(path) = soft_path_at(pair, value_offset) {
                    icons.by_map_object.entry(name).or_insert(path);
                }
            }

            let value_offset = ue::map_entries(row_struct, "levelObjectIconMap", row, &mut pairs)?;
            for &pair in &pairs {
                let relic: u8 = unsafe { ue::read_at(pair, 0) };
                if let Some(path) = soft_path_at(pair, value_offset) {
                    icons.by_relic.entry(relic).or_insert(path);
                }
            }
        }

        let mut kinds: Vec<_> = icons.by_kind.keys().copied().collect();
        kinds.sort_unstable();
        let mut map_objects: Vec<_> = icons.by_map_object.keys().map(|name| name.text()).collect();
        map_objects.sort();
        host::info(&format!(
            "icon table: marker types {:?}, map objects [{}], {} relic icons",
            kinds,
            map_objects.join(", "),
            icons.by_relic.len()
        ));
        Ok(icons)
    }

    pub fn for_kind(&self, kind: u8) -> Option<&str> {
        self.by_kind.get(&kind).map(String::as_str)
    }

    pub fn for_map_object(&self, name: Name) -> Option<&str> {
        self.by_map_object.get(&name).map(String::as_str)
    }

    pub fn for_relic(&self, relic: u8) -> Option<&str> {
        self.by_relic.get(&relic).map(String::as_str)
    }
}

fn soft_path_at(pair: *mut c_void, offset: usize) -> Option<String> {
    let path = unsafe { ue::soft_path(pair.cast::<u8>().add(offset)) };
    (!path.is_empty() && path != "None").then_some(path)
}

/// Finds the widget class the full map uses for each marker type. The game fills WBP_Map_Base's
/// IconClassMap at runtime, so this reads the live map widget, and falls back to learning from the
/// icons already placed on its map bodies.
pub struct IconClassFinder {
    map_base_class: Obj,
    body_class: Obj,
    canvas_offsets: Vec<usize>,
    children_count: Function,
    child_at: Function,
    location_base: Function,
    location_type: Function,
    instance: Option<Weak>,
    classes: HashMap<u8, Weak>,
    complete: bool,
    next_try: Instant,
}

impl IconClassFinder {
    pub fn new() -> Result<Self, String> {
        let map_base_class = ue::load(MAP_BASE_CLASS)?;
        let body_class = ue::load(MAP_BODY_CLASS)?;
        let panel = ue::find("/Script/UMG.PanelWidget")?;
        let map_icon = ue::find("/Script/Pal.PalUIWorldMapIcon")?;
        let location = ue::find("/Script/Pal.PalLocationBase")?;
        let mut classes = HashMap::new();
        let mut missing = Vec::new();
        for (kind, name) in KNOWN_ICON_CLASSES {
            match ue::load(&class_path(name)) {
                Ok(class) => {
                    classes.insert(kind, Weak::new(class));
                }
                Err(_) => missing.push(name),
            }
        }
        host::info(&format!(
            "map icon classes preloaded: {} of {}{}",
            classes.len(),
            KNOWN_ICON_CLASSES.len(),
            if missing.is_empty() { String::new() } else { format!(", not found: {}", missing.join(", ")) }
        ));
        Ok(Self {
            canvas_offsets: ICON_CANVASES
                .iter()
                .map(|name| ue::property_offset(body_class, name))
                .collect::<Result<_, _>>()?,
            children_count: Function::resolve(panel, "GetChildrenCount", &[])?,
            child_at: Function::resolve(panel, "GetChildAt", &["Index"])?,
            location_base: Function::resolve(map_icon, "GetLocationBase", &[])?,
            location_type: Function::resolve(location, "GetType", &[])?,
            map_base_class,
            body_class,
            instance: None,
            classes,
            complete: false,
            next_try: Instant::now(),
        })
    }

    pub fn for_kind(&self, kind: u8) -> Option<Obj> {
        self.classes.get(&kind).and_then(|weak| weak.get())
    }

    /// Returns true when new classes were learned.
    pub fn poll(&mut self) -> bool {
        if self.complete || Instant::now() < self.next_try {
            return false;
        }
        self.next_try = Instant::now() + RETRY_EVERY;
        let instance = match self.instance.and_then(Weak::get) {
            Some(instance) => instance,
            None => match ue::find_instance("WBP_Map_Base_C") {
                Some(instance) => {
                    self.instance = Some(Weak::new(instance));
                    instance
                }
                None => return false,
            },
        };

        let before = self.classes.len();
        let mut pairs = Vec::new();
        if let Ok(value_offset) = ue::map_entries(self.map_base_class, "IconClassMap", instance.raw(), &mut pairs) {
            for &pair in &pairs {
                let kind: u8 = unsafe { ue::read_at(pair, 0) };
                if let Some(class) = Obj::from_raw(unsafe { ue::read_at::<*mut c_void>(pair, value_offset) }) {
                    self.classes.insert(kind, Weak::new(class));
                }
            }
            if !pairs.is_empty() {
                self.complete = true;
            }
        }
        if !self.complete {
            self.learn_from_bodies(instance);
        }

        if self.classes.len() == before {
            return false;
        }
        let mut described: Vec<String> = self
            .classes
            .iter()
            .filter_map(|(kind, class)| Some(format!("{kind}={}", class.get()?.name())))
            .collect();
        described.sort();
        host::info(&format!(
            "map icon classes ({}): {}",
            if self.complete { "from the map" } else { "learned from placed icons" },
            described.join(", ")
        ));
        true
    }

    fn learn_from_bodies(&mut self, instance: Obj) {
        let mut bodies = Vec::new();
        let Ok(value_offset) = ue::map_entries(self.map_base_class, "MapBodyMap", instance.raw(), &mut bodies) else {
            return;
        };
        for &pair in &bodies {
            let Some(body) = Obj::from_raw(unsafe { ue::read_at::<*mut c_void>(pair, value_offset) }) else {
                continue;
            };
            if !ue::is_a(body, self.body_class) {
                continue;
            }
            for &offset in &self.canvas_offsets {
                let Some(canvas) = (unsafe { body.read_obj(offset) }) else { continue };
                let count: i32 = self.children_count.call(canvas, |_| {}).returned();
                for index in 0..count {
                    let child: *mut c_void = self.child_at.call(canvas, |a| a.set(0, index)).returned();
                    let Some(child) = Obj::from_raw(child) else { continue };
                    let location: *mut c_void = self.location_base.call(child, |_| {}).returned();
                    let Some(location) = Obj::from_raw(location) else { continue };
                    let kind: u8 = self.location_type.call(location, |_| {}).returned();
                    if let (false, Some(class)) = (self.classes.contains_key(&kind), child.class()) {
                        self.classes.insert(kind, Weak::new(class));
                    }
                }
            }
        }
    }
}
