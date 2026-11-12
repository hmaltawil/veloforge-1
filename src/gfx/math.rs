//! Minimal vector and column-major 4x4 matrix helpers for the GPU renderer.

pub type V3 = [f32; 3];
pub type M4 = [f32; 16];

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn len(a: V3) -> f32 {
    dot(a, a).sqrt()
}
pub fn norm(a: V3) -> V3 {
    scale(a, 1.0 / len(a).max(1e-9))
}
pub fn lerp(a: V3, b: V3, t: f32) -> V3 {
    add(a, scale(sub(b, a), t))
}

pub fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> M4 {
    let f = 1.0 / (fovy / 2.0).tan();
    let mut m = [0.0; 16];
    m[0] = f / aspect;
    m[5] = f;
    m[10] = (far + near) / (near - far);
    m[11] = -1.0;
    m[14] = 2.0 * far * near / (near - far);
    m
}

/// Orthographic projection of the box `x` in [l, r], `y` in [b, t], with
/// `near` and `far` measured along the viewing direction.
pub fn ortho(l: f32, r: f32, b: f32, t: f32, near: f32, far: f32) -> M4 {
    let mut m = [0.0; 16];
    m[0] = 2.0 / (r - l);
    m[5] = 2.0 / (t - b);
    m[10] = -2.0 / (far - near);
    m[12] = -(r + l) / (r - l);
    m[13] = -(t + b) / (t - b);
    m[14] = -(far + near) / (far - near);
    m[15] = 1.0;
    m
}

pub fn look_at(eye: V3, target: V3, up: V3) -> M4 {
    let f = norm(sub(target, eye));
    let s = norm(cross(f, up));
    let u = cross(s, f);
    [
        s[0],
        u[0],
        -f[0],
        0.0, //
        s[1],
        u[1],
        -f[1],
        0.0, //
        s[2],
        u[2],
        -f[2],
        0.0, //
        -dot(s, eye),
        -dot(u, eye),
        dot(f, eye),
        1.0,
    ]
}

pub fn mul(a: &M4, b: &M4) -> M4 {
    let mut r = [0.0; 16];
    for c in 0..4 {
        for row in 0..4 {
            r[c * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[c * 4 + k]).sum();
        }
    }
    r
}

pub fn transform(m: &M4, p: [f32; 4]) -> [f32; 4] {
    let mut r = [0.0; 4];
    for (row, v) in r.iter_mut().enumerate() {
        *v = (0..4).map(|k| m[k * 4 + row] * p[k]).sum();
    }
    r
}

pub fn invert(m: &M4) -> Option<M4> {
    // Gauss-Jordan on a row-major copy.
    let mut a = [[0.0f64; 8]; 4];
    for (r, row) in a.iter_mut().enumerate() {
        for c in 0..4 {
            row[c] = m[c * 4 + r] as f64;
        }
        row[4 + r] = 1.0;
    }
    for col in 0..4 {
        let pivot = (col..4).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))?;
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        let p = a[col][col];
        for v in a[col].iter_mut() {
            *v /= p;
        }
        for r in 0..4 {
            if r != col {
                let f = a[r][col];
                if f != 0.0 {
                    let pivot_row = a[col];
                    for (v, pv) in a[r].iter_mut().zip(pivot_row) {
                        *v -= f * pv;
                    }
                }
            }
        }
    }
    let mut out = [0.0; 16];
    for (r, row) in a.iter().enumerate() {
        for c in 0..4 {
            out[c * 4 + r] = row[4 + c] as f32;
        }
    }
    Some(out)
}

/// Screen position (pixels, origin top-left) of a point, if in front of the camera.
pub fn project(vp: &M4, p: V3, width: f32, height: f32) -> Option<(f32, f32, f32)> {
    let c = transform(vp, [p[0], p[1], p[2], 1.0]);
    if c[3] <= 0.1 {
        return None;
    }
    let (x, y) = (c[0] / c[3], c[1] / c[3]);
    Some(((x * 0.5 + 0.5) * width, (0.5 - y * 0.5) * height, c[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_roundtrip() {
        let p = perspective(1.0, 1.5, 0.3, 5000.0);
        let v = look_at([10.0, 5.0, -3.0], [12.0, 4.0, -20.0], [0.0, 1.0, 0.0]);
        let vp = mul(&p, &v);
        let inv = invert(&vp).unwrap();
        let id = mul(&vp, &inv);
        for (i, x) in id.iter().enumerate() {
            let want = if i % 5 == 0 { 1.0 } else { 0.0 };
            assert!((x - want).abs() < 1e-3, "{i}: {x}");
        }
    }

    #[test]
    fn ortho_maps_the_box_to_the_unit_cube() {
        let m = ortho(-10.0, 30.0, -5.0, 5.0, 1.0, 101.0);
        let near = transform(&m, [-10.0, -5.0, -1.0, 1.0]);
        let far = transform(&m, [30.0, 5.0, -101.0, 1.0]);
        for (got, want) in near.iter().zip([-1.0, -1.0, -1.0, 1.0]) {
            assert!((got - want).abs() < 1e-5);
        }
        for (got, want) in far.iter().zip([1.0, 1.0, 1.0, 1.0]) {
            assert!((got - want).abs() < 1e-5);
        }
    }

    #[test]
    fn look_at_puts_target_in_centre() {
        let p = perspective(1.0, 1.0, 0.3, 1000.0);
        let v = look_at([0.0, 2.0, 0.0], [0.0, 2.0, -50.0], [0.0, 1.0, 0.0]);
        let (x, y, _) = project(&mul(&p, &v), [0.0, 2.0, -50.0], 100.0, 100.0).unwrap();
        assert!((x - 50.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3);
        // A point to the right (+x) appears right of centre.
        let (x, _, _) = project(&mul(&p, &v), [5.0, 2.0, -50.0], 100.0, 100.0).unwrap();
        assert!(x > 50.0);
    }
}
