use std::collections::HashMap;
use std::ffi::c_void;

use crate::actors::{self, CHEST, ITEM_PICKUP, JUNK, ORE, SKILL_FRUIT};
use crate::host;
use crate::icons::IconTable;
use crate::ue::{self, Function, Name, Obj, SoftObjectPtr, Vec3};

const ORES_WITHOUT_MAP_ICON: [&str; 2] = ["DamagableRock0019", "DamagableRock0020"];

/// One map object worth a marker.
pub struct MapObject {
    pub model: Obj,
    pub kind: u8,
    pub icon: String,
    pub at: Vec3,
    /// Static item id, for item pickups.
    pub item: Option<String>,
}

/// An item pickup type, for its row in the settings menu.
#[derive(Clone)]
pub struct PickupType {
    pub label: String,
    pub icon: String,
}

/// What a map object id turns into.
#[derive(Clone)]
enum IdLook {
    Marker(u8, String),
    /// An item lying in the world. Each one says which item it is, so its icon comes from the item.
    Pickup(u8),
    Skip,
}

/// Every map object the host knows about, from the map object manager's model list. Unlike their actors,
/// which the game only keeps spawned near the player, the models cover the whole world.
pub struct MapObjects {
    utility: Obj,
    get_manager: Function,
    get_item_manager: Function,
    item_data: Function,
    manager_class: Obj,
    master_id_offset: usize,
    translation_offset: usize,
    concrete_model_offset: usize,
    opened_offset: usize,
    visual_item_offset: usize,
    item_icon_offset: usize,
    treasure_model_class: Obj,
    pickup_model_class: Obj,
    looks: HashMap<Name, IdLook>,
    items: HashMap<Name, Option<(String, PickupType)>>,
    item_name: Function,
    text_to_string: Function,
    ui_utility: Obj,
    text_library: Obj,
    reported: bool,
}

impl MapObjects {
    pub fn new() -> Result<Self, String> {
        let utility = ue::find("/Script/Pal.Default__PalUtility")?;
        let model_class = ue::find("/Script/Pal.PalMapObjectModel")?;
        let transform = ue::find("/Script/CoreUObject.Transform")?;
        let treasure_model_class = ue::find("/Script/Pal.PalMapObjectTreasureBoxModel")?;
        let pickup_model_class = ue::find("/Script/Pal.PalMapObjectPickupItemOnLevelModel")?;
        let item_manager_class = ue::find("/Script/Pal.PalItemIDManager")?;
        let item_data_class = ue::find("/Script/Pal.PalStaticItemDataBase")?;
        let ui_utility = ue::find("/Script/Pal.Default__PalUIUtility")?;
        let text_library = ue::find("/Script/Engine.Default__KismetTextLibrary")?;
        Ok(Self {
            item_name: Function::resolve(ui_utility, "GetItemName", &["WorldContextObject", "StaticItemId", "outName"])?,
            text_to_string: Function::resolve(text_library, "Conv_TextToString", &["InText"])?,
            ui_utility,
            text_library,
            get_manager: Function::resolve(utility, "GetMapObjectManager", &["WorldContextObject"])?,
            get_item_manager: Function::resolve(utility, "GetItemIDManager", &["WorldContextObject"])?,
            item_data: Function::resolve(item_manager_class, "GetStaticItemData", &["StaticItemId"])?,
            manager_class: ue::find("/Script/Pal.PalMapObjectManager")?,
            master_id_offset: ue::property_offset(model_class, "MapObjectMasterDataId")?,
            translation_offset: ue::property_offset(model_class, "InitialTransformCache")?
                + ue::property_offset(transform, "Translation")?,
            concrete_model_offset: ue::property_offset(model_class, "ConcreteModel")?,
            opened_offset: ue::property_offset(treasure_model_class, "bOpened")?,
            visual_item_offset: ue::property_offset(pickup_model_class, "VisualStaticItemId")?,
            item_icon_offset: ue::property_offset(item_data_class, "IconTexture")?,
            treasure_model_class,
            pickup_model_class,
            utility,
            looks: HashMap::new(),
            items: HashMap::new(),
            reported: false,
        })
    }

    pub fn world_unload(&mut self) {
        self.reported = false;
    }

    fn concrete(&self, model: Obj) -> Option<Obj> {
        unsafe { model.read_obj(self.concrete_model_offset) }
    }

    /// What a map object id becomes, or None while that cannot be told yet (no concrete model loaded).
    fn look(&self, id: Name, model: Obj, icons: &IconTable) -> Option<IdLook> {
        let text = id.text();
        let table_icon = icons.for_map_object(id).map(str::to_owned);
        // Rocks: ore when the game's map marks them, plus the later ores it does not. Plain stone is neither.
        if text.starts_with("DamagableRock") {
            return Some(match table_icon {
                Some(icon) => IdLook::Marker(ORE, icon),
                None if ORES_WITHOUT_MAP_ICON.contains(&text.as_str()) => {
                    actors::kind_icon(ORE).map_or(IdLook::Skip, |icon| IdLook::Marker(ORE, icon))
                }
                None => IdLook::Skip,
            });
        }
        // Loot boxes the game's own map marks, with its icon for each kind.
        if let (true, Some(icon)) = (text.starts_with("TreasureBox"), table_icon) {
            let kind = if text.contains("RequiredLongHold") { JUNK } else { CHEST };
            return Some(IdLook::Marker(kind, icon));
        }
        let concrete = self.concrete(model)?;
        if ue::is_a(concrete, self.pickup_model_class) {
            return Some(IdLook::Pickup(if text.contains("SkillFruit") { SKILL_FRUIT } else { ITEM_PICKUP }));
        }
        Some(IdLook::Skip)
    }

    /// Id, display name and inventory icon of an item, cached per item.
    fn item(&mut self, world: Obj, item: Name) -> Option<(String, PickupType)> {
        if let Some(known) = self.items.get(&item) {
            return known.clone();
        }
        let manager: *mut c_void = self
            .get_item_manager
            .call(self.utility, |a| a.set_obj(0, Some(world)))
            .returned();
        let manager = Obj::from_raw(manager)?;
        let data: *mut c_void = self.item_data.call(manager, |a| a.set(0, item)).returned();
        let icon = Obj::from_raw(data).and_then(|data| {
            let soft: SoftObjectPtr = unsafe { data.read(self.item_icon_offset) };
            let path = unsafe { ue::soft_path(soft.as_ptr()) };
            (!path.is_empty() && path != "None").then_some(path)
        });
        let id = item.text();
        let found = icon.map(|icon| {
            let label = self.item_label(world, item).filter(|label| !label.is_empty()).unwrap_or_else(|| id.clone());
            (id, PickupType { label, icon })
        });
        self.items.insert(item, found.clone());
        found
    }

    /// The item's name as the game shows it, in the game's language.
    fn item_label(&mut self, world: Obj, item: Name) -> Option<String> {
        self.item_name.call(self.ui_utility, |a| {
            a.set_obj(0, Some(world));
            a.set(1, item);
        });
        let text: ue::Text = self.item_name.param(2);
        let string: ue::EngineString = self.text_to_string.call(self.text_library, |a| a.set(0, text)).returned();
        Some(string.text())
    }

    /// Every item pickup type seen so far, by item id.
    pub fn pickup_types(&self) -> HashMap<String, PickupType> {
        self.items.values().flatten().cloned().collect()
    }

    /// Fills `out` with the map objects that get markers. Returns how many models the host has in all.
    pub fn scan(&mut self, world: Obj, icons: &IconTable, out: &mut Vec<MapObject>) -> Result<usize, String> {
        out.clear();
        let manager: *mut c_void = self
            .get_manager
            .call(self.utility, |a| a.set_obj(0, Some(world)))
            .returned();
        let Some(manager) = Obj::from_raw(manager) else { return Ok(0) };
        let mut pairs = Vec::new();
        let value_offset = ue::map_entries(self.manager_class, "MapObjectModelMapForServer", manager.raw(), &mut pairs)?;
        let mut counts: HashMap<Name, usize> = HashMap::new();
        for &pair in &pairs {
            let Some(model) = Obj::from_raw(unsafe { ue::read_at::<*mut c_void>(pair, value_offset) }) else { continue };
            let id: Name = unsafe { model.read(self.master_id_offset) };
            if !self.reported {
                *counts.entry(id).or_default() += 1;
            }
            let look = match self.looks.get(&id) {
                Some(look) => look.clone(),
                None => {
                    let Some(look) = self.look(id, model, icons) else { continue };
                    self.looks.insert(id, look.clone());
                    look
                }
            };
            let (kind, icon, item) = match look {
                IdLook::Marker(kind, icon) => (kind, icon, None),
                IdLook::Pickup(kind) => {
                    let Some(concrete) = self.concrete(model) else { continue };
                    let item: Name = unsafe { concrete.read(self.visual_item_offset) };
                    let Some((id, pickup)) = self.item(world, item) else { continue };
                    // Skill fruit go under their own category; every other pickup is switched per item.
                    let item = (kind == ITEM_PICKUP).then_some(id);
                    (kind, pickup.icon, item)
                }
                IdLook::Skip => continue,
            };
            out.push(MapObject { model, kind, icon, at: unsafe { model.read(self.translation_offset) }, item });
        }
        if !self.reported && !pairs.is_empty() {
            self.reported = true;
            self.report(&counts);
        }
        Ok(pairs.len())
    }

    /// Opened chests and searched junk piles stay in the world; their model says so.
    pub fn opened(&self, model: Obj) -> bool {
        let Some(concrete) = self.concrete(model) else { return false };
        ue::is_a(concrete, self.treasure_model_class) && unsafe { concrete.read::<u8>(self.opened_offset) } != 0
    }

    /// Logs every map object id the host has, with counts and whether it gets a marker, most common first.
    fn report(&self, counts: &HashMap<Name, usize>) {
        let mut ids: Vec<(String, usize, &str)> = counts
            .iter()
            .map(|(id, count)| {
                let marked = match self.looks.get(id) {
                    Some(IdLook::Marker(..)) => " (marked)",
                    Some(IdLook::Pickup(_)) => " (item pickup)",
                    _ => "",
                };
                (id.text(), *count, marked)
            })
            .collect();
        ids.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let listed: Vec<String> = ids.iter().map(|(id, count, marked)| format!("{id} x{count}{marked}")).collect();
        host::info(&format!("map object ids: {}", listed.join(", ")));
    }
}
