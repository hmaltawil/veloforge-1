//! What makes each scenery what it is, in one place: its sky and grass, its
//! water, the shape of its land, its fields and field edges, what grows
//! there, how its random routes run, and its colours in the simple 3D view.
//!
//! A new scenery is a new `Scenery` (in `route.rs`, where saved routes name
//! it) and an entry here. The land code reads these settings rather than
//! asking which scenery it is, so an entry can mix what the others have
//! (a lake with snow, a coast with fields). A new kind of land shape or
//! water is code (`gfx/terrain.rs`), chosen by a setting here.

use crate::gfx::mountains;
use crate::route::Scenery;
use std::ops::Range;

/// Everything that makes one scenery.
pub struct Spec {
    /// Its name in the menus.
    pub label: &'static str,
    pub water: Water,
    pub land: Land,
    /// The ranges the land rises into far from the roads.
    pub mountains: mountains::Kind,
    /// Above a snow line, rock and then snow (the line set by the route's
    /// highest point); on ridged land, forest thinning out towards the
    /// bare tops.
    pub snow: bool,
    /// Farmland: the ground painted as a patchwork of fields (kept clear of
    /// trees, bushes and rocks).
    pub fields: bool,
    /// Wooden fences along the road: on both sides, or with water beside
    /// the road only on the land side (like the dry-stone walls).
    pub fences: bool,
    /// Which of every five 140 m stretches of road have dry-stone walls
    /// along the fields (by a hash of the stretch: 0 to 4).
    pub walled: Range<u64>,
    pub plants: Plants,
    pub buildings: Buildings,
    pub look: Look,
    pub routes: RouteStyle,
    /// The simple 3D view (no detailed graphics).
    pub simple: Simple,
}

impl Spec {
    /// The sides of the road (-1 left, 1 right) that walls and fences run
    /// along: both, or with water beside the road only the land side (the
    /// water on `water_side`).
    pub fn edge_sides(&self, water_side: f64) -> &'static [f64] {
        if self.water == Water::None {
            &[-1.0, 1.0]
        } else if water_side > 0.0 {
            &[-1.0]
        } else {
            &[1.0]
        }
    }
}

/// Water beside the road, at a fixed level.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Water {
    None,
    /// A lake filling the inside of a loop (a route from one place to
    /// another has none), its shore falling away from the road.
    Lake,
    /// The sea, outside a loop (left of a route from one place to another)
    /// and on to the horizon: beaches where the road is low, cliffs where
    /// it runs high, islands and headlands across it.
    Sea,
}

impl Water {
    /// The water's level, for a route whose lowest point is `min_ele`.
    pub fn level(self, min_ele: f64, looped: bool) -> Option<f64> {
        match self {
            Water::Lake if looped => Some(min_ele - 0.8),
            Water::Sea => Some(min_ele - 4.0),
            _ => None,
        }
    }

    /// Which side of the road the water is (-1 left, 1 right), given the
    /// side the inside of the route is.
    pub fn side(self, inside: f64, looped: bool) -> f64 {
        match (self, looped) {
            (Water::Sea, true) => -inside,
            (Water::Sea, false) => -1.0,
            _ => inside,
        }
    }
}

/// The hills beside the road, rising with the distance from it up to where
/// the mountains take over.
pub struct Land {
    pub shape: LandShape,
    /// How high they rise (m).
    pub height: f64,
    /// How far across they are (m).
    pub size: f64,
    /// Their share of `height` beside the road, and how far from it (m)
    /// they have grown by another whole `height`.
    pub near: f64,
    pub growth: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LandShape {
    /// Rolling, round-topped hills.
    Rounded,
    /// Sharp ridges between valleys.
    Ridged,
}

/// What grows and stands in the land.
pub struct Plants {
    /// The kinds of tree, in turn, each with its share of the trees left
    /// after the kinds before it; the rest are broadleaves.
    pub trees: &'static [(Tree, f64)],
    pub bushes: f64,
    pub rocks: f64,
    /// Of the mown fields near farms.
    pub bales: f64,
}

/// The kinds of tree a scenery can grow (besides the broadleaves that
/// grow where no other does).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tree {
    Pine,
    Birch,
    Oak,
    Maple,
    Cherry,
    JapaneseMaple,
    Cedar,
    BlackPine,
}

/// The buildings along the road.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Buildings {
    None,
    /// Farmsteads: a house facing the road, often a barn behind it and
    /// sometimes a shed, fruit trees in the garden.
    Farms,
    /// The Japanese countryside: thatched farmhouses with a tree and a
    /// stone lantern in the garden, small shrines with their gates, and
    /// now and then a temple's pagoda behind its gate.
    Japanese,
}

/// The sky, light and grass of the detailed 3D view (before the time of
/// day and weather change them).
pub struct Look {
    /// Sky overhead and at the horizon (sRGB).
    pub sky_top: [u8; 3],
    pub sky_horizon: [u8; 3],
    pub sun: [f32; 3],
    /// How thick the haze is (per metre).
    pub fog: f32,
    /// Grass from dark to light.
    pub grass: [[f32; 3]; 3],
}

/// How its random routes run.
pub struct RouteStyle {
    /// Words their names are made of (the second joined to the first).
    pub names: (&'static [&'static str], &'static [&'static str]),
    /// The height of their lowest ground (m).
    pub base_ele: f64,
    /// Bends per km, how far each turns (rad) and how tight (radius, m):
    /// each a range a route picks from.
    pub bends_per_km: (f64, f64),
    pub bend_angles: (f64, f64),
    pub bend_radii: (f64, f64),
    /// How high the land beside an imported route rises (m), in the simple
    /// 3D view.
    pub side_hills: f64,
}

/// Colours and objects of the simple 3D view (sRGB).
pub struct Simple {
    pub sky_top: [u8; 3],
    pub sky_horizon: [u8; 3],
    pub haze: [u8; 3],
    pub grass_a: [u8; 3],
    pub grass_b: [u8; 3],
    pub road: [u8; 3],
    pub shoulder: [u8; 3],
    pub mtn_far: [u8; 3],
    pub mtn_near: [u8; 3],
    pub mtn_top: [u8; 3],
    pub tree_a: [u8; 3],
    pub tree_b: [u8; 3],
    pub trunk: [u8; 3],
    pub water: [u8; 3],
    pub rock: [u8; 3],
    /// How high its distant mountains are (1 for the usual).
    pub mountains: f64,
    /// Rock showing through the grass away from the road.
    pub rocky: bool,
    /// Trees beside the road (per 100 places).
    pub trees: u64,
    /// Which of them are pines: those whose hash leaves less than
    /// `pines.1` over a multiple of `pines.0`.
    pub pines: (u64, u64),
}

impl Scenery {
    /// Everything that makes this scenery.
    pub fn spec(self) -> &'static Spec {
        match self {
            Scenery::Hills => &HILLS,
            Scenery::Lake => &LAKE,
            Scenery::Alpine => &ALPINE,
            Scenery::Coast => &COAST,
            Scenery::Japan => &JAPAN,
        }
    }
}

const HILLS: Spec = Spec {
    label: "Farmland and hills",
    water: Water::None,
    land: Land {
        shape: LandShape::Rounded,
        height: 38.0,
        size: 420.0,
        near: 0.5,
        growth: 650.0,
    },
    mountains: mountains::Kind::Hills,
    snow: false,
    fields: true,
    fences: true,
    walled: 2..3,
    plants: Plants {
        trees: &[
            (Tree::Pine, 0.35),
            (Tree::Birch, 0.3),
            (Tree::Oak, 0.35),
            (Tree::Maple, 0.3),
        ],
        bushes: 0.05,
        rocks: 0.006,
        bales: 0.45,
    },
    buildings: Buildings::Farms,
    look: Look {
        sky_top: [44, 108, 196],
        sky_horizon: [176, 204, 228],
        sun: [1.1, 1.0, 0.86],
        fog: 0.00125,
        grass: [[0.036, 0.09, 0.02], [0.08, 0.15, 0.036], [0.16, 0.16, 0.06]],
    },
    routes: RouteStyle {
        names: (
            &[
                "Oak", "Mill", "Hay", "Thorn", "Fox", "Ash", "Elder", "Bramble", "Kestrel",
                "Barley", "Hazel", "Lark",
            ],
            &[
                "field", "ford", "hurst", "ley", "wick", "combe", "dale", "bury", "wood",
            ],
        ),
        base_ele: 110.0,
        bends_per_km: (0.8, 1.5),
        bend_angles: (0.25, 1.4),
        bend_radii: (60.0, 320.0),
        side_hills: 22.0,
    },
    simple: Simple {
        sky_top: [66, 128, 214],
        sky_horizon: [186, 216, 240],
        haze: [178, 202, 210],
        grass_a: [92, 158, 70],
        grass_b: [84, 148, 64],
        road: [78, 80, 86],
        shoulder: [150, 140, 120],
        mtn_far: [132, 156, 182],
        mtn_near: [98, 134, 112],
        mtn_top: [150, 172, 192],
        tree_a: [46, 110, 52],
        tree_b: [74, 136, 58],
        trunk: [98, 70, 46],
        water: [60, 120, 170],
        rock: [128, 126, 118],
        mountains: 1.0,
        rocky: false,
        trees: 48,
        pines: (5, 2),
    },
};

const LAKE: Spec = Spec {
    label: "Lakeside",
    water: Water::Lake,
    land: Land {
        shape: LandShape::Rounded,
        height: 26.0,
        size: 420.0,
        near: 0.5,
        growth: 700.0,
    },
    mountains: mountains::Kind::Lake,
    snow: false,
    fields: true,
    fences: true,
    walled: 2..3,
    plants: Plants {
        trees: &[
            (Tree::Pine, 0.45),
            (Tree::Birch, 0.4),
            (Tree::Oak, 0.25),
            (Tree::Maple, 0.35),
        ],
        bushes: 0.05,
        rocks: 0.006,
        bales: 0.35,
    },
    buildings: Buildings::Farms,
    look: Look {
        sky_top: [48, 118, 208],
        sky_horizon: [186, 212, 234],
        sun: [1.1, 1.01, 0.88],
        fog: 0.0011,
        grass: [
            [0.032, 0.09, 0.024],
            [0.07, 0.15, 0.04],
            [0.135, 0.16, 0.058],
        ],
    },
    routes: RouteStyle {
        names: (
            &[
                "Still", "Reed", "Heron", "Willow", "Swan", "Glass", "Silver", "Alder", "Otter",
                "Rush",
            ],
            &["water", "mere", "shore", "haven", "holm", "wick", "bank"],
        ),
        base_ele: 40.0,
        bends_per_km: (0.6, 1.1),
        bend_angles: (0.2, 1.1),
        bend_radii: (80.0, 380.0),
        side_hills: 14.0,
    },
    simple: Simple {
        sky_top: [84, 146, 222],
        sky_horizon: [204, 226, 244],
        haze: [190, 212, 224],
        grass_a: [110, 168, 76],
        grass_b: [102, 160, 70],
        road: [84, 86, 92],
        shoulder: [190, 176, 140],
        mtn_far: [140, 164, 192],
        mtn_near: [110, 146, 126],
        mtn_top: [166, 186, 206],
        tree_a: [54, 120, 60],
        tree_b: [90, 150, 66],
        trunk: [104, 76, 50],
        water: [52, 118, 176],
        rock: [140, 136, 126],
        mountains: 1.0,
        rocky: false,
        trees: 40,
        pines: (4, 1),
    },
};

const ALPINE: Spec = Spec {
    label: "Mountains",
    water: Water::None,
    land: Land {
        shape: LandShape::Ridged,
        height: 170.0,
        size: 800.0,
        near: 0.35,
        growth: 450.0,
    },
    mountains: mountains::Kind::Alpine,
    snow: true,
    fields: false,
    fences: false,
    walled: 2..3,
    plants: Plants {
        trees: &[
            (Tree::Pine, 0.95),
            (Tree::Birch, 0.3),
            (Tree::Oak, 0.0),
            (Tree::Maple, 0.25),
        ],
        bushes: 0.02,
        rocks: 0.05,
        bales: 0.0,
    },
    buildings: Buildings::None,
    look: Look {
        sky_top: [28, 86, 190],
        sky_horizon: [160, 194, 228],
        sun: [1.16, 1.07, 0.94],
        // Clear mountain air: the distant land shows, out to the peaks.
        fog: 0.0006,
        grass: [
            [0.032, 0.092, 0.028],
            [0.072, 0.155, 0.042],
            [0.125, 0.165, 0.065],
        ],
    },
    routes: RouteStyle {
        names: (
            &[
                "High", "Eagle", "Ice", "Granite", "Larch", "Cloud", "Chamois", "Grey", "Storm",
                "Pine",
            ],
            &[" Col", " Pass", "horn", " Fell", " Peak", " Alp", " Ridge"],
        ),
        base_ele: 680.0,
        bends_per_km: (1.3, 2.2),
        bend_angles: (0.35, 1.9),
        bend_radii: (40.0, 180.0),
        side_hills: 45.0,
    },
    simple: Simple {
        sky_top: [44, 104, 196],
        sky_horizon: [176, 206, 236],
        haze: [170, 192, 212],
        grass_a: [96, 148, 74],
        grass_b: [88, 138, 68],
        road: [72, 74, 80],
        shoulder: [140, 136, 128],
        mtn_far: [118, 136, 162],
        mtn_near: [92, 112, 118],
        mtn_top: [240, 244, 250],
        tree_a: [30, 84, 52],
        tree_b: [44, 104, 62],
        trunk: [86, 62, 42],
        water: [60, 120, 170],
        rock: [124, 122, 120],
        mountains: 2.2,
        rocky: true,
        trees: 62,
        pines: (1, 1),
    },
};

const COAST: Spec = Spec {
    label: "Coast",
    water: Water::Sea,
    land: Land {
        // Rugged island hills rising inland.
        shape: LandShape::Ridged,
        height: 70.0,
        size: 600.0,
        near: 0.3,
        growth: 500.0,
    },
    mountains: mountains::Kind::Coast,
    snow: false,
    fields: false,
    fences: false,
    walled: 0..2,
    plants: Plants {
        trees: &[
            (Tree::Pine, 0.7),
            (Tree::Birch, 0.15),
            (Tree::Oak, 0.3),
            (Tree::Maple, 0.15),
        ],
        bushes: 0.07,
        rocks: 0.006,
        bales: 0.0,
    },
    buildings: Buildings::Farms,
    look: Look {
        sky_top: [40, 116, 212],
        sky_horizon: [182, 216, 238],
        sun: [1.12, 1.03, 0.9],
        fog: 0.00095,
        grass: [
            [0.05, 0.095, 0.024],
            [0.11, 0.155, 0.042],
            [0.21, 0.185, 0.08],
        ],
    },
    routes: RouteStyle {
        names: (
            &[
                "Gull", "Salt", "Cliff", "Tide", "Kelp", "Puffin", "Amber", "Shell", "Wren",
                "Spray",
            ],
            &["head", " Cove", " Point", " Bay", "strand", "ness", "haven"],
        ),
        base_ele: 18.0,
        bends_per_km: (0.9, 1.6),
        bend_angles: (0.25, 1.4),
        bend_radii: (55.0, 260.0),
        side_hills: 26.0,
    },
    simple: Simple {
        sky_top: [60, 130, 220],
        sky_horizon: [200, 226, 246],
        haze: [190, 214, 230],
        grass_a: [118, 160, 78],
        grass_b: [110, 152, 72],
        road: [84, 86, 92],
        shoulder: [206, 190, 150],
        mtn_far: [150, 176, 200],
        mtn_near: [120, 150, 128],
        mtn_top: [170, 190, 210],
        tree_a: [52, 112, 64],
        tree_b: [96, 140, 70],
        trunk: [104, 76, 50],
        water: [30, 104, 170],
        rock: [150, 140, 124],
        mountains: 0.6,
        rocky: false,
        trees: 34,
        pines: (4, 1),
    },
};

const JAPAN: Spec = Spec {
    label: "Japan",
    water: Water::Lake,
    land: Land {
        // Rolling, wooded hills, rising into mountains beyond the lake.
        shape: LandShape::Rounded,
        height: 46.0,
        size: 380.0,
        near: 0.45,
        growth: 600.0,
    },
    mountains: mountains::Kind::Lake,
    snow: false,
    fields: true,
    fences: false,
    walled: 0..0,
    plants: Plants {
        // Cedar on the hills; cherry, Japanese maple and black pine in the
        // open and at the forest's edge.
        trees: &[
            (Tree::Cedar, 0.45),
            (Tree::Cherry, 0.55),
            (Tree::JapaneseMaple, 0.5),
            (Tree::BlackPine, 0.5),
        ],
        bushes: 0.06,
        rocks: 0.01,
        bales: 0.0,
    },
    buildings: Buildings::Japanese,
    look: Look {
        // A soft spring sky, a little hazy.
        sky_top: [50, 112, 200],
        sky_horizon: [198, 214, 232],
        sun: [1.12, 1.02, 0.9],
        fog: 0.00115,
        grass: [
            [0.034, 0.095, 0.03],
            [0.074, 0.16, 0.045],
            [0.15, 0.17, 0.068],
        ],
    },
    routes: RouteStyle {
        names: (
            &[
                "Sakura", "Matsu", "Momiji", "Hana", "Kaze", "Tsuki", "Mori", "Ishi", "Yuki",
                "Kawa",
            ],
            &["yama", "zaka", "dani", "mura", " Lake", " Pass", " Valley"],
        ),
        base_ele: 260.0,
        bends_per_km: (0.9, 1.6),
        bend_angles: (0.25, 1.4),
        bend_radii: (60.0, 300.0),
        side_hills: 30.0,
    },
    simple: Simple {
        sky_top: [74, 136, 214],
        sky_horizon: [206, 222, 240],
        haze: [196, 210, 226],
        grass_a: [100, 162, 74],
        grass_b: [92, 152, 68],
        road: [80, 82, 88],
        shoulder: [170, 160, 140],
        mtn_far: [136, 160, 190],
        mtn_near: [96, 132, 116],
        mtn_top: [200, 210, 224],
        // Cedar green and cherry pink.
        tree_a: [40, 96, 62],
        tree_b: [214, 160, 180],
        trunk: [92, 66, 58],
        water: [56, 120, 176],
        rock: [132, 130, 124],
        mountains: 1.3,
        rocky: false,
        trees: 50,
        pines: (5, 3),
    },
};
