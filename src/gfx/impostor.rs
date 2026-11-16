//! Pictures of each kind of tree, for drawing distant trees as single
//! cards ("impostors") instead of hundreds of triangles each.
//!
//! The pictures are made from the trees' own meshes (`mesh::pine` and so
//! on) by a small software renderer: each tree is drawn from `FRAMES`
//! directions around it, level with it, into a square atlas. Every texel
//! keeps what the 3D view needs to light the tree as if it were the mesh:
//! its colour, which way the surface faces (in the tree's own frame, so
//! the card can be turned with the tree), and whether it is foliage (lit
//! as leaves and tinted per tree, like the mesh's negative colours).
//!
//! Each texel also keeps how far the surface stands in front of the card's
//! plane (towards the viewer), so the 3D view can light, shadow and fog it
//! where the leaves really are rather than flat on the card.
//!
//! Colours and normals are stored premultiplied by coverage, so the
//! mipmaps (also made here) average only what is really there and edges
//! stay clean at every size. Colours are stored as their square root, which
//! keeps the darks of the foliage from banding in eight bits.

use super::math::*;
use super::mesh::VERTEX_STRIDE;

/// Views around the tree, evenly spaced; the 3D view blends the two
/// nearest to the direction it is seen from.
pub const FRAMES: usize = 16;
/// Views per row of the atlas.
pub const GRID: usize = 4;
/// Texels across one view.
pub const FRAME_PX: usize = 256;
/// Texels across the atlas.
pub const SIZE: usize = FRAME_PX * GRID;
/// The part of the tree's unit space a view covers: half its width (the
/// meshes reach about 0.5 from the trunk) and its bottom and top (they
/// stand on 0 and reach 1).
pub const HALF_WIDTH: f32 = 0.6;
pub const BOTTOM: f32 = -0.05;
pub const TOP: f32 = 1.15;
/// Each view is drawn this many times larger and then averaged down.
const SUPERSAMPLE: usize = 2;

/// One level of the atlas's mipmaps: its size and its texels, row 0 at the
/// bottom, four bytes each.
pub struct Level {
    pub size: usize,
    /// Square-root colour and coverage, premultiplied by coverage.
    pub albedo: Vec<u8>,
    /// Normal (as 0..1) premultiplied by coverage, and foliage times
    /// coverage.
    pub normal: Vec<u8>,
    /// How far in front of the card's plane the surface is (as 0..1 across
    /// `-HALF_WIDTH..HALF_WIDTH`), times coverage: one byte per texel.
    pub depth: Vec<u8>,
}

/// The atlas for one kind of tree, largest level first.
pub struct Atlas {
    pub levels: Vec<Level>,
}

/// The direction a view is seen from, in the tree's frame: its azimuth
/// `phi` measured from +z towards +x.
fn view_axes(frame: usize) -> (V3, V3) {
    let phi = frame as f32 / FRAMES as f32 * std::f32::consts::TAU;
    let towards = [phi.sin(), 0.0, phi.cos()];
    let right = [phi.cos(), 0.0, -phi.sin()];
    (towards, right)
}

/// Float texels of one view while it is drawn.
struct Canvas {
    n: usize,
    depth: Vec<f32>,
    col: Vec<V3>,
    nrm: Vec<V3>,
    leaf: Vec<f32>,
    covered: Vec<bool>,
}

impl Canvas {
    fn new(n: usize) -> Canvas {
        Canvas {
            n,
            depth: vec![f32::NEG_INFINITY; n * n],
            col: vec![[0.0; 3]; n * n],
            nrm: vec![[0.0; 3]; n * n],
            leaf: vec![0.0; n * n],
            covered: vec![false; n * n],
        }
    }
}

/// A vertex projected into a view: texel coordinates, nearness, and what
/// is carried across the triangle.
#[derive(Clone, Copy)]
struct Projected {
    x: f32,
    y: f32,
    z: f32,
    col: V3,
    nrm: V3,
    leaf: f32,
}

fn rasterise(c: &mut Canvas, t: [Projected; 3]) {
    let [a, b, d] = t;
    let area = (b.x - a.x) * (d.y - a.y) - (b.y - a.y) * (d.x - a.x);
    if area.abs() < 1e-9 {
        return;
    }
    let n = c.n as f32;
    let x0 = a.x.min(b.x).min(d.x).floor().max(0.0) as usize;
    let x1 = (a.x.max(b.x).max(d.x).ceil().min(n - 1.0)).max(0.0) as usize;
    let y0 = a.y.min(b.y).min(d.y).floor().max(0.0) as usize;
    let y1 = (a.y.max(b.y).max(d.y).ceil().min(n - 1.0)).max(0.0) as usize;
    for y in y0..=y1 {
        for x in x0..=x1 {
            // Sample at the texel's centre; triangles of either winding.
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((b.x - px) * (d.y - py) - (b.y - py) * (d.x - px)) / area;
            let w1 = ((d.x - px) * (a.y - py) - (d.y - py) * (a.x - px)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let z = w0 * a.z + w1 * b.z + w2 * d.z;
            let i = y * c.n + x;
            if z <= c.depth[i] {
                continue;
            }
            c.depth[i] = z;
            c.covered[i] = true;
            let mix = |p: V3, q: V3, r: V3| add(add(scale(p, w0), scale(q, w1)), scale(r, w2));
            c.col[i] = mix(a.col, b.col, d.col);
            c.nrm[i] = mix(a.nrm, b.nrm, d.nrm);
            c.leaf[i] = w0 * a.leaf + w1 * b.leaf + w2 * d.leaf;
        }
    }
}

/// Draw `verts` (mesh vertices, `VERTEX_STRIDE` floats each, three per
/// triangle) into the atlas.
pub fn bake(verts: &[f32]) -> Atlas {
    let big = FRAME_PX * SUPERSAMPLE;
    let mut albedo = vec![0u8; SIZE * SIZE * 4];
    let mut normal = vec![0u8; SIZE * SIZE * 4];
    let mut depth = vec![0u8; SIZE * SIZE];
    let tris = verts.as_chunks::<{ VERTEX_STRIDE * 3 }>().0;
    for frame in 0..FRAMES {
        let (towards, right) = view_axes(frame);
        let mut canvas = Canvas::new(big);
        let to_px = |u: f32, v: f32| {
            (
                (u + HALF_WIDTH) / (2.0 * HALF_WIDTH) * big as f32,
                (v - BOTTOM) / (TOP - BOTTOM) * big as f32,
            )
        };
        for t in tris {
            let p = |k: usize| -> Projected {
                let v = &t[k * VERTEX_STRIDE..(k + 1) * VERTEX_STRIDE];
                let pos = [v[0], v[1], v[2]];
                let (x, y) = to_px(dot(pos, right), pos[1]);
                // Negative colours are tinted per tree: foliage.
                let tinted = v[6] < 0.0;
                Projected {
                    x,
                    y,
                    z: dot(pos, towards),
                    col: [v[6].abs(), v[7].abs(), v[8].abs()],
                    nrm: [v[3], v[4], v[5]],
                    leaf: if tinted { 1.0 } else { 0.0 },
                }
            };
            rasterise(&mut canvas, [p(0), p(1), p(2)]);
        }
        // Average down into the frame's place in the atlas.
        let (fx, fy) = (frame % GRID, frame / GRID);
        for y in 0..FRAME_PX {
            for x in 0..FRAME_PX {
                let mut cov = 0.0;
                let mut col = [0.0; 3];
                let mut nrm = [0.0; 3];
                let mut leaf = 0.0;
                let mut front = 0.0;
                for sy in 0..SUPERSAMPLE {
                    for sx in 0..SUPERSAMPLE {
                        let i = (y * SUPERSAMPLE + sy) * big + x * SUPERSAMPLE + sx;
                        if canvas.covered[i] {
                            cov += 1.0;
                            let c = canvas.col[i];
                            col = add(col, [c[0].sqrt(), c[1].sqrt(), c[2].sqrt()]);
                            let n = norm(canvas.nrm[i]);
                            nrm = add(nrm, [n[0] * 0.5 + 0.5, n[1] * 0.5 + 0.5, n[2] * 0.5 + 0.5]);
                            leaf += canvas.leaf[i];
                            front += (canvas.depth[i] / (2.0 * HALF_WIDTH) + 0.5).clamp(0.0, 1.0);
                        }
                    }
                }
                let k = (SUPERSAMPLE * SUPERSAMPLE) as f32;
                let o = ((fy * FRAME_PX + y) * SIZE + fx * FRAME_PX + x) * 4;
                let byte = |v: f32| (v / k * 255.0).round().clamp(0.0, 255.0) as u8;
                albedo[o..o + 4].copy_from_slice(&[
                    byte(col[0]),
                    byte(col[1]),
                    byte(col[2]),
                    byte(cov),
                ]);
                normal[o..o + 4].copy_from_slice(&[
                    byte(nrm[0]),
                    byte(nrm[1]),
                    byte(nrm[2]),
                    byte(leaf),
                ]);
                depth[o / 4] = byte(front);
            }
        }
    }
    // Averaged down, the gaps between clumps thin a tree's coverage, so a
    // distant card would look sparser than the mesh it stands in for. Each
    // level's coverage is scaled so the tree covers as much of its picture
    // as at full size (colour and normal with it, so they decode the same).
    let solid = |l: &[u8], k: f32| {
        l.as_chunks::<4>()
            .0
            .iter()
            .filter(|t| t[3] as f32 * k >= 127.5)
            .count()
    };
    let target = solid(&albedo, 1.0);
    let mut raw = vec![Level {
        size: SIZE,
        albedo,
        normal,
        depth,
    }];
    while raw.last().unwrap().size > 1 {
        let prev = raw.last().unwrap();
        let size = prev.size / 2;
        let half = |src: &[u8], chans: usize| -> Vec<u8> {
            let mut out = vec![0u8; size * size * chans];
            for y in 0..size {
                for x in 0..size {
                    for ch in 0..chans {
                        let at = |dx: usize, dy: usize| {
                            src[((y * 2 + dy) * prev.size + x * 2 + dx) * chans + ch] as u32
                        };
                        let sum = at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1);
                        out[(y * size + x) * chans + ch] = ((sum + 2) / 4) as u8;
                    }
                }
            }
            out
        };
        let next = Level {
            size,
            albedo: half(&prev.albedo, 4),
            normal: half(&prev.normal, 4),
            depth: half(&prev.depth, 1),
        };
        raw.push(next);
    }
    let levels = raw
        .into_iter()
        .map(|l| {
            let want = target as f64 * (l.size * l.size) as f64 / (SIZE * SIZE) as f64;
            let (mut lo, mut hi) = (1.0f32, 4.0f32);
            for _ in 0..24 {
                let k = 0.5 * (lo + hi);
                if (solid(&l.albedo, k) as f64) < want {
                    lo = k;
                } else {
                    hi = k;
                }
            }
            let k = if l.size == SIZE { 1.0 } else { lo };
            let scaled = |data: &[u8], chans: usize| -> Vec<u8> {
                let mut out = data.to_vec();
                for (o, t) in out
                    .chunks_exact_mut(chans)
                    .zip(l.albedo.as_chunks::<4>().0.iter())
                {
                    // Scale the texel by k, but never past full coverage.
                    let f = if t[3] == 0 {
                        1.0
                    } else {
                        k.min(255.0 / t[3] as f32)
                    };
                    for c in o.iter_mut() {
                        *c = (*c as f32 * f).round().min(255.0) as u8;
                    }
                }
                out
            };
            Level {
                size: l.size,
                albedo: scaled(&l.albedo, 4),
                normal: scaled(&l.normal, 4),
                depth: scaled(&l.depth, 1),
            }
        })
        .collect();
    Atlas { levels }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::mesh;

    /// Coverage of one view at the largest level.
    fn coverage(a: &Atlas, frame: usize) -> f32 {
        let l = &a.levels[0];
        let (fx, fy) = (frame % GRID, frame / GRID);
        let mut sum = 0u32;
        for y in 0..FRAME_PX {
            for x in 0..FRAME_PX {
                sum += l.albedo[((fy * FRAME_PX + y) * SIZE + fx * FRAME_PX + x) * 4 + 3] as u32;
            }
        }
        sum as f32 / (255.0 * (FRAME_PX * FRAME_PX) as f32)
    }

    #[test]
    fn every_view_shows_the_whole_tree() {
        for (name, m) in [
            ("pine", mesh::pine()),
            ("broadleaf", mesh::broadleaf()),
            ("birch", mesh::birch()),
        ] {
            let a = bake(&m.v);
            assert_eq!(a.levels[0].size, SIZE);
            assert_eq!(a.levels.last().unwrap().size, 1);
            for f in 0..FRAMES {
                let c = coverage(&a, f);
                // A tree fills a good part of its view but never the edges.
                assert!(c > 0.12 && c < 0.75, "{name} view {f}: coverage {c}");
                let l = &a.levels[0];
                let (fx, fy) = (f % GRID, f / GRID);
                for i in 0..FRAME_PX {
                    for (x, y) in [(0, i), (FRAME_PX - 1, i), (i, FRAME_PX - 1)] {
                        let o = ((fy * FRAME_PX + y) * SIZE + fx * FRAME_PX + x) * 4;
                        assert_eq!(l.albedo[o + 3], 0, "{name} view {f} touches its edge");
                    }
                }
            }
        }
    }

    #[test]
    fn foliage_is_marked_and_trunks_are_not() {
        let a = bake(&mesh::broadleaf().v);
        let l = &a.levels[0];
        // The middle of the first view, low down: the trunk. Higher up: leaves.
        let at = |x: usize, y: usize| (y * SIZE + x) * 4;
        let trunk = at(
            FRAME_PX / 2,
            ((0.2 - BOTTOM) / (TOP - BOTTOM) * FRAME_PX as f32) as usize,
        );
        let crown = at(
            FRAME_PX / 2,
            ((0.7 - BOTTOM) / (TOP - BOTTOM) * FRAME_PX as f32) as usize,
        );
        assert!(
            l.albedo[trunk + 3] > 200 && l.normal[trunk + 3] < 30,
            "trunk"
        );
        assert!(
            l.albedo[crown + 3] > 200 && l.normal[crown + 3] > 200,
            "crown"
        );
    }

    #[test]
    fn mipmaps_keep_the_colour_of_what_is_there() {
        // Premultiplied: averaged down, the tree keeps its colour instead of
        // fading into the empty background, at every level.
        let a = bake(&mesh::pine().v);
        let mean = |l: &Level| {
            let (mut c, mut w) = (0.0, 0.0);
            for t in l.albedo.as_chunks::<4>().0.iter() {
                c += t[1] as f64;
                w += t[3] as f64;
            }
            c / w
        };
        let top = mean(&a.levels[0]);
        for l in &a.levels[1..7] {
            let m = mean(l);
            assert!(
                (m - top).abs() < 0.02 * top,
                "level {}: {m} against {top}",
                l.size
            );
        }
    }

    /// Write the atlases as raw RGBA (1024 x 1024, row 0 at the bottom) to
    /// look at: `VELOFORGE_IMPOSTOR_OUT=dir cargo test write_atlases -- --ignored`.
    #[test]
    #[ignore]
    fn write_atlases() {
        let dir = std::env::var("VELOFORGE_IMPOSTOR_OUT").expect("VELOFORGE_IMPOSTOR_OUT");
        for (name, m) in [
            ("pine", mesh::pine()),
            ("broadleaf", mesh::broadleaf()),
            ("birch", mesh::birch()),
        ] {
            let a = bake(&m.v);
            std::fs::write(format!("{dir}/{name}-albedo.rgba"), &a.levels[0].albedo).unwrap();
            std::fs::write(format!("{dir}/{name}-normal.rgba"), &a.levels[0].normal).unwrap();
        }
    }

    #[test]
    fn distant_pictures_stay_as_dense_as_the_tree() {
        let a = bake(&mesh::broadleaf().v);
        let solid = |l: &Level| {
            l.albedo
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|t| t[3] >= 128)
                .count() as f32
                / (l.size * l.size) as f32
        };
        let top = solid(&a.levels[0]);
        for l in &a.levels[1..6] {
            let s = solid(l);
            assert!(
                (s - top).abs() < 0.04,
                "level {}: covers {s} against {top}",
                l.size
            );
        }
    }
}
