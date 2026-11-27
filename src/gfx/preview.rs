//! A small 3D view of the rider on the bike, on its own (Settings ›
//! Profile): no world around it, a studio light, turning when dragged, so
//! the rider sees their jersey, bike and kit while choosing them.

use super::*;

/// What the preview shows: the rider's colours and the view.
#[derive(Debug, Clone, Copy)]
pub struct PreviewLook {
    /// sRGB.
    pub jersey: [u8; 3],
    pub skin: [u8; 3],
    /// Angle round the rider: 0 from the right side, growing towards the
    /// front.
    pub yaw: f32,
    /// Seconds, for pedalling and breathing.
    pub time: f64,
}

/// One frame of the preview, worked out on the CPU.
struct PreviewFrame {
    vp: M4,
    cam: V3,
    bike: Vec<f32>,
    bones: [M4; body::BONES],
    tint: V3,
    skin: V3,
}

/// The studio light: a warm key from above and in front, a cool fill, a
/// sky-and-floor ambient and a rim, without shadows or fog.
const PREVIEW_OBJ_VS: &str = r#"
in vec3 a_pos;
in vec3 a_nrm;
in vec3 a_col;
in float a_mat;
uniform mat4 u_vp;
out vec3 v_pos;
out vec3 v_nrm;
out vec3 v_col;
out float v_mat;
out float v_h;
void main() {
    v_pos = a_pos;
    v_nrm = a_nrm;
    v_col = a_col;
    v_mat = a_mat;
    v_h = 0.5;
    gl_Position = u_vp * vec4(a_pos, 1.0);
}
"#;

const PREVIEW_FS: &str = r#"
in vec3 v_pos;
in vec3 v_nrm;
in vec3 v_col;
in float v_mat;
in float v_h;
uniform vec3 u_cam;
out vec4 frag;
void main() {
    vec3 n = normalize(v_nrm);
    vec3 v = normalize(u_cam - v_pos);
    if (dot(n, v) < 0.0) n = -n;
    vec3 key = normalize(vec3(0.45, 0.85, -0.35));
    vec3 fill = normalize(vec3(-0.6, 0.35, 0.5));
    float d = max(dot(n, key), 0.0);
    float f = max(dot(n, fill), 0.0);
    float hemi = 0.5 + 0.5 * n.y;
    bool gloss = abs(v_mat - 3.0) < 0.5;
    bool skin = abs(v_mat - 6.0) < 0.5;
    vec3 light = vec3(1.0, 0.96, 0.9) * d * 1.7
        + vec3(0.55, 0.62, 0.78) * f * 0.5
        + mix(vec3(0.14, 0.13, 0.12), vec3(0.4, 0.44, 0.52), hemi);
    vec3 col = v_col * light;
    // Light scattered under the skin softens its shadow side.
    if (skin) col += v_col * vec3(0.18, 0.06, 0.03) * (1.0 - d);
    float facing = max(dot(n, v), 0.0);
    col += pow(1.0 - facing, 3.0) * 0.16 * vec3(0.8, 0.86, 1.0);
    vec3 h = normalize(key + v);
    float shine = gloss ? 90.0 : (skin ? 20.0 : 10.0);
    col += pow(max(dot(n, h), 0.0), shine) * (gloss ? 0.7 : 0.05) * d;
    // Gloss paint reflects a soft studio sky.
    if (gloss) col += mix(vec3(0.05), vec3(0.45, 0.5, 0.58), hemi) * 0.35 * pow(1.0 - facing, 2.0);
    // Lamps (the rear light) glow.
    if (abs(v_mat - 2.0) < 0.5) col = col * 0.4 + v_col * 1.6;
    col = col / (1.0 + 0.12 * col);
    frag = vec4(pow(col, vec3(1.0 / 2.2)), 1.0);
}
"#;

/// Copies the smooth preview onto the screen, keeping its soft edges.
const PREVIEW_COPY_VS: &str = r#"
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const PREVIEW_COPY_FS: &str = r#"
in vec2 v_uv;
uniform sampler2D u_tex;
out vec4 frag;
void main() {
    frag = texture(u_tex, v_uv);
}
"#;

/// Where the preview is drawn first: multisampled, as the 3D view is, so
/// thin spokes and edges come out smooth rather than crawling as the
/// wheels turn (the window itself isn't multisampled). Then resolved into
/// a texture that goes on the screen.
pub(super) struct PreviewTarget {
    w: i32,
    h: i32,
    fbo: glow::Framebuffer,
    color: glow::Renderbuffer,
    depth: glow::Renderbuffer,
    resolved: Target,
}

impl PreviewTarget {
    unsafe fn new(gl: &glow::Context, w: i32, h: i32) -> Result<PreviewTarget, String> {
        unsafe {
            let samples = gl.get_parameter_i32(glow::MAX_SAMPLES).clamp(1, 8);
            let color = gl.create_renderbuffer()?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
            gl.renderbuffer_storage_multisample(glow::RENDERBUFFER, samples, glow::RGBA8, w, h);
            let depth = gl.create_renderbuffer()?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage_multisample(
                glow::RENDERBUFFER,
                samples,
                glow::DEPTH_COMPONENT24,
                w,
                h,
            );
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(color),
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            let complete =
                gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
            let resolved = if complete {
                GlState::target(gl, w, h, false).ok()
            } else {
                None
            };
            let Some(resolved) = resolved else {
                gl.delete_framebuffer(fbo);
                gl.delete_renderbuffer(color);
                gl.delete_renderbuffer(depth);
                return Err("multisampled preview not supported".into());
            };
            Ok(PreviewTarget {
                w,
                h,
                fbo,
                color,
                depth,
                resolved,
            })
        }
    }

    unsafe fn free(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.fbo);
            gl.delete_renderbuffer(self.color);
            gl.delete_renderbuffer(self.depth);
            gl.delete_framebuffer(self.resolved.fbo);
            gl.delete_texture(self.resolved.tex);
        }
    }
}

impl GpuWorld {
    /// Draw the rider on the bike in `rect`, over whatever is painted
    /// there already (the stage behind it).
    pub fn paint_preview(&self, painter: &egui::Painter, rect: Rect, look: &PreviewLook) {
        let t = look.time;
        let cadence = 1.4; // turns a second: an easy 84 rpm
        let crank = ((t * cadence * std::f64::consts::TAU) % std::f64::consts::TAU) as f32;
        let pose = mesh::RiderPose {
            pos: [0.0; 3],
            fwd: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            crank,
            wheel: ((t * 6.0) % std::f64::consts::TAU) as f32,
            jersey: srgb(look.jersey[0], look.jersey[1], look.jersey[2]),
            stand: 0.0,
            tuck: 0.0,
            effort: 0.6,
            time: (t % 3600.0) as f32,
        };
        let joints = mesh::joints(&pose);
        let bike = if bike_model::aero() {
            bike_model::mesh(&pose, &joints, false).v
        } else {
            mesh::bike(&pose, &joints).v
        };

        // Orbit a point between the wheels at the rider's waist.
        let target = [0.0, 0.66, -0.08];
        let (dist, pitch) = (3.45f32, 0.24f32);
        let cam = [
            target[0] + dist * look.yaw.cos() * pitch.cos(),
            target[1] + dist * pitch.sin(),
            target[2] - dist * look.yaw.sin() * pitch.cos(),
        ];
        let aspect = rect.width() / rect.height().max(1.0);
        // Fit the bike's length in a narrow view and the rider's height in
        // a wide one.
        let fov = (0.5f32).max(0.62 / aspect.min(1.0)).min(0.9);
        let vp = mul(
            &perspective(fov, aspect, 0.1, 20.0),
            &look_at(cam, target, [0.0, 1.0, 0.0]),
        );
        // The floor: a lit disc the bike stands on, a soft shadow under it
        // and a dark touch where each tyre meets the ground, drawn first.
        let floor = |x: f32, z: f32| {
            project(&vp, [x, 0.0, z], rect.width(), rect.height())
                .map(|(px, py, _)| rect.min + egui::vec2(px, py))
        };
        let disc: Vec<Pos2> = (0..48)
            .filter_map(|i| {
                let a = i as f32 / 48.0 * std::f32::consts::TAU;
                floor(0.8 * a.cos(), -0.08 + 0.9 * a.sin())
            })
            .collect();
        if disc.len() == 48 {
            painter.add(egui::Shape::convex_polygon(
                disc,
                egui::Color32::from_white_alpha(14),
                egui::Stroke::new(1.0, egui::Color32::from_white_alpha(28)),
            ));
        }
        let blob = |x0: f32, z0: f32, rx: f32, rz: f32, alpha: u8| {
            let pts: Vec<Pos2> = (0..24)
                .filter_map(|i| {
                    let a = i as f32 / 24.0 * std::f32::consts::TAU;
                    floor(x0 + rx * a.cos(), z0 + rz * a.sin())
                })
                .collect();
            if pts.len() == 24 {
                painter.add(egui::Shape::convex_polygon(
                    pts,
                    egui::Color32::from_black_alpha(alpha),
                    egui::Stroke::NONE,
                ));
            }
        };
        for (k, alpha) in [(1.0f32, 40u8), (0.7, 45), (0.45, 50)] {
            blob(0.0, -0.07, 0.22 * k, 0.78 * k, alpha);
        }
        // Rear and front tyres.
        for z in [0.41f32, -0.57] {
            blob(0.0, z, 0.05, 0.12, 90);
            blob(0.0, z, 0.025, 0.06, 120);
        }
        let frame = Arc::new(PreviewFrame {
            vp,
            cam,
            bike,
            bones: body::skinning(&self.rest_inverse, &joints),
            tint: pose.jersey,
            skin: srgb(look.skin[0], look.skin[1], look.skin[2]),
        });
        let state = self.gl.clone();
        let cb = egui_glow::CallbackFn::new(move |info, p| {
            if let Ok(mut st) = state.lock() {
                st.paint_preview(p.gl(), &info, &frame);
            }
        });
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(cb),
        });
    }
}

impl GlState {
    fn paint_preview(
        &mut self,
        gl: &glow::Context,
        info: &egui::PaintCallbackInfo,
        f: &PreviewFrame,
    ) {
        unsafe {
            if !self.preview_ready {
                self.preview_ready = true;
                let obj = ["a_pos", "a_nrm", "a_col", "a_mat"];
                let skinned = ["a_pos", "a_nrm", "a_col", "a_mat", "a_bones", "a_weights"];
                let made = self
                    .compile(gl, "preview_obj", &[PREVIEW_OBJ_VS], &[PREVIEW_FS], &obj)
                    .and_then(|_| {
                        self.compile(
                            gl,
                            "preview_skin",
                            &[shaders::SKINNED_VS],
                            &[PREVIEW_FS],
                            &skinned,
                        )
                    })
                    .and_then(|_| {
                        self.compile(
                            gl,
                            "preview_copy",
                            &[PREVIEW_COPY_VS],
                            &[PREVIEW_COPY_FS],
                            &[],
                        )
                    })
                    .and_then(|_| {
                        Self::buffers(
                            gl,
                            &[(3, 0), (3, 3), (3, 6), (1, 9)],
                            mesh::VERTEX_STRIDE as i32,
                            &[],
                            None,
                        )
                    });
                match made {
                    Ok(b) => self.preview_bike = Some(b),
                    Err(e) => {
                        eprintln!("rider preview: {e}");
                        return;
                    }
                }
            }
            if !self.skin_tried {
                if let Some(skin) = SKIN.get() {
                    self.skin_tried = true;
                    self.skin = Self::upload_skin(gl, skin).ok();
                }
            }

            let vp = info.viewport_in_pixels();
            let clip = info.clip_rect_in_pixels();
            let (w, h) = (vp.width_px.max(1), vp.height_px.max(1));
            let screen = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            // The smooth buffer, made again when the preview changes size. A
            // driver that can't make one gets the preview drawn straight on
            // the screen.
            if self
                .preview_target
                .as_ref()
                .is_none_or(|t| t.w != w || t.h != h)
            {
                if let Some(old) = self.preview_target.take() {
                    old.free(gl);
                }
                self.preview_target = PreviewTarget::new(gl, w, h).ok();
            }
            let smooth = self
                .preview_target
                .as_ref()
                .map(|t| (t.fbo, t.resolved.fbo, t.resolved.tex));
            if let Some((fbo, _, _)) = smooth {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.disable(glow::SCISSOR_TEST);
                gl.viewport(0, 0, w, h);
                gl.color_mask(true, true, true, true);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.depth_mask(true);
                gl.clear_depth_f32(1.0);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            } else {
                gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
                gl.viewport(vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px);
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor(
                    clip.left_px,
                    clip.from_bottom_px,
                    clip.width_px,
                    clip.height_px,
                );
                gl.depth_mask(true);
                gl.clear_depth_f32(1.0);
                gl.clear(glow::DEPTH_BUFFER_BIT);
            }
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::BLEND);

            let Some(bike) = self.preview_bike.as_mut() else {
                return;
            };
            // The bike.
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(bike.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&f.bike), glow::DYNAMIC_DRAW);
            bike.count = (f.bike.len() / mesh::VERTEX_STRIDE) as i32;
            let (vao, count) = (bike.vao, bike.count);
            let bike_vao = vao;
            let p = self.programs.get_mut("preview_obj").unwrap();
            gl.use_program(Some(p.id));
            p.m4(gl, "u_vp", &f.vp);
            p.v3(gl, "u_cam", f.cam);
            gl.bind_vertex_array(Some(vao));
            gl.draw_arrays(glow::TRIANGLES, 0, count);

            // The rider.
            if let Some((vao, count)) = self.skin.as_ref().map(|b| (b.vao, b.count)) {
                let p = self.programs.get_mut("preview_skin").unwrap();
                gl.use_program(Some(p.id));
                p.m4(gl, "u_vp", &f.vp);
                p.v3(gl, "u_cam", f.cam);
                let flat: Vec<f32> = f.bones.iter().flatten().copied().collect();
                gl.uniform_matrix_4_f32_slice(p.loc(gl, "u_bones"), false, &flat);
                p.v3(gl, "u_tint", f.tint);
                p.v3(gl, "u_skin", f.skin);
                let kit: Vec<f32> = body::kit().iter().flatten().copied().collect();
                gl.uniform_3_f32_slice(p.loc(gl, "u_kit"), &kit);
                gl.bind_vertex_array(Some(vao));
                gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, 0);
            }

            // The smooth picture onto the screen, its soft edges blended
            // over what's behind it.
            if let Some((fbo, resolved, tex)) = smooth {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(resolved));
                gl.blit_framebuffer(
                    0,
                    0,
                    w,
                    h,
                    0,
                    0,
                    w,
                    h,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
                gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
                gl.viewport(vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px);
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor(
                    clip.left_px,
                    clip.from_bottom_px,
                    clip.width_px,
                    clip.height_px,
                );
                gl.disable(glow::DEPTH_TEST);
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                let p = self.programs.get_mut("preview_copy").unwrap();
                gl.use_program(Some(p.id));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                p.i1(gl, "u_tex", 0);
                gl.bind_vertex_array(Some(bike_vao));
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }

            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
        }
    }
}
