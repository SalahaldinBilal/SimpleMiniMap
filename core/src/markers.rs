use std::ffi::c_void;

use crate::icons::IconTable;
use crate::ue::{self, Function, Guid, Name, Obj, Vec3};

// EPalLocationType values.
const PLAYER: u8 = 5;
const CHARACTER: u8 = 8;
const QUEST_TRACKING_ACTOR: u8 = 22;

// Types drawn above the fog. Everything else stays hidden under unexplored clouds.
const ABOVE_FOG: [u8; 9] = [4, 8, 12, 13, 15, 19, 21, 22, 23];
const MOVING: [u8; 2] = [CHARACTER, QUEST_TRACKING_ACTOR];

pub struct MarkerInfo {
    pub icon: Option<String>,
    pub above_fog: bool,
    pub moves: bool,
}

pub struct Locations {
    utility: Obj,
    manager_class: Obj,
    level_object_class: Obj,
    map_object_class: Obj,
    get_manager: Function,
    get_type: Function,
    get_id: Function,
    location_point: Function,
    get_location: Function,
    get_relic: Function,
    get_map_object_name: Function,
    should_display_offset: usize,
    show_in_map_offset: usize,
}

impl Locations {
    pub fn new() -> Result<Self, String> {
        let utility = ue::find("/Script/Pal.Default__PalUtility")?;
        let manager_class = ue::find("/Script/Pal.PalLocationManager")?;
        let base_class = ue::find("/Script/Pal.PalLocationBase")?;
        let point_class = ue::find("/Script/Pal.PalLocationPoint")?;
        let level_object_class = ue::find("/Script/Pal.PalLocationPoint_LevelObject")?;
        let map_object_class = ue::find("/Script/Pal.PalLocationPoint_MapObject")?;
        Ok(Self {
            get_manager: Function::resolve(utility, "GetLocationManager", &["WorldContextObject"])?,
            get_type: Function::resolve(base_class, "GetType", &[])?,
            get_id: Function::resolve(base_class, "GetLocationId", &[])?,
            location_point: Function::resolve(base_class, "LocationPoint", &[])?,
            get_location: Function::resolve(point_class, "GetLocation", &[])?,
            get_relic: Function::resolve(level_object_class, "GetRelicType", &[])?,
            get_map_object_name: Function::resolve(map_object_class, "GetMapObjectName", &["MapObjectName"])?,
            should_display_offset: ue::property_offset(base_class, "bShouldDisplay")?,
            show_in_map_offset: ue::property_offset(base_class, "bShowInMap")?,
            utility,
            manager_class,
            level_object_class,
            map_object_class,
        })
    }

    pub fn current(&mut self, world: Obj, out: &mut Vec<Obj>) -> Result<(), String> {
        out.clear();
        let manager: *mut c_void = self
            .get_manager
            .call(self.utility, |a| a.set_obj(0, Some(world)))
            .returned();
        let Some(manager) = Obj::from_raw(manager) else { return Ok(()) };
        let mut pairs = Vec::with_capacity(out.capacity().max(256));
        let value_offset = ue::map_entries(self.manager_class, "LocationMapCombined", manager.raw(), &mut pairs)?;
        out.extend(
            pairs
                .iter()
                .filter_map(|&pair| Obj::from_raw(unsafe { ue::read_at::<*mut c_void>(pair, value_offset) })),
        );
        Ok(())
    }

    pub fn shown_on_map(&self, location: Obj) -> bool {
        let should_display: u8 = unsafe { location.read(self.should_display_offset) };
        let show_in_map: u8 = unsafe { location.read(self.show_in_map_offset) };
        should_display != 0 && show_in_map != 0
    }

    pub fn point(&mut self, location: Obj) -> Option<Obj> {
        let point: *mut c_void = self.location_point.call(location, |_| {}).returned();
        Obj::from_raw(point)
    }

    pub fn id(&mut self, location: Obj) -> Guid {
        self.get_id.call(location, |_| {}).returned()
    }

    pub fn position(&mut self, location: Obj) -> Option<Vec3> {
        let point = self.point(location)?;
        let at: Vec3 = self.get_location.call(point, |_| {}).returned();
        at.x.is_finite().then_some(at)
    }

    pub fn kind(&mut self, location: Obj) -> u8 {
        self.get_type.call(location, |_| {}).returned()
    }

    pub fn describe(&mut self, location: Obj, kind: u8, icons: Option<&IconTable>) -> MarkerInfo {
        let icon = icons.and_then(|icons| {
            let specific = if ue::is_a(location, self.level_object_class) {
                let relic: u8 = self.get_relic.call(location, |_| {}).returned();
                icons.for_relic(relic)
            } else if ue::is_a(location, self.map_object_class) {
                let found = self.get_map_object_name.call(location, |_| {}).returned::<u8>() != 0;
                let name: Name = self.get_map_object_name.param(0);
                if found { icons.for_map_object(name) } else { None }
            } else {
                None
            };
            specific.or_else(|| icons.for_kind(kind)).map(str::to_owned)
        });
        MarkerInfo {
            icon: if kind == PLAYER { None } else { icon },
            above_fog: ABOVE_FOG.contains(&kind),
            moves: MOVING.contains(&kind),
        }
    }

    pub fn is_player(kind: u8) -> bool {
        kind == PLAYER
    }
}
