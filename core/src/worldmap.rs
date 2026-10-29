use crate::ue::Vec2;

// Bounds from DT_WorldMapUIData. Map u follows world Y, map v follows -world X.
pub struct Region {
    pub name: &'static str,
    pub texture: &'static str,
    min_x: f64,
    max_x: f64,
    min_y: f64,
    max_y: f64,
}

pub static MAIN: Region = Region {
    name: "MainMap",
    texture: "/Game/Pal/Texture/UI/Map/T_WorldMap.T_WorldMap",
    min_x: -1_099_400.0,
    max_x: 349_400.0,
    min_y: -724_400.0,
    max_y: 724_400.0,
};

pub static TREE: Region = Region {
    name: "Tree",
    texture: "/Game/Pal/Texture/UI/Map/T_TreeMap.T_TreeMap",
    min_x: 347_351.5,
    max_x: 689_148.5,
    min_y: -818_197.0,
    max_y: -476_400.0,
};

pub static REGIONS: [&Region; 2] = [&MAIN, &TREE];

impl Region {
    pub fn to_uv(&self, world_x: f64, world_y: f64) -> (f64, f64) {
        (
            (world_y - self.min_y) / (self.max_y - self.min_y),
            (self.max_x - world_x) / (self.max_x - self.min_x),
        )
    }

    pub fn contains(&self, world_x: f64, world_y: f64) -> bool {
        (self.min_x..=self.max_x).contains(&world_x) && (self.min_y..=self.max_y).contains(&world_y)
    }

    /// Size of the whole region image when `view_units` of world fit across `box_size` slate units.
    pub fn image_size(&self, box_size: f64, view_units: f64) -> Vec2 {
        let scale = box_size / view_units;
        Vec2 {
            x: (self.max_y - self.min_y) * scale,
            y: (self.max_x - self.min_x) * scale,
        }
    }
}
