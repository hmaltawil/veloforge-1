//! The picture behind the screens: decoding it, giving each part of the
//! app its own colour of it, and the soft copy that frosted panels show.
//! Pure pixels, no GL, so it can be tested and reused.

/// An RGBA picture, row by row.
#[derive(Clone)]
pub struct Picture {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

/// The picture that comes with the app.
pub const WAVES: &[u8] = include_bytes!("../assets/wallpapers/waves.jpg");

/// Wider pictures are shrunk to this when loaded: the screen never needs
/// more, and each colour of it is kept as a texture.
const MAX_WIDTH: u32 = 1920;

/// Decode a JPEG or PNG, shrunk to at most `MAX_WIDTH` across.
pub fn decode(bytes: &[u8]) -> Option<Picture> {
    let mut img = image::load_from_memory(bytes).ok()?.to_rgba8();
    if img.width() > MAX_WIDTH {
        let h = (img.height() as u64 * MAX_WIDTH as u64 / img.width() as u64).max(1) as u32;
        img = image::imageops::resize(&img, MAX_WIDTH, h, image::imageops::FilterType::Triangle);
    }
    Some(Picture {
        size: [img.width() as usize, img.height() as usize],
        rgba: img.into_raw(),
    })
}

/// Turn every colour round the colour wheel by `degrees` (keeping how light
/// it is), then scale how colourful it is by `saturation` (0 grey, 1 as is).
/// The turn is made in YIQ, the way television colour works, so lightness
/// stays put while the hue moves.
pub fn tint(p: &Picture, degrees: f32, saturation: f32) -> Picture {
    let (s, c) = degrees.to_radians().sin_cos();
    let mut out = p.clone();
    for px in out.rgba.as_chunks_mut::<4>().0 {
        let [r, g, b] = [px[0], px[1], px[2]].map(|v| v as f32 / 255.0);
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        let i = 0.596 * r - 0.274 * g - 0.322 * b;
        let q = 0.211 * r - 0.523 * g + 0.312 * b;
        let (i, q) = ((i * c + q * s) * saturation, (q * c - i * s) * saturation);
        let rgb = [
            y + 0.956 * i + 0.621 * q,
            y - 0.272 * i - 0.647 * q,
            y - 1.106 * i + 1.703 * q,
        ];
        for (d, v) in px.iter_mut().zip(rgb) {
            *d = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    out
}

/// A small copy about `width` pixels across, for thumbnails.
pub fn shrink(p: &Picture, width: usize) -> Picture {
    let [w, h] = p.size;
    let k = (w / width.max(1)).max(1);
    let (sw, sh) = ((w / k).max(1), (h / k).max(1));
    let mut out = Vec::with_capacity(sw * sh * 4);
    for y in 0..sh {
        for x in 0..sw {
            let mut sum = [0u32; 4];
            for yy in y * k..((y + 1) * k).min(h) {
                for xx in x * k..((x + 1) * k).min(w) {
                    let o = (yy * w + xx) * 4;
                    for (s, v) in sum.iter_mut().zip(&p.rgba[o..o + 4]) {
                        *s += *v as u32;
                    }
                }
            }
            let n = (k * k) as u32;
            out.extend(sum.iter().map(|s| (s / n) as u8));
        }
    }
    Picture {
        size: [sw, sh],
        rgba: out,
    }
}

/// A small, very soft copy for frosted glass: about 1/8 the size, blurred.
/// Drawn stretched with linear filtering, it looks like the picture seen
/// through frosted glass, without blurring anything each frame.
pub fn frosted(p: &Picture) -> Picture {
    let [w, h] = p.size;
    let k = (w / 240).max(1);
    let (sw, sh) = ((w / k).max(1), (h / k).max(1));
    // Shrink by averaging k x k blocks.
    let mut small = vec![0f32; sw * sh * 4];
    for y in 0..sh {
        for x in 0..sw {
            let mut sum = [0f32; 4];
            for yy in y * k..((y + 1) * k).min(h) {
                for xx in x * k..((x + 1) * k).min(w) {
                    let o = (yy * w + xx) * 4;
                    for (s, v) in sum.iter_mut().zip(&p.rgba[o..o + 4]) {
                        *s += *v as f32;
                    }
                }
            }
            let n = (k * k) as f32;
            for (c, s) in sum.iter().enumerate() {
                small[(y * sw + x) * 4 + c] = s / n;
            }
        }
    }
    // Three box blurs, across then down, come close to a Gaussian.
    for _ in 0..3 {
        small = box_blur(&small, sw, sh, 3, true);
        small = box_blur(&small, sw, sh, 3, false);
    }
    Picture {
        size: [sw, sh],
        rgba: small
            .iter()
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
            .collect(),
    }
}

/// Average each pixel with `r` neighbours either side, along rows
/// (`across`) or columns; the edge pixels repeat past the edge.
fn box_blur(src: &[f32], w: usize, h: usize, r: usize, across: bool) -> Vec<f32> {
    let mut out = vec![0f32; src.len()];
    let (len, lines) = if across { (w, h) } else { (h, w) };
    let at = |line: usize, i: usize| {
        if across {
            (line * w + i) * 4
        } else {
            (i * w + line) * 4
        }
    };
    let n = (2 * r + 1) as f32;
    for line in 0..lines {
        for i in 0..len {
            let mut sum = [0f32; 4];
            for d in 0..=2 * r {
                let j = (i + d).saturating_sub(r).min(len - 1);
                let o = at(line, j);
                for (s, v) in sum.iter_mut().zip(&src[o..o + 4]) {
                    *s += v;
                }
            }
            let o = at(line, i);
            for c in 0..4 {
                out[o + c] = sum[c] / n;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(rgb: [u8; 3], w: usize, h: usize) -> Picture {
        Picture {
            size: [w, h],
            rgba: [rgb[0], rgb[1], rgb[2], 255].repeat(w * h),
        }
    }

    #[test]
    fn the_bundled_picture_decodes() {
        let p = decode(WAVES).unwrap();
        assert_eq!(p.size[0], 1920);
        assert_eq!(p.rgba.len(), p.size[0] * p.size[1] * 4);
    }

    #[test]
    fn no_turn_keeps_the_colours() {
        let p = solid([150, 60, 230], 2, 2);
        let t = tint(&p, 0.0, 1.0);
        for (a, b) in p.rgba.iter().zip(&t.rgba) {
            assert!(a.abs_diff(*b) <= 2, "{a} vs {b}");
        }
        let full = tint(&p, 360.0, 1.0);
        for (a, b) in p.rgba.iter().zip(&full.rgba) {
            assert!(a.abs_diff(*b) <= 2, "{a} vs {b}");
        }
    }

    #[test]
    fn no_saturation_is_grey() {
        let t = tint(&solid([150, 60, 230], 1, 1), 40.0, 0.0);
        let [r, g, b] = [t.rgba[0], t.rgba[1], t.rgba[2]];
        assert!(r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1, "{r} {g} {b}");
    }

    #[test]
    fn frosting_shrinks_and_keeps_a_flat_colour() {
        let p = solid([90, 40, 200], 960, 600);
        let f = frosted(&p);
        assert_eq!(f.size, [240, 150]);
        assert!(f
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|px| *px == [90, 40, 200, 255]));
    }
}
