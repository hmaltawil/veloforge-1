//! GLSL sources, written for both GLSL 3.30 core and GLSL ES 3.00 (a
//! version header is prepended at compile time).
//!
//! The scene is lit in linear colour and written to a floating point
//! target; exposure, tonemapping and grading happen once in `COMPOSITE_FS`.
//! Where float targets are missing the scene shaders encode for the screen
//! themselves (`u_encode`).

/// Noise and ground colours, shared by vertex and fragment shaders.
pub const NOISE: &str = r#"
uniform vec2 u_origin;
uniform float u_time;
uniform vec3 u_grass0;
uniform vec3 u_grass1;
uniform vec3 u_grass2;

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}
float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash12(i), hash12(i + vec2(1.0, 0.0)), u.x),
               mix(hash12(i + vec2(0.0, 1.0)), hash12(i + vec2(1.0, 1.0)), u.x), u.y);
}
float fbm(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int i = 0; i < 5; i++) {
        s += a * vnoise(p);
        p = p * 2.03 + vec2(17.1, 9.2);
        a *= 0.5;
    }
    return s / 0.96875;
}
float fbm3(vec2 p) {
    float s = 0.0;
    float a = 0.5;
    for (int i = 0; i < 3; i++) {
        s += a * vnoise(p);
        p = p * 2.03 + vec2(17.1, 9.2);
        a *= 0.5;
    }
    return s / 0.875;
}
// World (east, north) coordinates of a render-space point.
vec2 world_xz(vec3 p) { return vec2(p.x + u_origin.x, -p.z + u_origin.y); }

// Meadow colour from the large-scale noise `n1`: lush, pale and dry patches.
vec3 grass_albedo(float n1) {
    vec3 g = mix(u_grass0, u_grass1, n1);
    return mix(g, u_grass2, smoothstep(0.62, 0.85, n1) * 0.55);
}
"#;

/// Fragment helpers: sky, clouds, shadows, fog, lighting and output.
pub const LIGHT: &str = r#"
uniform vec3 u_cam;
uniform vec3 u_sun;
uniform vec3 u_sun_col;
uniform vec3 u_sky_top;
uniform vec3 u_sky_hor;
uniform vec3 u_sky_anti;
uniform float u_fog;
uniform float u_fog_falloff;
uniform float u_glow;
uniform float u_clouds;
uniform float u_night;
uniform float u_lamp;
uniform vec3 u_lamp_pos;
uniform vec3 u_lamp_dir;
uniform float u_encode;
uniform float u_exposure;
uniform float u_desat;
uniform sampler2DShadow u_sm0;
uniform sampler2DShadow u_sm1;
uniform mat4 u_lvp0;
uniform mat4 u_lvp1;
// Shadow strength, world size of a texel in each cascade, 1 / resolution.
uniform vec4 u_sm;

vec3 sky_color(vec3 d) {
    float up = clamp(d.y, 0.0, 1.0);
    vec3 c = mix(u_sky_hor, u_sky_top, pow(up, 0.5));
    // The zenith is deeper still.
    c = mix(c, u_sky_top * 0.6, smoothstep(0.35, 1.0, up) * 0.6);
    // Opposite a low sun the horizon turns to dusky pink and blue.
    vec2 hd = normalize(d.xz + vec2(1.0e-5));
    vec2 hs = normalize(u_sun.xz + vec2(1.0e-5));
    float away = 0.5 - 0.5 * dot(hd, hs);
    c = mix(c, u_sky_anti, away * away * exp(-up * 5.0) * 0.85);
    float s = max(dot(d, u_sun), 0.0);
    c += u_sun_col * u_glow * (0.06 * pow(s, 4.0) + 0.2 * pow(s, 28.0) + 0.45 * pow(s, 220.0));
    if (d.y < 0.0) c = mix(c, u_sky_hor * 0.8, clamp(-d.y * 4.0, 0.0, 1.0));
    return c;
}

// Cloud cover (0..1) at a world position on the cloud deck.
float cloud_cover(vec2 q) {
    q += u_time * vec2(6.0, 2.0);
    float v = fbm(q * 0.0009) + 0.26 * (fbm3(q * 0.0042 + 7.3) - 0.5);
    return smoothstep(0.57 - u_clouds * 0.43, 0.8 - u_clouds * 0.33, v);
}
// How much sun gets through the cloud deck at `q` (from `cloud_q`).
float cloud_light(vec2 q) {
    float cov = smoothstep(0.57 - u_clouds * 0.43, 0.8 - u_clouds * 0.33, fbm(q * 0.0009));
    return 1.0 - cov * 0.72 * (1.0 - 0.6 * u_clouds);
}
// Where the line from `p` towards the sun meets the drifting cloud deck.
vec2 cloud_q(vec3 p) {
    float t = (u_cam.y + 1400.0 - p.y) / max(u_sun.y, 0.12);
    return world_xz(p + u_sun * t) + u_time * vec2(6.0, 2.0);
}
// `cloud_light` worked out once per frame for the deck above the view
// (`CLOUD_MAP_FS`): centre and half size, or a size of 0 when there is none.
uniform sampler2D u_cloud_map;
uniform vec3 u_cloud_box;
// How much sun reaches the ground at `p` through the drifting clouds.
float cloud_shadow(vec3 p) {
    vec2 q = cloud_q(p);
    if (u_cloud_box.z > 0.0) {
        vec2 uv = (q - u_cloud_box.xy) / (2.0 * u_cloud_box.z) + 0.5;
        if (uv == clamp(uv, 0.0, 1.0)) return texture(u_cloud_map, uv).r;
    }
    return cloud_light(q);
}

float sm_pcf(sampler2DShadow sm, vec3 c, float r) {
    float s = texture(sm, c);
    s += texture(sm, vec3(c.xy + vec2(-0.94, -0.4) * r, c.z));
    s += texture(sm, vec3(c.xy + vec2(0.94, 0.34) * r, c.z));
    s += texture(sm, vec3(c.xy + vec2(-0.3, 0.95) * r, c.z));
    s += texture(sm, vec3(c.xy + vec2(0.34, -0.9) * r, c.z));
    s += texture(sm, vec3(c.xy + vec2(-0.7, 0.55) * r, c.z));
    s += texture(sm, vec3(c.xy + vec2(0.7, -0.5) * r, c.z));
    s += texture(sm, vec3(c.xy + vec2(0.55, 0.8) * r, c.z));
    s += texture(sm, vec3(c.xy + vec2(-0.5, -0.8) * r, c.z));
    return s / 9.0;
}
float sm_far(vec3 p, vec3 n, float k) {
    vec4 c = u_lvp1 * vec4(p + n * u_sm.z * k, 1.0);
    float e = max(abs(c.x), abs(c.y));
    if (e >= 1.0) return 1.0;
    vec3 s = c.xyz * 0.5 + 0.5;
    float v = sm_pcf(u_sm1, vec3(s.xy, clamp(s.z, 0.0, 1.0) - 0.00003), 1.2 * u_sm.w);
    return mix(v, 1.0, smoothstep(0.85, 1.0, e));
}
// Sunlight reaching `p` (0 in shadow): two cascades around the camera.
float sun_shadow(vec3 p, vec3 n) {
    if (u_sm.x <= 0.0) return 1.0;
    float k = 1.0 + 2.0 * (1.0 - clamp(dot(n, u_sun), 0.0, 1.0));
    vec4 c = u_lvp0 * vec4(p + n * u_sm.y * k, 1.0);
    float e = max(abs(c.x), abs(c.y));
    float vis;
    if (e < 1.0) {
        vec3 s = c.xyz * 0.5 + 0.5;
        vis = sm_pcf(u_sm0, vec3(s.xy, clamp(s.z, 0.0, 1.0) - 0.00002), 1.7 * u_sm.w);
        if (e > 0.88) vis = mix(vis, sm_far(p, n, k), (e - 0.88) / 0.12);
    } else {
        vis = sm_far(p, n, k);
    }
    return mix(1.0, vis, u_sm.x);
}
// One-tap version for grass.
float sun_shadow_fast(vec3 p) {
    if (u_sm.x <= 0.0) return 1.0;
    vec4 c = u_lvp0 * vec4(p, 1.0);
    float vis = 1.0;
    if (max(abs(c.x), abs(c.y)) < 1.0) {
        vec3 s = c.xyz * 0.5 + 0.5;
        vis = texture(u_sm0, vec3(s.xy, clamp(s.z, 0.0, 1.0) - 0.00003));
    } else {
        c = u_lvp1 * vec4(p, 1.0);
        float e = max(abs(c.x), abs(c.y));
        if (e < 1.0) {
            vec3 s = c.xyz * 0.5 + 0.5;
            vis = mix(texture(u_sm1, vec3(s.xy, clamp(s.z, 0.0, 1.0) - 0.00006)), 1.0,
                      smoothstep(0.85, 1.0, e));
        }
    }
    return mix(1.0, vis, u_sm.x);
}
float sun_vis(vec3 p, vec3 n) { return sun_shadow(p, n) * cloud_shadow(p); }

// Sky light from above, light bounced off the ground from below.
vec3 ambient(vec3 n) {
    vec3 sky = mix(u_sky_hor, u_sky_top, 0.55) * 0.8;
    vec3 ground = (u_sky_hor * 0.3 + u_sun_col * max(u_sun.y, 0.0) * 0.16) * vec3(0.6, 0.64, 0.42);
    return mix(ground, sky, n.y * 0.5 + 0.5);
}
vec3 shade(vec3 albedo, vec3 n, float ao, float vis) {
    float ndl = max(dot(n, u_sun), 0.0);
    return albedo * (u_sun_col * ndl * vis * 1.45 + ambient(n) * ao);
}
float fog_amount(vec3 p) {
    float dist = length(p - u_cam);
    // Haze is thicker low down and thinner higher up.
    float density = u_fog * exp(-max(p.y - u_cam.y + 30.0, 0.0) * u_fog_falloff);
    return 1.0 - exp(-dist * density);
}
vec3 apply_fog(vec3 col, vec3 p) {
    vec3 v = p - u_cam;
    float dist = length(v);
    vec3 d = v / max(dist, 0.001);
    vec3 fc = sky_color(normalize(vec3(d.x, max(d.y, 0.0) * 0.25 + 0.015, d.z)));
    return mix(col, fc, clamp(fog_amount(p), 0.0, 1.0));
}
// Bike headlight: a forward cone from just in front of the rider.
vec3 lamp_light(vec3 p, vec3 n) {
    if (u_lamp <= 0.0) return vec3(0.0);
    vec3 l = p - u_lamp_pos;
    float d = length(l);
    vec3 ld = l / max(d, 0.001);
    float cone = smoothstep(0.9, 0.99, dot(ld, u_lamp_dir));
    float ndl = max(dot(n, -ld), 0.0);
    // The beam is aimed down the road, so the ground right in front stays dim.
    float reach = smoothstep(1.5, 9.0, d);
    return vec3(1.0, 0.93, 0.8) * u_lamp * cone * ndl * reach * 900.0 / (30.0 + d * d);
}
vec3 tonemap(vec3 c) {
    c *= u_exposure;
    c = mix(c, vec3(dot(c, vec3(0.3, 0.6, 0.1))), u_desat);
    c = (c * (2.51 * c + 0.03)) / (c * (2.43 * c + 0.59) + 0.14);
    return pow(clamp(c, 0.0, 1.0), vec3(1.0 / 2.2));
}
vec4 out_col(vec3 c) {
    if (u_encode > 0.5) return vec4(tonemap(c), 1.0);
    return vec4(min(c, vec3(64.0)), 1.0);
}
"#;

pub const SCREEN_VS: &str = r#"
out vec2 v_ndc;
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)) * 2.0 - 1.0;
    v_ndc = p;
    v_uv = p * 0.5 + 0.5;
    gl_Position = vec4(p, 1.0, 1.0);
}
"#;

pub const SKY_FS: &str = r#"
uniform mat4 u_inv_vp;
uniform float u_stars;
uniform float u_moon;
uniform vec3 u_cloud_lit;
uniform vec3 u_cloud_dark;
uniform vec3 u_disk;
in vec2 v_ndc;
in vec2 v_uv;
out vec4 frag;

float star_layer(vec3 d, float scale, float density) {
    vec2 sp = vec2(atan(d.x, d.z), asin(clamp(d.y, -1.0, 1.0))) * scale;
    vec2 cell = floor(sp);
    float h = hash12(cell);
    if (h < density) return 0.0;
    vec2 off = vec2(hash12(cell + 1.3), hash12(cell + 2.7)) - 0.5;
    vec2 f = fract(sp) - 0.5 - off * 0.6;
    float tw = 0.7 + 0.3 * sin(u_time * (1.5 + 3.0 * h) + h * 400.0);
    return smoothstep(0.2, 0.0, length(f)) * tw * (h - density) / (1.0 - density);
}

void main() {
    vec4 a = u_inv_vp * vec4(v_ndc, -1.0, 1.0);
    vec4 b = u_inv_vp * vec4(v_ndc, 1.0, 1.0);
    vec3 d = normalize(b.xyz / b.w - a.xyz / a.w);
    vec3 c = sky_color(d);
    float s = dot(d, u_sun);
    if (d.y > 0.0) {
        float rise = smoothstep(0.0, 0.2, d.y);
        if (u_stars > 0.0) {
            // The Milky Way: a faint, dusty band of light across the sky.
            float band = exp(-pow(dot(d, normalize(vec3(0.5, 0.45, -0.74))) * 4.0, 2.0));
            float dust = fbm(vec2(atan(d.x, d.z) * 6.0, d.y * 9.0));
            c += vec3(0.5, 0.56, 0.75) * band * (0.35 + 0.9 * dust * dust) * 0.035 * u_stars * rise;
            float st = star_layer(d, 150.0, 0.982) * 1.6 + star_layer(d, 310.0, 0.975 - 0.02 * band) * 0.55;
            vec3 tint = mix(vec3(1.0, 0.85, 0.7), vec3(0.75, 0.86, 1.0), hash12(floor(d.xz * 400.0)));
            c += tint * st * u_stars * rise * 0.9;
        }
        // The sun or moon sits behind the clouds.
        if (u_moon > 0.5) {
            vec3 t1 = normalize(cross(u_sun, vec3(0.0, 1.0, 0.0)));
            vec3 t2 = cross(u_sun, t1);
            vec2 m = vec2(dot(d, t1), dot(d, t2)) / 0.028;
            float r = length(m);
            if (s > 0.0 && r < 1.0) {
                float limb = sqrt(max(1.0 - r * r, 0.0));
                float maria = smoothstep(0.42, 0.62, fbm(m * 2.3 + 4.0));
                c += u_disk * (0.55 + 0.45 * limb) * (1.0 - 0.45 * maria) * smoothstep(1.0, 0.94, r);
            }
            c += u_disk * 0.06 * pow(max(s, 0.0), 900.0);
        } else {
            c += u_disk * smoothstep(0.99955, 0.9998, s);
        }
        // Clouds on a deck 1.4 km up, drifting with the wind.
        float t = 1400.0 / max(d.y, 0.02);
        vec2 q = world_xz(u_cam + d * t);
        float far = exp(-t * 0.00007) * smoothstep(0.0, 0.07, d.y);
        // Thin, high cirrus.
        float ci = fbm(vec2(q.x * 0.00011, q.y * 0.00042) + u_time * 0.0015);
        ci = smoothstep(0.5, 0.95, ci) * 0.4 * (1.0 - u_clouds) * far;
        c = mix(c, u_cloud_lit * 0.8 + c * 0.25, ci);
        float cov = cloud_cover(q);
        if (cov > 0.0) {
            // More cloud between here and the sun means a darker underside.
            vec2 sw = normalize(vec2(u_sun.x, -u_sun.z) + vec2(1.0e-5));
            float toward = cloud_cover(q + sw * 240.0);
            float thick = fbm3(q * 0.003 + 5.0);
            float dark = clamp(0.2 + 0.75 * (toward - 0.25) + 0.5 * smoothstep(0.4, 0.9, thick) * cov, 0.0, 1.0);
            vec3 cc = mix(u_cloud_lit, u_cloud_dark, dark);
            // Silver lining around the sun.
            cc += u_sun_col * u_glow * 0.7 * pow(max(s, 0.0), 8.0) * (1.0 - cov * 0.8);
            cc = mix(sky_color(normalize(vec3(d.x, 0.03, d.z))), cc, far);
            c = mix(c, cc, cov * smoothstep(0.0, 0.05, d.y));
        }
    }
    frag = out_col(c);
}
"#;

pub const TERRAIN_VS: &str = r#"
in vec3 a_pos;
in vec3 a_nrm;
in float a_dist;
in float a_forest;
uniform mat4 u_vp;
out vec3 v_pos;
out vec3 v_nrm;
out float v_dist;
out float v_forest;
void main() {
    v_pos = a_pos;
    v_nrm = a_nrm;
    v_dist = a_dist;
    v_forest = a_forest;
    gl_Position = u_vp * vec4(a_pos, 1.0);
}
"#;

pub const TERRAIN_FS: &str = r#"
uniform int u_scenery;   // 0 hills, 1 lake, 2 alpine, 3 coast
uniform float u_water;   // water level, or a large negative number
uniform float u_snow;    // snow line (alpine)
uniform float u_wet;
in vec3 v_pos;
in vec3 v_nrm;
in float v_dist;
in float v_forest;
out vec4 frag;
void main() {
    vec3 n = normalize(v_nrm);
    vec2 w = world_xz(v_pos);
    float dcam = length(v_pos - u_cam);
    // Fine detail fades out with distance so it does not shimmer.
    float detail = 1.0 - smoothstep(40.0, 260.0, dcam);
    float near = 1.0 - smoothstep(10.0, 90.0, dcam);
    float n1 = fbm(w * 0.012);
    // Far away, the finest noise is smaller than a pixel: skip it.
    float n2 = detail > 0.0 ? mix(0.5, fbm(w * 0.11), 0.4 + 0.6 * detail)
                            : mix(0.5, fbm3(w * 0.11), 0.4);
    float n3 = detail > 0.0 ? mix(0.5, vnoise(w * 1.7), detail) : 0.5;
    float n4 = near > 0.0 ? mix(0.5, vnoise(w * 7.0), near) : 0.5;
    vec3 grass = grass_albedo(n1) * (0.76 + 0.36 * n2 + 0.1 * n3) * (0.88 + 0.24 * n4);
    // Wild flowers scattered through the meadows.
    if (near > 0.0) {
        vec2 fc = floor(w * 2.4);
        float fh = hash12(fc);
        if (fh > 0.93) {
            vec2 ff = fract(w * 2.4) - 0.5 - (vec2(hash12(fc + 1.7), hash12(fc + 5.1)) - 0.5) * 0.6;
            float patchy = smoothstep(0.45, 0.7, fbm3(w * 0.021 + 9.0));
            vec3 fcol = fh > 0.975 ? vec3(0.7, 0.68, 0.62) : (fh > 0.952 ? vec3(0.7, 0.5, 0.04) : vec3(0.36, 0.16, 0.5));
            grass = mix(grass, fcol, smoothstep(0.17, 0.07, length(ff)) * near * patchy);
        }
    }
    vec3 col = grass;

    if (u_scenery < 2) {
        // Farmland: a patchwork of fields with crop rows and hedges.
        float flat_ground = smoothstep(0.93, 0.985, n.y);
        float open_land = 1.0 - smoothstep(0.12, 0.35, v_forest);
        float mask = flat_ground * open_land * smoothstep(28.0, 50.0, v_dist);
        vec2 q = mat2(0.94, 0.34, -0.34, 0.94) * w / vec2(150.0, 105.0);
        vec2 cell = floor(q);
        vec2 f = fract(q);
        float id = hash12(cell + 3.7);
        vec3 crop = id < 0.2 ? vec3(0.42, 0.33, 0.08)       // ripe wheat
                  : id < 0.32 ? vec3(0.5, 0.44, 0.03)       // rapeseed
                  : id < 0.46 ? vec3(0.085, 0.2, 0.03)      // young green
                  : id < 0.58 ? vec3(0.17, 0.11, 0.065)     // ploughed
                  : id < 0.74 ? vec3(0.14, 0.24, 0.05)      // pasture
                  : grass;
        float along = id > 0.5 ? q.x * 150.0 : q.y * 105.0;
        float rows = 0.84 + 0.16 * sin(along * 1.6) * detail;
        crop *= rows * (0.86 + 0.28 * n2);
        float edge = min(min(f.x, 1.0 - f.x) * 150.0, min(f.y, 1.0 - f.y) * 105.0);
        crop = mix(vec3(0.028, 0.065, 0.014) * (0.7 + 0.6 * n3), crop, smoothstep(1.5, 4.5, edge));
        col = mix(col, crop, mask);
    }
    // Forest floor: needles, moss and leaf litter.
    vec3 floor_col = mix(vec3(0.03, 0.06, 0.018), vec3(0.075, 0.06, 0.03), n2) * (0.8 + 0.4 * n3);
    col = mix(col, floor_col, smoothstep(0.15, 0.55, v_forest));
    // Rock on steep slopes, in tilted layers.
    float rock = smoothstep(0.8, 0.62, n.y);
    float strata = vnoise(vec2(dot(w, vec2(0.02, 0.013)), v_pos.y * 0.45 + n2 * 2.0));
    vec3 rockc = mix(vec3(0.2, 0.185, 0.165), vec3(0.1, 0.098, 0.097), strata) * (0.75 + 0.5 * n2);
    if (u_scenery == 2) {
        rock = max(rock, smoothstep(u_snow - 220.0, u_snow - 90.0, v_pos.y + n1 * 70.0) * 0.85);
    }
    col = mix(col, rockc, rock);
    float gloss = 0.0;
    if (u_scenery == 2) {
        float snow = smoothstep(u_snow - 15.0, u_snow + 25.0, v_pos.y + n1 * 90.0) * smoothstep(0.5, 0.75, n.y);
        col = mix(col, vec3(0.86, 0.89, 0.95), snow);
        gloss = snow;
    }
    // Sand and mud at the waterline; wide pale beaches by the sea.
    if (u_scenery == 3) {
        float beach = (1.0 - smoothstep(1.5, 3.5, v_pos.y - u_water + n1 * 1.5)) * smoothstep(0.75, 0.9, n.y);
        float ripples = 0.9 + 0.1 * sin(dot(w, vec2(0.9, 0.5)) * 3.0 + n2 * 6.0) * near;
        col = mix(col, vec3(0.52, 0.44, 0.3) * (0.85 + 0.25 * n3) * ripples, beach);
        // Wet sand where the waves reach.
        col *= 1.0 - 0.45 * (1.0 - smoothstep(0.0, 0.7, v_pos.y - u_water));
    } else {
        float shore = 1.0 - smoothstep(0.0, 1.2, v_pos.y - u_water);
        col = mix(col, vec3(0.2, 0.16, 0.1) * (0.8 + 0.3 * n3), shore);
    }
    // Under water the bed is darker and bluer.
    float below = smoothstep(0.0, -3.0, v_pos.y - u_water);
    col = mix(col, col * vec3(0.5, 0.75, 0.8), below);
    col *= 1.0 - 0.22 * u_wet;

    vec3 nb = normalize(n + vec3(n2 - 0.5, 0.0, n3 - 0.5) * 0.45 * detail);
    float ao = 1.0 - 0.5 * smoothstep(0.3, 0.8, v_forest);
    float vis = sun_vis(v_pos, n);
    vec3 lit = shade(col, nb, ao, vis) + col * lamp_light(v_pos, n);
    // Snow glitters towards the sun.
    vec3 view = normalize(u_cam - v_pos);
    vec3 hv = normalize(view + u_sun);
    lit += u_sun_col * vis * gloss * 0.35 * pow(max(dot(nb, hv), 0.0), 24.0);
    frag = out_col(apply_fog(lit, v_pos));
}
"#;

pub const ROAD_VS: &str = r#"
in vec3 a_pos;
in vec2 a_uv;
uniform mat4 u_vp;
out vec3 v_pos;
out vec2 v_uv;
void main() {
    v_pos = a_pos;
    v_uv = a_uv;
    gl_Position = u_vp * vec4(a_pos, 1.0);
}
"#;

pub const ROAD_FS: &str = r#"
uniform float u_wet;
uniform float u_snow;
in vec3 v_pos;
in vec2 v_uv;
out vec4 frag;
void main() {
    float u = v_uv.x;
    float a = abs(u);
    float v = v_uv.y;
    float dcam = length(v_pos - u_cam);
    float detail = 1.0 - smoothstep(25.0, 140.0, dcam);
    float near = 1.0 - smoothstep(6.0, 45.0, dcam);
    float g = fbm(vec2(u * 1.5, v * 0.4));
    float fine = mix(0.5, vnoise(vec2(u, v) * 14.0), detail);
    // Stone chips in the tarmac.
    float chips = mix(0.5, vnoise(vec2(u, v) * 42.0), near);
    vec3 asphalt = vec3(0.03, 0.032, 0.036) * (0.76 + 0.3 * g + 0.16 * fine + 0.2 * (chips - 0.5));
    // Polished wheel tracks, darker repair patches and tar-filled cracks.
    asphalt *= 1.0 - 0.16 * exp(-pow((a - 1.6) / 0.4, 2.0));
    asphalt = mix(asphalt, asphalt * 0.62, smoothstep(0.66, 0.7, fbm(vec2(u * 0.25, v * 0.04) + 3.0)));
    float crack = abs(fbm3(vec2(u * 0.9, v * 0.22) + 21.0) - 0.5);
    asphalt *= 1.0 - 0.45 * (1.0 - smoothstep(0.0, 0.0045, crack)) * detail * smoothstep(0.52, 0.7, fbm3(vec2(u * 0.1, v * 0.02)));
    vec3 col = asphalt;
    float aa = fwidth(u) * 1.5;
    float edge = smoothstep(2.86 - aa, 2.86, a) * (1.0 - smoothstep(3.0, 3.0 + aa, a));
    float dash = step(fract(v / 12.0), 0.42);
    float centre = (1.0 - smoothstep(0.065, 0.065 + aa, a)) * dash;
    // Paint wears thin where tyres run over it.
    float wear = 0.75 + 0.25 * smoothstep(0.3, 0.6, fbm3(vec2(u * 3.0, v * 0.5) + 2.0));
    vec3 paint = vec3(0.66, 0.66, 0.63) * (0.85 + 0.2 * fine);
    float painted = max(edge, centre) * wear;
    col = mix(col, paint, painted);
    vec3 gravel = vec3(0.11, 0.098, 0.078) * (0.6 + 0.5 * fine + 0.5 * (chips - 0.5));
    float shoulder = smoothstep(3.2, 3.35, a);
    col = mix(col, gravel, shoulder);
    // The verge takes the colour of the meadow beside it.
    vec2 w = world_xz(v_pos);
    float n1 = fbm(w * 0.012);
    vec3 verge = grass_albedo(n1) * (0.72 + 0.4 * g + 0.2 * fine);
    // High up the verge lies under snow, ploughed back from the tarmac.
    float snow = smoothstep(u_snow - 15.0, u_snow + 25.0, v_pos.y + n1 * 90.0);
    verge = mix(verge, vec3(0.86, 0.89, 0.95), snow * smoothstep(3.75, 4.3, a));
    float grassy = smoothstep(3.7 + 0.3 * g, 4.25, a);
    col = mix(col, verge, grassy);
    float tarmac = 1.0 - smoothstep(3.0, 3.3, a);
    // Rain darkens the surface and leaves puddles that mirror the sky.
    float puddle = u_wet * tarmac * smoothstep(0.58, 0.7, fbm(vec2(u * 0.7, v * 0.09) + 11.0));
    col *= 1.0 - 0.42 * u_wet * (1.0 - smoothstep(3.6, 4.2, a));

    vec3 up = vec3(0.0, 1.0, 0.0);
    vec3 n = normalize(vec3((g - 0.5) * 0.1 * tarmac, 1.0, (fine - 0.5) * 0.06 * tarmac));
    float vis = sun_vis(v_pos, up);
    vec3 lit = shade(col, up, 1.0, vis) + col * lamp_light(v_pos, up);
    vec3 view = normalize(u_cam - v_pos);
    // Low sun glares off the road surface.
    vec3 h = normalize(view + u_sun);
    float fres = 0.04 + 0.96 * pow(1.0 - max(dot(view, up), 0.0), 5.0);
    float rough = mix(60.0, 600.0, u_wet);
    float spec = pow(max(dot(n, h), 0.0), rough);
    lit += u_sun_col * vis * spec * fres * (1.1 + 7.0 * u_wet) * (tarmac * (1.0 - 0.5 * painted) + 0.1);
    // The headlight catches the paint.
    lit += paint * lamp_light(v_pos, up) * painted * 1.5;
    if (u_wet > 0.0) {
        vec2 rip = vec2(vnoise(vec2(u, v) * 2.5 + u_time * 0.5), vnoise(vec2(v, u) * 2.5 - u_time * 0.4)) - 0.5;
        // Rings spreading where drops land.
        vec2 rc = fract(vec2(u, v) * 1.3) - 0.5;
        float ph = fract(u_time * 1.4 + hash12(floor(vec2(u, v) * 1.3)) * 7.0);
        float ring = sin(clamp((length(rc) - ph * 0.45) * 40.0, -3.14, 3.14)) * (1.0 - ph) * near;
        vec3 nrm = normalize(vec3(rip.x * 0.1 * (1.0 - puddle * 0.7) + ring * rc.x * 0.6, 1.0, rip.y * 0.1 + ring * rc.y * 0.6));
        vec3 r = reflect(-view, nrm);
        r.y = abs(r.y);
        float fr = 0.04 + 0.96 * pow(1.0 - max(dot(view, nrm), 0.0), 5.0);
        lit = mix(lit, sky_color(r), clamp(fr * (0.55 * u_wet * tarmac + 0.6 * puddle) + 0.25 * puddle, 0.0, 1.0));
    }
    frag = out_col(apply_fog(lit, v_pos));
}
"#;

pub const OBJECT_VS: &str = r#"
in vec3 a_pos;
in vec3 a_nrm;
in vec3 a_col;
in float a_mat;
in vec4 i_pos_rot;
in vec2 i_scale;
in vec3 i_tint;
uniform mat4 u_vp;
uniform float u_obj_time;
uniform float u_wind;
out vec3 v_pos;
out vec3 v_nrm;
out vec3 v_col;
out float v_mat;
out float v_h;
void main() {
    float c = cos(i_pos_rot.w);
    float s = sin(i_pos_rot.w);
    vec3 p = vec3(a_pos.x * i_scale.x, a_pos.y * i_scale.y, a_pos.z * i_scale.x);
    p = vec3(c * p.x + s * p.z, p.y, -s * p.x + c * p.z);
    // Trees sway gently, more towards the top, and their leaves flutter.
    float ph = i_pos_rot.x * 0.13 + i_pos_rot.z * 0.07;
    float sway = u_wind * a_pos.y * a_pos.y * (sin(u_obj_time * 1.1 + ph) + 0.4 * sin(u_obj_time * 2.3 + ph * 1.7));
    p.x += sway * i_scale.y * 0.011;
    p.z += sway * i_scale.y * 0.007;
    vec3 n = vec3(c * a_nrm.x + s * a_nrm.z, a_nrm.y, -s * a_nrm.x + c * a_nrm.z);
    n = normalize(n / vec3(i_scale.x, i_scale.y, i_scale.x));
    if (a_mat > 4.5 && u_wind > 0.0) {
        float fl = sin(u_obj_time * 3.1 + a_pos.x * 37.0 + a_pos.z * 29.0 + ph * 3.0)
                 + sin(u_obj_time * 4.7 + a_pos.y * 41.0 + ph);
        p += n * fl * 0.011 * i_scale.x * u_wind;
    }
    v_pos = p + i_pos_rot.xyz;
    v_nrm = n;
    v_col = a_col.r < 0.0 ? -a_col * i_tint : a_col;
    v_mat = a_mat;
    v_h = a_pos.y;
    gl_Position = u_vp * vec4(v_pos, 1.0);
}
"#;

/// Materials (`a_mat`): 0 matt, 1 window (lit at night), 2 lamp, 3 glossy,
/// 4 reflector, 5 leaves.
pub const OBJECT_FS: &str = r#"
in vec3 v_pos;
in vec3 v_nrm;
in vec3 v_col;
in float v_mat;
in float v_h;
out vec4 frag;
void main() {
    vec3 n = normalize(v_nrm);
    vec3 view = normalize(u_cam - v_pos);
    float vis = sun_vis(v_pos, n);
    vec3 lamp = lamp_light(v_pos, n);
    vec3 lit;
    if (v_mat > 4.5) {
        // Leaves: light wraps round the crown and glows through it.
        float wrap = max((dot(n, u_sun) + 0.45) / 1.45, 0.0);
        lit = v_col * (u_sun_col * wrap * vis * 1.45 + ambient(n));
        float back = pow(max(dot(view, -u_sun), 0.0), 3.0);
        lit += v_col * vec3(1.2, 1.35, 0.5) * u_sun_col * vis * back * 0.7;
        lit += v_col * lamp;
    } else {
        lit = shade(v_col, n, 1.0, vis) + v_col * lamp;
        vec3 h = normalize(view + u_sun);
        float gloss = v_mat > 2.5 && v_mat < 3.5 ? 1.0 : 0.0;
        float fres = 0.04 + 0.96 * pow(1.0 - max(dot(view, n), 0.0), 5.0);
        lit += u_sun_col * vis * (0.06 + gloss * 0.9) * pow(max(dot(n, h), 0.0), 30.0 + gloss * 90.0);
        lit += sky_color(reflect(-view, n)) * fres * gloss * 0.35;
        if (v_mat > 0.5 && v_mat < 1.5) {
            // Windows mirror the sky by day and glow warm after dark.
            lit = mix(lit, sky_color(reflect(-view, n)), 0.35 * (1.0 - u_night));
            lit += vec3(1.0, 0.62, 0.25) * u_night * 0.9;
        } else if (v_mat > 1.5 && v_mat < 2.5) {
            lit += v_col * (1.5 + 10.0 * u_night);
        } else if (v_mat > 3.5) {
            // Reflectors throw the headlight straight back.
            lit += v_col * lamp * 14.0;
        }
    }
    frag = out_col(apply_fog(lit, v_pos));
}
"#;

/// Depth-only fragment shader for the shadow maps.
pub const DEPTH_FS: &str = r#"
void main() {}
"#;

pub const BLOB_VS: &str = r#"
in vec2 a_corner;
in vec4 i_shadow;   // centre (3), radius
uniform mat4 u_vp;
out vec2 v_uv;
out vec3 v_pos;
void main() {
    v_uv = a_corner;
    vec3 p = i_shadow.xyz + vec3(a_corner.x, 0.0, a_corner.y) * i_shadow.w;
    v_pos = p;
    gl_Position = u_vp * vec4(p, 1.0);
}
"#;

/// Soft darkening under trees and riders, where little sky light reaches.
pub const BLOB_FS: &str = r#"
uniform float u_strength;
in vec2 v_uv;
in vec3 v_pos;
out vec4 frag;
void main() {
    float r = dot(v_uv, v_uv);
    float a = (1.0 - smoothstep(0.1, 1.0, r)) * u_strength * (1.0 - fog_amount(v_pos));
    frag = vec4(0.0, 0.0, 0.0, a);
}
"#;

pub const GRASS_VS: &str = r#"
in vec3 a_pos;
in vec3 a_col;
in vec4 i_seed;   // position in the patch (2), rotation, random
uniform mat4 u_vp;
uniform vec3 u_cam;
uniform vec2 u_centre;
uniform float u_patch;
uniform sampler2D u_height;   // height, road distance, forest
uniform vec3 u_hmap;          // half size, cell size, cells per side
uniform float u_water;
uniform float u_snow;
uniform float u_gust;
out vec3 v_col;
out vec3 v_pos;
out vec3 v_base;
out float v_h;

vec4 ground(vec2 xz) {
    vec2 g = vec2(xz.x + u_hmap.x, u_hmap.x - xz.y) / u_hmap.y;
    g = clamp(g, vec2(0.0), vec2(u_hmap.z - 1.001));
    ivec2 i = ivec2(floor(g));
    vec2 f = fract(g);
    vec4 a = texelFetch(u_height, i, 0);
    vec4 b = texelFetch(u_height, i + ivec2(1, 0), 0);
    vec4 c = texelFetch(u_height, i + ivec2(0, 1), 0);
    vec4 d = texelFetch(u_height, i + ivec2(1, 1), 0);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

void main() {
    // The patch follows the camera; tufts wrap round as it moves.
    vec2 rel = mod(i_seed.xy - u_centre, u_patch) - u_patch * 0.5;
    vec2 xz = u_centre + rel;
    vec4 g = ground(xz);
    vec4 gx = ground(xz + vec2(2.0, 0.0));
    vec4 gz = ground(xz + vec2(0.0, 2.0));
    float steep = length(vec2(gx.x - g.x, gz.x - g.x)) * 0.5;
    float rnd = i_seed.w;
    float size = (0.55 + 0.9 * rnd);
    // No grass on the road, in water, on rock or snow, or far away.
    size *= smoothstep(u_patch * 0.5, u_patch * 0.34, length(rel));
    size *= smoothstep(4.1, 6.5, g.y) * (0.45 + 0.55 * smoothstep(5.0, 14.0, g.y));
    size *= smoothstep(u_water + 0.5, u_water + 1.6, g.x);
    size *= 1.0 - smoothstep(0.45, 0.7, steep);
    size *= 1.0 - 0.6 * smoothstep(0.2, 0.7, g.z);
    // Beside the road the ground rises to meet the tarmac.
    float y = g.x + 0.27 * (1.0 - smoothstep(4.6, 5.8, g.y)) - 0.03;
    float c = cos(i_seed.z);
    float s = sin(i_seed.z);
    vec2 w = vec2(xz.x + u_origin.x, -xz.y + u_origin.y);
    float n1 = fbm(w * 0.012);
    // Only a few tips show through the snow.
    size *= 1.0 - 0.93 * smoothstep(u_snow - 45.0, u_snow - 5.0, g.x + n1 * 90.0);
    // Little grows on the scree below the snow line.
    size *= 1.0 - 0.85 * smoothstep(u_snow - 210.0, u_snow - 110.0, g.x + n1 * 70.0);
    vec3 p = a_pos * vec3(0.5, 0.42, 0.5) * vec3(1.0, size, 1.0);
    p = vec3(c * p.x + s * p.z, p.y, -s * p.x + c * p.z);
    // Wind runs across the meadow in waves.
    float wave = sin(u_time * 1.7 + w.x * 0.21 + w.y * 0.13) + 0.6 * sin(u_time * 2.9 + w.x * 0.47 - w.y * 0.31 + rnd * 6.0);
    float bend = a_pos.y * a_pos.y * (0.07 + 0.09 * u_gust) * wave;
    p.x += bend;
    p.z += bend * 0.6;
    vec3 base = vec3(xz.x, y, xz.y);
    v_base = base + vec3(0.0, 0.08, 0.0);
    v_pos = base + (size > 0.01 ? p : vec3(0.0));
    v_h = a_pos.y;
    vec3 blade = grass_albedo(n1) * (0.8 + 0.4 * vnoise(w * 0.11)) * (0.82 + 0.36 * rnd);
    if (a_col.r < -1.5) {
        // Flower heads: the kind is chosen per tuft.
        v_col = rnd < 0.3 ? vec3(0.6, 0.59, 0.55) : (rnd < 0.58 ? vec3(0.62, 0.43, 0.03) : (rnd < 0.82 ? vec3(0.33, 0.16, 0.5) : vec3(0.55, 0.05, 0.035)));
    } else if (a_col.r < 0.0) {
        // Darker at the root, sunlit at the tip.
        v_col = blade * (-a_col) * mix(0.55, 1.12, a_pos.y);
    } else {
        v_col = a_col;
    }
    gl_Position = u_vp * vec4(v_pos, 1.0);
}
"#;

pub const GRASS_FS: &str = r#"
in vec3 v_col;
in vec3 v_pos;
in vec3 v_base;
in float v_h;
out vec4 frag;
void main() {
    vec3 up = vec3(0.0, 1.0, 0.0);
    float vis = sun_shadow_fast(v_base) * cloud_shadow(v_base);
    vec3 lit = shade(v_col, normalize(vec3(u_sun.x * 0.2, 1.0, u_sun.z * 0.2)), 0.75 + 0.25 * v_h, vis);
    // Blades glow when the sun is behind them.
    vec3 view = normalize(u_cam - v_pos);
    lit += v_col * vec3(1.2, 1.3, 0.45) * u_sun_col * vis * pow(max(dot(view, -u_sun), 0.0), 3.0) * 0.5 * v_h;
    lit += v_col * lamp_light(v_pos, up);
    frag = out_col(apply_fog(lit, v_pos));
}
"#;

pub const RAIN_VS: &str = r#"
in vec2 a_corner;
in vec4 i_seed;
uniform mat4 u_vp;
uniform vec3 u_cam;
uniform vec3 u_cam_vel;
out float v_x;
out float v_fade;
void main() {
    vec3 box = vec3(36.0, 26.0, 36.0);
    vec3 vel = vec3(-2.2, -(10.0 + 5.0 * i_seed.w), 1.1);
    vec3 p0 = i_seed.xyz * box + vel * u_time;
    vec3 rel = mod(p0 - u_cam, box) - box * 0.5;
    vec3 p = u_cam + rel;
    // Streaks follow the drop's motion as seen from the moving camera.
    vec3 dir = normalize(vel - u_cam_vel);
    float dist = length(rel);
    vec3 side = normalize(cross(dir, rel)) * max(0.004, dist * 0.0011);
    p += dir * a_corner.y * (0.35 + 0.25 * i_seed.w) + side * a_corner.x;
    v_x = a_corner.x;
    v_fade = smoothstep(18.0, 12.0, dist) * smoothstep(0.4, 1.5, dist);
    gl_Position = u_vp * vec4(p, 1.0);
}
"#;

pub const RAIN_FS: &str = r#"
in float v_x;
in float v_fade;
out vec4 frag;
void main() {
    float a = (1.0 - abs(v_x)) * v_fade * 0.3;
    vec3 c = (u_sky_hor * 0.9 + u_sun_col * 0.25) * 1.2 + vec3(0.03) * u_night;
    if (u_encode > 0.5) c = tonemap(c);
    frag = vec4(c, a);
}
"#;

pub const WATER_VS: &str = r#"
in vec3 a_pos;
uniform mat4 u_vp;
out vec3 v_pos;
void main() {
    v_pos = a_pos;
    gl_Position = u_vp * vec4(a_pos, 1.0);
}
"#;

pub const WATER_FS: &str = r#"
uniform sampler2D u_scene;
uniform sampler2D u_depth;
uniform mat4 u_inv_vp;
uniform vec2 u_res;
uniform float u_has_scene;
uniform float u_sea;
uniform vec3 u_cloud_lit;
in vec3 v_pos;
out vec4 frag;

vec3 behind(vec2 uv) {
    float z = texture(u_depth, uv).r;
    vec4 p = u_inv_vp * vec4(uv * 2.0 - 1.0, z * 2.0 - 1.0, 1.0);
    return p.xyz / p.w;
}

void main() {
    vec2 w = world_xz(v_pos);
    vec3 vv = v_pos - u_cam;
    float dist = length(vv);
    vec3 vdir = vv / dist;
    // Ripples flatten out with distance so the far water stays calm.
    float calm = 1.0 / (1.0 + dist * 0.004);
    float swell = mix(0.7, 1.6, u_sea);
    vec2 g = vec2(fbm(w * 0.07 + vec2(u_time * 0.07, 0.0)), fbm(w * 0.07 + vec2(7.0, -u_time * 0.06))) - 0.5;
    g += 0.55 * (vec2(vnoise(w * 0.55 + u_time * 0.45), vnoise(w * 0.55 - u_time * 0.4 + 3.0)) - 0.5);
    g += 0.3 * (vec2(vnoise(w * 2.3 - u_time * 0.9), vnoise(w * 2.3 + u_time * 0.8 + 5.0)) - 0.5) * calm;
    // Long swell rolling in from the open sea.
    g += u_sea * 0.25 * vec2(cos(dot(w, vec2(0.05, 0.03)) + u_time * 0.8), cos(dot(w, vec2(-0.03, 0.06)) + u_time * 0.6));
    vec3 n = normalize(vec3(g.x * 0.3 * swell, 1.0, g.y * 0.3 * swell));
    n = normalize(mix(vec3(0.0, 1.0, 0.0), n, 0.25 + 0.75 * calm));

    vec3 deep = mix(vec3(0.008, 0.045, 0.055), vec3(0.004, 0.035, 0.07), u_sea);
    vec3 body = deep * (0.35 + 1.3 * max(u_sun.y, 0.0) * length(u_sun_col) * 0.5 + 0.6 * length(u_sky_top));
    float foam = 0.0;
    if (u_has_scene > 0.5) {
        vec2 uv = gl_FragCoord.xy / u_res;
        vec3 bed = behind(uv);
        float depth = max(v_pos.y - bed.y, 0.0);
        // Refraction: look through the surface a little to the side.
        vec2 ruv = uv + n.xz * 0.035 * clamp(depth, 0.0, 2.0) / (1.0 + dist * 0.03);
        vec3 rbed = behind(ruv);
        if (rbed.y > v_pos.y) { ruv = uv; rbed = bed; }
        float thick = length(rbed - v_pos);
        vec3 seen = texture(u_scene, ruv).rgb;
        // Shallows are clear and turquoise; depth swallows the light.
        vec3 shallow = seen * mix(vec3(1.0), vec3(0.55, 0.9, 0.85), clamp(thick * 0.5, 0.0, 1.0));
        body = mix(shallow, body, 1.0 - exp(-thick * mix(0.35, 0.22, u_sea)));
        // Foam where waves meet the shore.
        float lap = sin(depth * 5.0 - u_time * 1.6 + fbm3(w * 0.2) * 6.0) * 0.5 + 0.5;
        foam = (1.0 - smoothstep(0.0, 0.5 + 0.9 * u_sea, depth)) * (0.35 + 0.65 * lap);
        foam += u_sea * (1.0 - smoothstep(0.0, 2.6, depth)) * smoothstep(0.55, 0.8, vnoise(w * 0.9 + u_time * 0.3)) * 0.6;
        foam = clamp(foam, 0.0, 1.0) * smoothstep(0.0, 0.06, depth);
    }
    vec3 r = reflect(vdir, n);
    r.y = abs(r.y);
    float fres = 0.02 + 0.98 * pow(1.0 - max(dot(-vdir, n), 0.0), 5.0);
    vec3 sky = sky_color(r);
    // Clouds mirrored in the water.
    float ct = 1400.0 / max(r.y, 0.05);
    float cov = cloud_cover(world_xz(v_pos + r * ct)) * exp(-ct * 0.00007);
    sky = mix(sky, u_cloud_lit * 0.9, cov * 0.8);
    vec3 col = mix(body, sky, fres);
    float vis = cloud_shadow(v_pos);
    // The sun's glitter path.
    vec3 h = normalize(u_sun - vdir);
    col += u_sun_col * vis * (pow(max(dot(n, h), 0.0), 900.0) * 14.0 + pow(max(dot(n, h), 0.0), 90.0) * 0.25);
    vec3 foam_col = vec3(0.8, 0.85, 0.88) * (u_sun_col * max(u_sun.y, 0.0) * vis * 1.2 + ambient(vec3(0.0, 1.0, 0.0)));
    col = mix(col, foam_col, foam * 0.85);
    frag = out_col(apply_fog(col, v_pos));
}
"#;

pub const MOUNTAIN_VS: &str = r#"
in vec3 a_pos;
in vec3 a_nrm;
in vec3 a_col;
uniform mat4 u_vp;
uniform vec3 u_anchor;   // follows the camera so the ring stays at the horizon
out vec3 v_dir;
out vec3 v_nrm;
out vec3 v_col;
void main() {
    vec3 p = a_pos + u_anchor;
    v_dir = a_pos;
    v_nrm = a_nrm;
    v_col = a_col;
    gl_Position = u_vp * vec4(p, 1.0);
}
"#;

pub const MOUNTAIN_FS: &str = r#"
uniform float u_haze;
uniform float u_snowline;   // height of the snow line, or a huge number
uniform float u_treeline;
uniform float u_ring;
in vec3 v_dir;
in vec3 v_nrm;
in vec3 v_col;
out vec4 frag;
void main() {
    float ang = atan(v_dir.x, -v_dir.z);
    float h = v_dir.y;
    // Ridges and gullies running down the slopes.
    vec2 q = vec2(ang * 38.0 + u_ring * 17.0, h * 0.0035);
    float ridges = fbm(q);
    float gx = fbm(q + vec2(0.35, 0.0)) - ridges;
    vec3 tangent = normalize(vec3(-v_dir.z, 0.0, v_dir.x));
    vec3 n = normalize(v_nrm + tangent * gx * 5.0 + vec3(0.0, (ridges - 0.5) * 0.5, 0.0));
    float fine = fbm3(vec2(ang * 160.0, h * 0.02));
    vec3 rock = v_col * (0.7 + 0.6 * ridges) * (0.85 + 0.3 * fine);
    vec3 col = rock;
    // Forest below the tree line, in ragged tongues.
    float trees = 1.0 - smoothstep(u_treeline - 120.0, u_treeline + 60.0, h + (ridges - 0.5) * 320.0);
    col = mix(col, vec3(0.022, 0.05, 0.026) * (0.7 + 0.6 * fine), trees * 0.9);
    // Snow above the snow line, thinner on steep faces.
    float snow = smoothstep(u_snowline - 40.0, u_snowline + 110.0, h + (ridges - 0.5) * 300.0 + (fine - 0.5) * 90.0);
    snow *= 0.55 + 0.45 * smoothstep(-0.25, 0.25, gx + 0.1);
    col = mix(col, vec3(0.84, 0.87, 0.94), snow);
    vec3 d = normalize(v_dir);
    vec3 lit = shade(col, n, 1.0, 1.0);
    // Distant mountains fade into the sky, most strongly at their base.
    float haze = clamp(u_haze + (1.0 - smoothstep(-150.0, 900.0, h)) * 0.32, 0.0, 0.96);
    vec3 air = sky_color(normalize(vec3(d.x, 0.03, d.z)));
    frag = out_col(mix(lit, air, haze));
}
"#;

// ---------------------------------------------------------------------------
// Post-processing

/// Downsample with a wide 13-tap kernel (stable, no flicker).
/// The cloud shadow map: `cloud_light` across the cloud deck above the view.
pub const CLOUD_MAP_FS: &str = r#"
in vec2 v_uv;
out vec4 frag;
void main() {
    frag = vec4(cloud_light(u_cloud_box.xy + (v_uv * 2.0 - 1.0) * u_cloud_box.z));
}
"#;

pub const DOWN_FS: &str = r#"
uniform sampler2D u_tex;
uniform vec2 u_texel;
uniform float u_first;
in vec2 v_uv;
out vec4 frag;
vec3 tap(vec2 o) {
    vec3 c = texture(u_tex, v_uv + o * u_texel).rgb;
    // Tame single very bright pixels on the first pass.
    if (u_first > 0.5) c /= 1.0 + 0.15 * max(c.r, max(c.g, c.b));
    return c;
}
void main() {
    vec3 a = tap(vec2(-2.0, 2.0));
    vec3 b = tap(vec2(0.0, 2.0));
    vec3 c = tap(vec2(2.0, 2.0));
    vec3 d = tap(vec2(-2.0, 0.0));
    vec3 e = tap(vec2(0.0, 0.0));
    vec3 f = tap(vec2(2.0, 0.0));
    vec3 g = tap(vec2(-2.0, -2.0));
    vec3 h = tap(vec2(0.0, -2.0));
    vec3 i = tap(vec2(2.0, -2.0));
    vec3 j = tap(vec2(-1.0, 1.0));
    vec3 k = tap(vec2(1.0, 1.0));
    vec3 l = tap(vec2(-1.0, -1.0));
    vec3 m = tap(vec2(1.0, -1.0));
    vec3 s = e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
    frag = vec4(s, 1.0);
}
"#;

/// Blend a blurrier level into the next sharper one with a tent filter.
pub const UP_FS: &str = r#"
uniform sampler2D u_tex;    // the smaller, blurrier level
uniform sampler2D u_base;   // this level
uniform vec2 u_texel;
uniform float u_mix;
in vec2 v_uv;
out vec4 frag;
void main() {
    vec2 t = u_texel;
    vec3 s = texture(u_tex, v_uv).rgb * 4.0;
    s += (texture(u_tex, v_uv + vec2(t.x, 0.0)).rgb + texture(u_tex, v_uv - vec2(t.x, 0.0)).rgb
        + texture(u_tex, v_uv + vec2(0.0, t.y)).rgb + texture(u_tex, v_uv - vec2(0.0, t.y)).rgb) * 2.0;
    s += texture(u_tex, v_uv + t).rgb + texture(u_tex, v_uv - t).rgb
       + texture(u_tex, v_uv + vec2(t.x, -t.y)).rgb + texture(u_tex, v_uv + vec2(-t.x, t.y)).rgb;
    frag = vec4(mix(texture(u_base, v_uv).rgb, s / 16.0, u_mix), 1.0);
}
"#;

/// Shafts of sunlight: march from each pixel towards the sun and count how
/// much open sky lies in between.
pub const RAYS_FS: &str = r#"
uniform sampler2D u_depth;
uniform vec2 u_sun_uv;
uniform float u_aspect;
in vec2 v_uv;
out vec4 frag;
float ign(vec2 p) { return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715)))); }
void main() {
    vec2 delta = u_sun_uv - v_uv;
    float span = length(delta * vec2(u_aspect, 1.0));
    vec2 stepv = delta / 28.0 * min(1.0, 0.9 / max(span, 0.001));
    vec2 uv = v_uv + stepv * ign(gl_FragCoord.xy);
    float sum = 0.0;
    float wgt = 1.0;
    for (int i = 0; i < 28; i++) {
        vec2 c = clamp(uv, vec2(0.001), vec2(0.999));
        float sky = step(0.99999, texture(u_depth, c).r);
        // Sky close to the sun counts most.
        vec2 ds = (u_sun_uv - uv) * vec2(u_aspect, 1.0);
        sum += sky * wgt * exp(-dot(ds, ds) * 5.0);
        wgt *= 0.955;
        uv += stepv;
    }
    frag = vec4(vec3(sum / 28.0), 1.0);
}
"#;

/// Final pass: bloom, light shafts, frosted glass behind the HUD, exposure,
/// tonemapping, grading, vignette and dither.
pub const COMPOSITE_FS: &str = r#"
uniform sampler2D u_scene;
uniform sampler2D u_bloom;
uniform sampler2D u_blur;
uniform sampler2D u_rays;
uniform float u_hdr;
uniform float u_exposure;
uniform float u_desat;
uniform float u_bloom_k;
uniform vec3 u_ray_col;
uniform vec2 u_res;
uniform vec3 u_grade;       // contrast, saturation, vignette
uniform vec3 u_tint_shadow;
uniform vec3 u_tint_light;
uniform float u_time;
uniform int u_glass_n;
uniform vec4 u_glass[16];   // x, y, width, height in pixels from the top left
uniform float u_glass_r[16];
in vec2 v_uv;
out vec4 frag;

float ign(vec2 p) { return fract(52.9829189 * fract(dot(p, vec2(0.06711056, 0.00583715)))); }

vec3 aces(vec3 c) {
    return (c * (2.51 * c + 0.03)) / (c * (2.43 * c + 0.59) + 0.14);
}

void main() {
    vec3 c = texture(u_scene, v_uv).rgb;
    vec3 bloom = texture(u_bloom, v_uv).rgb;
    // Frosted glass under the HUD panels.
    vec2 px = vec2(v_uv.x, 1.0 - v_uv.y) * u_res;
    float glass = 0.0;
    for (int i = 0; i < 16; i++) {
        if (i >= u_glass_n) break;
        vec4 g = u_glass[i];
        vec2 half_size = g.zw * 0.5;
        vec2 q = abs(px - (g.xy + half_size)) - half_size + u_glass_r[i];
        float sd = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - u_glass_r[i];
        glass = max(glass, 1.0 - smoothstep(-1.0, 0.5, sd));
    }
    if (glass > 0.0) c = mix(c, texture(u_blur, v_uv).rgb, glass);
    if (u_hdr > 0.5) {
        c = mix(c, bloom, u_bloom_k * (1.0 - 0.6 * glass));
        float rays = texture(u_rays, v_uv).r;
        c += u_ray_col * rays * (1.0 - 0.5 * glass);
        c *= u_exposure;
        c = mix(c, vec3(dot(c, vec3(0.3, 0.6, 0.1))), u_desat);
        c = aces(c);
        c = pow(clamp(c, 0.0, 1.0), vec3(1.0 / 2.2));
    } else {
        c = mix(c, max(c, bloom), 0.25 * (1.0 - glass));
    }
    // Grade: a gentle S-curve, a little more colour, split toning.
    float l = dot(c, vec3(0.299, 0.587, 0.114));
    c = mix(vec3(l), c, u_grade.y);
    c = mix(c, c * c * (3.0 - 2.0 * c), u_grade.x);
    c *= mix(u_tint_shadow, u_tint_light, smoothstep(0.0, 0.8, l));
    vec2 vg = v_uv - 0.5;
    c *= 1.0 - u_grade.z * smoothstep(0.25, 1.1, dot(vg, vg) * 2.2);
    // The glass darkens what is behind it so text stays readable.
    c = mix(c, c * 0.42 + vec3(0.012, 0.016, 0.024), glass);
    c += (ign(gl_FragCoord.xy + fract(u_time) * 61.0) - 0.5) / 255.0;
    frag = vec4(c, 1.0);
}
"#;
