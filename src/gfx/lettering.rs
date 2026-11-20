//! Lettering painted on things in the world (the banners over the road):
//! block capitals and figures made of straight strokes, built as part of a
//! mesh so they sit exactly on what they are painted on, lit and hazed with
//! it, instead of floating over the picture.

use super::math::*;
use super::mesh::Mesh;

/// Each character's strokes on a grid 4 wide and 6 high (0 at the bottom).
fn strokes(c: char) -> &'static [(f32, f32, f32, f32)] {
    match c {
        '0' => &[
            (0., 0., 4., 0.),
            (4., 0., 4., 6.),
            (4., 6., 0., 6.),
            (0., 6., 0., 0.),
        ],
        '1' => &[(2., 0., 2., 6.), (2., 6., 0.8, 4.8), (0.8, 0., 3.2, 0.)],
        '2' => &[
            (0., 6., 4., 6.),
            (4., 6., 4., 3.),
            (4., 3., 0., 3.),
            (0., 3., 0., 0.),
            (0., 0., 4., 0.),
        ],
        '3' => &[
            (0., 6., 4., 6.),
            (4., 6., 4., 0.),
            (4., 0., 0., 0.),
            (1., 3., 4., 3.),
        ],
        '4' => &[(0., 6., 0., 3.), (0., 3., 4., 3.), (4., 6., 4., 0.)],
        '5' | 'S' => &[
            (4., 6., 0., 6.),
            (0., 6., 0., 3.),
            (0., 3., 4., 3.),
            (4., 3., 4., 0.),
            (4., 0., 0., 0.),
        ],
        '6' => &[
            (4., 6., 0., 6.),
            (0., 6., 0., 0.),
            (0., 0., 4., 0.),
            (4., 0., 4., 3.),
            (4., 3., 0., 3.),
        ],
        '7' => &[(0., 6., 4., 6.), (4., 6., 1.2, 0.)],
        '8' => &[
            (0., 0., 4., 0.),
            (4., 0., 4., 6.),
            (4., 6., 0., 6.),
            (0., 6., 0., 0.),
            (0., 3., 4., 3.),
        ],
        '9' => &[
            (4., 3., 0., 3.),
            (0., 3., 0., 6.),
            (0., 6., 4., 6.),
            (4., 6., 4., 0.),
            (4., 0., 0., 0.),
        ],
        'A' => &[(0., 0., 2., 6.), (2., 6., 4., 0.), (1., 2.4, 3., 2.4)],
        'F' => &[(0., 0., 0., 6.), (0., 6., 4., 6.), (0., 3., 3., 3.)],
        'H' => &[(0., 0., 0., 6.), (4., 0., 4., 6.), (0., 3., 4., 3.)],
        'I' => &[(2., 0., 2., 6.), (1., 0., 3., 0.), (1., 6., 3., 6.)],
        'K' => &[(0., 0., 0., 6.), (0., 2.6, 4., 6.), (1.3, 3.6, 4., 0.)],
        'M' => &[
            (0., 0., 0., 6.),
            (0., 6., 2., 2.8),
            (2., 2.8, 4., 6.),
            (4., 6., 4., 0.),
        ],
        'N' => &[(0., 0., 0., 6.), (0., 6., 4., 0.), (4., 0., 4., 6.)],
        'R' => &[
            (0., 0., 0., 6.),
            (0., 6., 4., 6.),
            (4., 6., 4., 3.),
            (4., 3., 0., 3.),
            (1.6, 3., 4., 0.),
        ],
        'T' => &[(0., 6., 4., 6.), (2., 6., 2., 0.)],
        '·' => &[(2., 2.6, 2., 3.4)],
        _ => &[],
    }
}

/// Grid units from one character to the next.
const ADVANCE: f32 = 6.0;
/// How thick a stroke is, in grid units.
const STROKE: f32 = 0.95;

/// How wide `text` is when `height` tall.
pub fn width(text: &str, height: f32) -> f32 {
    let n = text.chars().count() as f32;
    ((n * ADVANCE - (ADVANCE - 4.0)) * height / 6.0).max(0.0)
}

/// Paint `text` `height` tall, centred on `centre`, on a surface whose
/// right and up directions are `right` and `up` (its face looking along
/// their cross product, towards whoever reads it).
pub fn paint(m: &mut Mesh, text: &str, centre: V3, right: V3, up: V3, height: f32, col: V3) {
    let unit = height / 6.0;
    let face = norm(cross(right, up));
    let w = width(text, height);
    let origin = sub(sub(centre, scale(right, w / 2.0)), scale(up, height / 2.0));
    let at = |x: f32, y: f32| add(origin, add(scale(right, x * unit), scale(up, y * unit)));
    for (k, c) in text.chars().enumerate() {
        let x0 = k as f32 * ADVANCE;
        for &(ax, ay, bx, by) in strokes(c) {
            let (a, b) = (at(x0 + ax, ay), at(x0 + bx, by));
            let dir = sub(b, a);
            let l = len(dir);
            let along = if l > 1e-6 { scale(dir, 1.0 / l) } else { up };
            let across = norm(cross(face, along));
            let half = STROKE * unit / 2.0;
            // Square ends half a stroke past each point, so strokes meet at
            // their corners without gaps.
            let (a, b) = (sub(a, scale(along, half)), add(b, scale(along, half)));
            let quad = [
                sub(a, scale(across, half)),
                sub(b, scale(across, half)),
                add(b, scale(across, half)),
                add(a, scale(across, half)),
            ];
            for i in [0, 1, 2, 0, 2, 3] {
                m.push(quad[i], face, col);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_banner_character_has_strokes() {
        for text in ["START", "FINISH", "START · FINISH", "0123456789 KM"] {
            for c in text.chars().filter(|c| *c != ' ') {
                assert!(!strokes(c).is_empty(), "{c}");
            }
        }
    }

    #[test]
    fn lettering_sits_on_its_surface_centred() {
        let mut m = Mesh::default();
        let (right, up) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        paint(&mut m, "12 KM", [5.0, 3.0, 2.0], right, up, 0.7, [1.0; 3]);
        let v: Vec<&[f32]> = m.v.chunks(super::super::mesh::VERTEX_STRIDE).collect();
        assert!(!v.is_empty());
        // Flat on the surface, facing out of it.
        assert!(v
            .iter()
            .all(|p| (p[2] - 2.0).abs() < 1e-5 && (p[5] - 1.0).abs() < 1e-5));
        let (lo, hi) = v.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p[0]), hi.max(p[0]))
        });
        assert!(((lo + hi) / 2.0 - 5.0).abs() < 0.05, "centred: {lo} {hi}");
        let (bot, top) = v.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p[1]), hi.max(p[1]))
        });
        // As tall as asked, and half a stroke more at each end.
        assert!(
            (top - bot - 0.7 * (1.0 + STROKE / 6.0)).abs() < 0.06,
            "{bot} {top}"
        );
    }
}
