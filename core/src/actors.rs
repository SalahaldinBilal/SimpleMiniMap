use std::collections::HashMap;
use std::ffi::c_void;

use crate::icons::IconTable;
use crate::ue::{self, Function, Name, Obj, Vec3, Weak};

// Marker kinds for world actors, kept clear of EPalLocationType values.
pub const WILD_PAL: u8 = 200;
pub const HUMAN: u8 = 201;
pub const OWN_PAL: u8 = 202;
pub const PLAYER: u8 = 203;
pub const CHEST: u8 = 204;
pub const EGG: u8 = 205;
pub const EFFIGY: u8 = 206;
pub const NOTE: u8 = 207;
pub const SKILL_FRUIT: u8 = 208;
pub const ORE: u8 = 209;
pub const LOTUS: u8 = 210;
pub const FORAGE: u8 = 211;
pub const JUNK: u8 = 212;
pub const FISHING: u8 = 213;
pub const SHINY_PAL: u8 = 214;
pub const ITEM_PICKUP: u8 = 215;

const ITEM_ICONS: &str = "/Game/Others/InventoryItemIcon/Texture/";
const UI_ICONS: &str = "/Game/Pal/Texture/UI/InGame/";

// Resource spawners by class name, first match wins. Plain stone, wood and the like match nothing.
// Ore is not here: the rocks themselves are tracked, see ROCK_KINDS.
const SPAWNER_KINDS: [(&str, Option<u8>); 5] = [
    ("Lotus", Some(LOTUS)),
    ("Mushroom", Some(FORAGE)),
    ("Berry", Some(FORAGE)),
    ("Poppy", Some(FORAGE)),
    ("AffectionFruit", Some(FORAGE)),
];

// Spawned rocks. The game's map icon table marks the main ores (DamagableRock0001 to 0006); the
// later ores below are missing from it, and plain stone (0010 and up) is in neither.
const ROCK_KINDS: [(&str, Option<u8>); 1] = [("DamagableRock", Some(ORE))];
const ORES_WITHOUT_MAP_ICON: [&str; 2] = ["DamagableRock0019", "DamagableRock0020"];

// Junk piles and the fruit hanging on skill fruit trees are treasure boxes too.
const TREASURE_KINDS: [(&str, Option<u8>); 2] = [("Junk", Some(JUNK)), ("SkillFruits", None)];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Base {
    Character,
    Chest,
    Egg,
    Effigy,
    Note,
    Resource,
    FruitTree,
    FishingSpot,
    Rock,
}

impl Base {
    pub fn moves(self) -> bool {
        self == Base::Character
    }
}

/// What an actor looks like on the minimap.
pub enum ActorLook {
    Texture(String),
    Character { id: Name, shiny: bool },
    Hidden,
    NotYet,
}

pub struct Described {
    pub kind: u8,
    pub look: ActorLook,
}

/// Native classes whose actors become markers. Blueprint subclasses inherit from these.
pub struct ActorClasses {
    player_character: Obj,
    bases: [(Obj, Base); 9],
}

impl ActorClasses {
    pub fn resolve() -> Result<Self, String> {
        Ok(Self {
            player_character: ue::find("/Script/Pal.PalPlayerCharacter")?,
            bases: [
                (ue::find("/Script/Pal.PalCharacter")?, Base::Character),
                (ue::find("/Script/Pal.PalMapObjectTreasureBox")?, Base::Chest),
                (ue::find("/Script/Pal.PalMapObjectPalEgg")?, Base::Egg),
                (ue::find("/Script/Pal.PalLevelObjectRelic")?, Base::Effigy),
                (ue::find("/Script/Pal.PalLevelObjectNote")?, Base::Note),
                (ue::find("/Script/Pal.PalMapObjectSpawnerSimple")?, Base::Resource),
                (ue::find("/Script/Pal.PalMapObjectSpawnerMultiItem")?, Base::FruitTree),
                (ue::find("/Script/Pal.PalFishingSpotArea")?, Base::FishingSpot),
                // Last: treasure boxes, eggs and buildings are map objects too.
                (ue::find("/Script/Pal.PalMapObject")?, Base::Rock),
            ],
        })
    }

    /// Raw class pointers for the spawn filter, which runs outside the core lock.
    pub fn filter(&self) -> Vec<usize> {
        self.bases.iter().map(|(class, _)| class.raw() as usize).collect()
    }

    pub fn base(&self, actor: Obj) -> Option<Base> {
        self.bases
            .iter()
            .find(|(class, _)| ue::is_a(actor, *class))
            .map(|(_, base)| *base)
    }
}

/// Answers questions about world actors through the game's own helper functions.
pub struct ActorInspector {
    classes: ActorClasses,
    utility: Obj,
    location: Function,
    is_pal_monster: Function,
    is_otomo: Function,
    is_players_otomo: Function,
    is_base_camp_pal: Function,
    is_dead: Function,
    parameter_component: Function,
    individual: Function,
    character_id: Function,
    is_rare: Function,
    relic_type: Function,
    fish_gone: Function,
    picked_offset: usize,
    controller_offset: usize,
    model_offset: usize,
    concrete_model_offset: usize,
    opened_offset: usize,
    master_id_offset: usize,
    pool_spawned_offset: usize,
    pool_revision_offset: usize,
    treasure_model_class: Obj,
    weak_point_ore: Obj,
    ores_without_map_icon: Vec<Name>,
    kinds_by_class: HashMap<Weak, Option<u8>>,
}

impl ActorInspector {
    pub fn new() -> Result<Self, String> {
        let utility = ue::find("/Script/Pal.Default__PalUtility")?;
        let actor = ue::find("/Script/Engine.Actor")?;
        let character = ue::find("/Script/Pal.PalCharacter")?;
        let component = ue::find("/Script/Pal.PalCharacterParameterComponent")?;
        let individual = ue::find("/Script/Pal.PalIndividualCharacterParameter")?;
        let relic = ue::find("/Script/Pal.PalLevelObjectRelic")?;
        let obtainable = ue::find("/Script/Pal.PalLevelObjectObtainable")?;
        let fishing_spot = ue::find("/Script/Pal.PalFishingSpotArea")?;
        let map_object = ue::find("/Script/Pal.PalMapObject")?;
        let map_object_model = ue::find("/Script/Pal.PalMapObjectModel")?;
        let treasure_model_class = ue::find("/Script/Pal.PalMapObjectTreasureBoxModel")?;
        let pool_state = ue::find("/Script/Pal.PalMapObjectPoolSpawnState")?;
        let pool_offset = ue::property_offset(map_object, "PoolSpawnState")?;
        Ok(Self {
            weak_point_ore: ue::find("/Script/Pal.PalMapObjectWeakPointOre")?,
            ores_without_map_icon: ORES_WITHOUT_MAP_ICON.iter().map(|id| Name::new(id)).collect(),
            master_id_offset: ue::property_offset(map_object_model, "MapObjectMasterDataId")?,
            pool_spawned_offset: pool_offset + ue::property_offset(pool_state, "bSpawnedInWorld")?,
            pool_revision_offset: pool_offset + ue::property_offset(pool_state, "Revision")?,
            classes: ActorClasses::resolve()?,
            location: Function::resolve(actor, "K2_GetActorLocation", &[])?,
            is_pal_monster: Function::resolve(utility, "IsPalMonster", &["Actor"])?,
            is_otomo: Function::resolve(utility, "IsOtomo", &["Actor"])?,
            is_players_otomo: Function::resolve(utility, "IsPlayersOtomo", &["Actor"])?,
            is_base_camp_pal: Function::resolve(utility, "IsBaseCampPal", &["Actor"])?,
            is_dead: Function::resolve(utility, "IsDead", &["Actor"])?,
            parameter_component: Function::resolve(character, "GetCharacterParameterComponent", &[])?,
            individual: Function::resolve(component, "GetIndividualParameter", &[])?,
            character_id: Function::resolve(individual, "GetCharacterID", &[])?,
            is_rare: Function::resolve(individual, "IsRarePal", &[])?,
            relic_type: Function::resolve(relic, "GetRelicType", &[])?,
            fish_gone: Function::resolve(fishing_spot, "IsEmptyFishShadow", &[])?,
            picked_offset: ue::property_offset(obtainable, "bPickedInClient")?,
            controller_offset: ue::property_offset(ue::find("/Script/Engine.Pawn")?, "Controller")?,
            model_offset: ue::property_offset(map_object, "MapObjectModel")?,
            concrete_model_offset: ue::property_offset(map_object_model, "ConcreteModel")?,
            opened_offset: ue::property_offset(treasure_model_class, "bOpened")?,
            treasure_model_class,
            kinds_by_class: HashMap::new(),
            utility,
        })
    }

    /// Whether an actor of this base is worth tracking at all. Plain stone or wood spawners and
    /// the fruit hanging on skill fruit trees are not.
    pub fn wanted(&mut self, actor: Obj, base: Base) -> bool {
        match base {
            Base::Resource => self.kind_by_class(actor, &SPAWNER_KINDS, None).is_some(),
            Base::Chest => self.kind_by_class(actor, &TREASURE_KINDS, Some(CHEST)).is_some(),
            Base::Rock => ue::is_a(actor, self.weak_point_ore) || self.kind_by_class(actor, &ROCK_KINDS, None).is_some(),
            _ => true,
        }
    }

    /// Mined rocks are not destroyed: the game parks them in a pool, without a model, for reuse.
    fn rock_in_world(&self, actor: Obj) -> bool {
        let spawned: u8 = unsafe { actor.read(self.pool_spawned_offset) };
        spawned != 0 && self.map_object_id(actor).is_some()
    }

    /// Changes whenever the pool hands this actor out again, possibly somewhere else.
    pub fn pool_revision(&self, actor: Obj) -> u32 {
        unsafe { actor.read(self.pool_revision_offset) }
    }

    fn map_object_id(&self, actor: Obj) -> Option<Name> {
        let model = unsafe { actor.read_obj(self.model_offset) }?;
        Some(unsafe { model.read(self.master_id_offset) })
    }

    /// The marker kind for an actor whose class name says what it is, cached per class.
    fn kind_by_class(&mut self, actor: Obj, rules: &[(&str, Option<u8>)], otherwise: Option<u8>) -> Option<u8> {
        let class = actor.class()?;
        *self.kinds_by_class.entry(Weak::new(class)).or_insert_with(|| {
            let name = class.name();
            rules
                .iter()
                .find(|(part, _)| name.contains(part))
                .map_or(otherwise, |(_, kind)| *kind)
        })
    }

    fn opened(&self, actor: Obj) -> bool {
        let Some(model) = (unsafe { actor.read_obj(self.model_offset) }) else { return false };
        let Some(concrete) = (unsafe { model.read_obj(self.concrete_model_offset) }) else { return false };
        ue::is_a(concrete, self.treasure_model_class) && unsafe { concrete.read::<u8>(self.opened_offset) } != 0
    }

    pub fn classes(&self) -> &ActorClasses {
        &self.classes
    }

    pub fn position(&mut self, actor: Obj) -> Option<Vec3> {
        let at: Vec3 = self.location.call(actor, |_| {}).returned();
        at.x.is_finite().then_some(at)
    }

    fn ask(function: &mut Function, utility: Obj, actor: Obj) -> bool {
        function.call(utility, |a| a.set_obj(0, Some(actor))).returned::<u8>() != 0
    }

    /// Whether the actor should be drawn right now: alive, not already collected or opened, fish still there.
    pub fn present(&mut self, actor: Obj, base: Base) -> bool {
        match base {
            Base::Character => !Self::ask(&mut self.is_dead, self.utility, actor),
            Base::Effigy | Base::Note => {
                let picked: u8 = unsafe { actor.read(self.picked_offset) };
                picked == 0
            }
            Base::Chest => !self.opened(actor),
            Base::FishingSpot => self.fish_gone.call(actor, |_| {}).returned::<u8>() == 0,
            Base::Rock => self.rock_in_world(actor),
            Base::Egg | Base::Resource | Base::FruitTree => true,
        }
    }

    /// The local player's own character: the controller's pawn, or any pawn that controller drives.
    pub fn is_local_player(&self, actor: Obj, pawn: Obj, controller: Obj) -> bool {
        actor == pawn || unsafe { actor.read_obj(self.controller_offset) } == Some(controller)
    }

    pub fn describe(&mut self, actor: Obj, base: Base, icons: Option<&IconTable>) -> Described {
        match base {
            Base::Character => self.describe_character(actor),
            Base::Chest => match self.kind_by_class(actor, &TREASURE_KINDS, Some(CHEST)) {
                Some(kind) => Described::icon(kind),
                None => Described { kind: CHEST, look: ActorLook::Hidden },
            },
            Base::Resource => match self.kind_by_class(actor, &SPAWNER_KINDS, None) {
                Some(kind) => Described::icon(kind),
                None => Described { kind: ORE, look: ActorLook::Hidden },
            },
            Base::FruitTree => Described::icon(SKILL_FRUIT),
            Base::FishingSpot => Described::icon(FISHING),
            Base::Rock => {
                let look = match self.map_object_id(actor) {
                    None => ActorLook::NotYet,
                    Some(id) => match icons.and_then(|icons| icons.for_map_object(id)) {
                        Some(path) => ActorLook::Texture(path.to_owned()),
                        None if ue::is_a(actor, self.weak_point_ore) || self.ores_without_map_icon.contains(&id) => {
                            ActorLook::Texture(compass_icon("T_icon_compass_03"))
                        }
                        None => ActorLook::Hidden,
                    },
                };
                Described { kind: ORE, look }
            }
            Base::Egg => Described {
                kind: EGG,
                look: ActorLook::Texture(egg_icon(actor)),
            },
            Base::Effigy => {
                let relic: u8 = self.relic_type.call(actor, |_| {}).returned();
                let path = icons
                    .and_then(|icons| icons.for_relic(relic))
                    .map_or_else(|| format!("{ITEM_ICONS}T_itemicon_Relic.T_itemicon_Relic"), str::to_owned);
                Described {
                    kind: EFFIGY,
                    look: ActorLook::Texture(path),
                }
            }
            Base::Note => Described {
                kind: NOTE,
                look: ActorLook::Texture(format!("{UI_ICONS}T_icon_compass_ClearCheck.T_icon_compass_ClearCheck")),
            },
        }
    }

    fn describe_character(&mut self, actor: Obj) -> Described {
        // Whether a player is the local one is asked every cull (`is_local_player`): the local character
        // can change after a respawn, and a look decided once would stick.
        if ue::is_a(actor, self.classes.player_character) {
            let look = ActorLook::Texture(format!("{UI_ICONS}T_icon_map_member.T_icon_map_member"));
            return Described { kind: PLAYER, look };
        }
        let individual = self.individual_of(actor);
        let id = individual.map(|individual| self.character_id.call(individual, |_| {}).returned::<Name>());
        if !Self::ask(&mut self.is_pal_monster, self.utility, actor) {
            let look = id.map_or(ActorLook::Hidden, |id| ActorLook::Character { id, shiny: false });
            return Described { kind: HUMAN, look };
        }
        let shiny = individual.is_some_and(|individual| self.is_rare.call(individual, |_| {}).returned::<u8>() != 0);
        let look = id.map_or(ActorLook::Hidden, |id| ActorLook::Character { id, shiny });
        let owned = Self::ask(&mut self.is_otomo, self.utility, actor)
            || Self::ask(&mut self.is_players_otomo, self.utility, actor)
            || Self::ask(&mut self.is_base_camp_pal, self.utility, actor);
        Described {
            kind: if owned { OWN_PAL } else { WILD_PAL },
            look,
        }
    }

    fn individual_of(&mut self, actor: Obj) -> Option<Obj> {
        let component: *mut c_void = self.parameter_component.call(actor, |_| {}).returned();
        let component = Obj::from_raw(component)?;
        let individual: *mut c_void = self.individual.call(component, |_| {}).returned();
        Obj::from_raw(individual)
    }
}

impl Described {
    fn icon(kind: u8) -> Self {
        let path = match kind {
            SKILL_FRUIT => format!("{ITEM_ICONS}T_itemicon_Consume_SkillCard_Neutral.T_itemicon_Consume_SkillCard_Neutral"),
            ORE => compass_icon("T_icon_compass_03"),
            LOTUS => compass_icon("T_icon_compass_13"),
            FORAGE => compass_icon("T_icon_compass_09"),
            JUNK => compass_icon("T_icon_compass_Search_Junk"),
            FISHING => compass_icon("T_icon_compass_15"),
            _ => compass_icon("T_icon_compass_Search_Treasure"),
        };
        Self {
            kind,
            look: ActorLook::Texture(path),
        }
    }
}

fn compass_icon(name: &str) -> String {
    format!("{UI_ICONS}{name}.{name}")
}

fn portrait(name: &str) -> String {
    format!("/Game/Pal/Texture/PalIcon/Normal/T_{name}_icon_normal.T_{name}_icon_normal")
}

/// A picture for an actor marker kind in the settings menu, the same art the minimap draws.
pub fn kind_icon(kind: u8) -> Option<String> {
    Some(match kind {
        WILD_PAL | SHINY_PAL | OWN_PAL => portrait("SheepBall"),
        HUMAN => portrait("Police"),
        PLAYER => compass_icon("T_icon_map_member"),
        EGG => format!("{ITEM_ICONS}T_itemicon_Material_PalEgg.T_itemicon_Material_PalEgg"),
        EFFIGY => format!("{ITEM_ICONS}T_itemicon_Relic.T_itemicon_Relic"),
        NOTE => compass_icon("T_icon_compass_ClearCheck"),
        CHEST | JUNK | SKILL_FRUIT | ORE | LOTUS | FORAGE | FISHING => {
            let ActorLook::Texture(path) = Described::icon(kind).look else { return None };
            path
        }
        _ => return None,
    })
}

/// Wild egg blueprints are BP_MapObject_PickupItem_PalEgg[_<Element>]_C; the element picks the item icon.
fn egg_icon(actor: Obj) -> String {
    let class = actor.class().map(Obj::name).unwrap_or_default();
    let element = class
        .strip_prefix("BP_MapObject_PickupItem_PalEgg")
        .and_then(|rest| rest.strip_suffix("_C"))
        .map(|rest| rest.trim_start_matches('_'));
    let file = match element {
        Some("") | Some("Base") => "T_itemicon_Material_PalEgg".to_string(),
        Some("MutationPal") => "T_itemicon_Material_PalEgg_MutationPal".to_string(),
        Some(element @ ("Dark" | "Dragon" | "Earth" | "Electricity" | "Fire" | "Ice" | "Leaf" | "Water" | "WorldTree")) => {
            format!("T_itemicon_Material_PalEgg_{element}_01")
        }
        _ => "T_itemicon_Material_PalEgg_Unknown".to_string(),
    };
    format!("{ITEM_ICONS}{file}.{file}")
}
