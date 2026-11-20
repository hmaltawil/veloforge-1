//! What grows and stands where. Each scenery has a palette: the kinds of
//! tree that grow in it, in turn, how often each grows, and what land each
//! likes, and how much undergrowth and how many rocks and farmhouses there
//! are. A spot of land is described by what it is like (height, the forest
//! around it, how far the road is), and the palette picks what stands there.
//!
//! New kinds (a maple, a palm) are added here: a model, its size and colour,
//! its share in each palette and what land it likes; nothing else changes.

use super::terrain::{BIRCH, BROADLEAF, PINE};
use crate::route::Scenery;

/// What a spot of land is like. Kinds that care about more (height above
/// the sea, slope, dryness) add it here.
#[derive(Clone, Copy, Debug)]
pub struct Spot {
    /// How thick the forest around it is (0 open land .. 1 deep forest).
    pub forest: f64,
    /// Distance to the road (metres).
    pub dist: f64,
}

/// A kind of tree, and where it grows.
pub struct TreeKind {
    /// Which model it is drawn with (the kind constants in `terrain`).
    pub kind: usize,
    /// How often it grows where it may, before the kinds after it.
    pub share: f64,
    /// How well a spot suits it (1 as well as anywhere, 0 not at all).
    pub likes: fn(&Spot) -> f64,
    /// Height of a typical tree (metres), and its width for its height.
    pub height: f64,
    pub width: f64,
    /// How much of the yellower autumn-like warmth its leaves may have.
    pub warm: f32,
}

/// What a scenery grows and builds.
pub struct Palette {
    /// The kinds of tree, tried in turn; the last grows where none of the
    /// others did.
    pub trees: Vec<TreeKind>,
    /// How much undergrowth (bushes) and how many loose rocks there are.
    pub bushes: f64,
    pub rocks: f64,
    /// Farmhouses along the road.
    pub farmhouses: bool,
}

/// What stands on a spot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cover {
    Tree,
    Bush,
    Rock,
}

fn anywhere(_: &Spot) -> f64 {
    1.0
}

/// Birches stand among the broadleaves, mostly at the forest's edge.
fn forest_edge(s: &Spot) -> f64 {
    1.3 - s.forest
}

const fn pine(share: f64) -> TreeKind {
    TreeKind {
        kind: PINE,
        share,
        likes: anywhere,
        height: 14.0,
        width: 0.72,
        warm: 0.25,
    }
}

const fn birch(share: f64) -> TreeKind {
    TreeKind {
        kind: BIRCH,
        share,
        likes: forest_edge,
        height: 11.0,
        width: 0.7,
        warm: 0.7,
    }
}

const BROADLEAF_TREE: TreeKind = TreeKind {
    kind: BROADLEAF,
    share: 1.0,
    likes: anywhere,
    height: 10.5,
    width: 1.0,
    warm: 0.7,
};

impl Palette {
    pub fn of(scenery: Scenery) -> Palette {
        let (pines, birches, bushes) = match scenery {
            Scenery::Alpine => (0.95, 0.3, 0.02),
            Scenery::Hills => (0.35, 0.3, 0.05),
            Scenery::Lake => (0.45, 0.4, 0.05),
            Scenery::Coast => (0.7, 0.15, 0.07),
        };
        Palette {
            trees: vec![pine(pines), birch(birches), BROADLEAF_TREE],
            bushes,
            rocks: if scenery == Scenery::Alpine {
                0.05
            } else {
                0.006
            },
            farmhouses: scenery != Scenery::Alpine,
        }
    }

    /// What stands on `spot`, for a random `r` in 0..1: a tree, more often
    /// in thicker forest, else perhaps a bush (not right by the road) or a
    /// rock, else nothing.
    pub fn cover(&self, spot: &Spot, r: f64) -> Option<Cover> {
        let trees = spot.forest * 0.85 + 0.025;
        if r < trees {
            Some(Cover::Tree)
        } else if r < trees + self.bushes * (0.4 + spot.forest) && spot.dist > 12.0 {
            Some(Cover::Bush)
        } else if r < trees + self.bushes * 1.4 + self.rocks {
            Some(Cover::Rock)
        } else {
            None
        }
    }

    /// Which kind of tree grows on `spot`: each in turn, with its share
    /// times how much it likes the spot, drawing a random number from
    /// `next` for each one tried; the last where none of the others did.
    pub fn tree(&self, spot: &Spot, mut next: impl FnMut() -> f64) -> &TreeKind {
        let (last, tried) = self.trees.split_last().expect("a palette has trees");
        for t in tried {
            if next() < t.share * (t.likes)(spot) {
                return t;
            }
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: u64) -> impl FnMut() -> f64 {
        let mut x = seed;
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    #[test]
    fn each_scenery_grows_its_own_mix() {
        let spot = Spot {
            forest: 0.6,
            dist: 50.0,
        };
        for (scenery, pines) in [(Scenery::Alpine, 0.95), (Scenery::Hills, 0.35)] {
            let p = Palette::of(scenery);
            let mut next = rng(7);
            let n = 20_000;
            let got = (0..n)
                .filter(|_| p.tree(&spot, &mut next).kind == PINE)
                .count() as f64
                / n as f64;
            assert!((got - pines).abs() < 0.015, "{scenery:?}: {got}");
        }
        assert!(!Palette::of(Scenery::Alpine).farmhouses);
        assert!(Palette::of(Scenery::Hills).farmhouses);
    }

    #[test]
    fn a_kind_never_grows_where_it_does_not_like_the_land() {
        // A kind that grows only in the open, ahead of the usual ones.
        fn open_land(s: &Spot) -> f64 {
            if s.forest < 0.2 {
                1.0
            } else {
                0.0
            }
        }
        let mut p = Palette::of(Scenery::Hills);
        p.trees.insert(
            0,
            TreeKind {
                kind: 99,
                share: 0.5,
                likes: open_land,
                height: 8.0,
                width: 1.0,
                warm: 0.0,
            },
        );
        let mut next = rng(3);
        let deep = Spot {
            forest: 0.9,
            dist: 50.0,
        };
        let open = Spot {
            forest: 0.1,
            dist: 50.0,
        };
        assert!((0..5000).all(|_| p.tree(&deep, &mut next).kind != 99));
        let in_open = (0..5000)
            .filter(|_| p.tree(&open, &mut next).kind == 99)
            .count();
        assert!((2300..2700).contains(&in_open), "{in_open}");
    }

    #[test]
    fn thicker_forest_has_more_trees_and_bushes_keep_off_the_road() {
        let p = Palette::of(Scenery::Hills);
        let count = |spot: Spot, c: Cover| {
            (0..1000)
                .filter(|&i| p.cover(&spot, i as f64 / 1000.0) == Some(c))
                .count()
        };
        let thin = Spot {
            forest: 0.1,
            dist: 50.0,
        };
        let thick = Spot {
            forest: 0.8,
            dist: 50.0,
        };
        assert!(count(thick, Cover::Tree) > 4 * count(thin, Cover::Tree));
        let by_road = Spot {
            forest: 0.5,
            dist: 11.0,
        };
        assert_eq!(count(by_road, Cover::Bush), 0);
        assert!(count(thick, Cover::Bush) > 0);
    }
}
