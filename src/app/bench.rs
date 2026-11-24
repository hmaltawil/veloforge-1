//! The benchmark (`--bench`): timing each frame and part of the 3D view,
//! and the report printed at the end.

use super::*;

impl App {
    /// How long to wait before drawing the next frame while riding: what is
    /// left of this frame's share of a second under the frame-rate limit, or
    /// nothing when there is no limit (the screen's own rate).
    /// Record one benchmark frame; at the end print the results and quit.
    pub(super) fn bench_frame(
        &mut self,
        ctx: &egui::Context,
        gpu_ms: Option<[f32; crate::gfx::STAGES]>,
        size: (f32, f32),
        scale: f32,
    ) {
        let Some(b) = &mut self.bench else { return };
        ctx.request_repaint();
        let now = Instant::now();
        let elapsed = now.duration_since(b.start).as_secs_f64();
        if elapsed < Bench::WARM_UP {
            b.last = now;
            return;
        }
        b.measuring = true;
        let ms = now.duration_since(b.last).as_secs_f32() * 1000.0;
        b.frames.push(ms);
        b.last = now;
        if let Some(g) = gpu_ms {
            b.gpu.push(g);
        }
        let other = self.gpu.other_ms();
        if let Some(o) = other {
            b.other.push(o);
        }
        if let Some(c) = self.gpu.stage_counts() {
            b.counts.push(c);
        }
        b.rows.push(BenchRow {
            t: (elapsed - Bench::WARM_UP) as f32,
            ms,
            cpu: b.last_cpu,
            gpu: gpu_ms.map_or(0.0, |g| g.iter().sum()),
            show: other.map_or(0.0, |o| o[1]),
            land: self.gpu.land_uploads(),
        });
        if elapsed < Bench::WARM_UP + b.secs {
            return;
        }
        let s = &self.store.settings;
        let report = b.report(
            env!("CARGO_PKG_VERSION"),
            &self.routes[self.route_idx].name,
            s.gfx_quality.label(),
            scale,
            size,
            &self.graphics,
        );
        let csv = b.frames_csv();
        println!("{report}");
        for line in report.lines() {
            self.log_line(line.to_string());
        }
        let _ = std::fs::write(self.paths.data.join("bench.txt"), &report);
        let _ = std::fs::write(self.paths.data.join("bench-frames.csv"), csv);
        self.bench = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Measurements for `--bench`.
pub(super) struct Bench {
    start: Instant,
    last: Instant,
    secs: f64,
    /// Time between frames, in milliseconds.
    frames: Vec<f32>,
    /// GPU milliseconds per frame for each stage (see `gfx::STAGES`).
    gpu: Vec<[f32; crate::gfx::STAGES]>,
    /// What each stage drew per frame: triangles, pixel samples and
    /// fragment shader runs (the same on any graphics card, unlike the
    /// times).
    counts: Vec<[crate::gfx::StageCounts; crate::gfx::STAGES]>,
    /// The computer's milliseconds per frame: the app's work (of which
    /// preparing the 3D frame), issuing the 3D drawing commands, and the
    /// rest of drawing and presenting the screen.
    cpu: Vec<[f32; 4]>,
    /// GPU milliseconds outside the 3D view: the rest of the screen, and
    /// showing the frame.
    other: Vec<[f32; 2]>,
    /// Every measured frame, in order, to see what the slow ones have in
    /// common.
    rows: Vec<BenchRow>,
    /// The computer's latest timings (as in `cpu`).
    last_cpu: [f32; 4],
    measuring: bool,
    /// Where along the route the ride started (`--km`).
    pub(super) from_km: f64,
}

/// One measured frame.
#[derive(Clone, Copy, Default)]
pub(super) struct BenchRow {
    /// Seconds since measuring began.
    t: f32,
    /// Milliseconds since the frame before.
    ms: f32,
    /// The computer's latest timings: the app, preparing the 3D view,
    /// issuing its drawing commands, the rest of the screen and showing it.
    cpu: [f32; 4],
    /// The graphics card's latest time for the 3D view (its timings arrive
    /// a few frames late), and for showing the frame.
    gpu: f32,
    show: f32,
    /// How many times the land had been uploaded by then.
    land: u32,
}

impl Bench {
    /// Seconds to let the world build and the rider settle before measuring.
    const WARM_UP: f64 = 10.0;

    pub(super) fn new(secs: f64) -> Bench {
        let now = Instant::now();
        Bench {
            start: now,
            last: now,
            secs,
            frames: Vec::new(),
            gpu: Vec::new(),
            counts: Vec::new(),
            cpu: Vec::new(),
            other: Vec::new(),
            rows: Vec::new(),
            last_cpu: [0.0; 4],
            measuring: false,
            from_km: 0.0,
        }
    }

    pub(super) fn cpu_frame(&mut self, app: f32, present: f32, prep: f32, paint: f32) {
        self.last_cpu = [app, prep, paint, (present - paint).max(0.0)];
        if self.measuring {
            self.cpu.push(self.last_cpu);
        }
    }

    fn report(
        &self,
        version: &str,
        route: &str,
        detail: &str,
        scale: f32,
        size: (f32, f32),
        graphics: &str,
    ) -> String {
        let n = self.frames.len().max(1) as f32;
        let total: f32 = self.frames.iter().sum();
        let mut sorted = self.frames.clone();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let slow = &sorted[..(sorted.len() / 100).max(1).min(sorted.len())];
        let slow_ms = slow.iter().sum::<f32>() / slow.len().max(1) as f32;
        let mut out = format!(
            "VeloForge {version} benchmark: {:.0} s on {route}{}\n\
             Detail {detail}, world drawn at {:.0}% of {:.0}x{:.0} pixels\n\
             Graphics: {graphics}\n\
             Frames: {} ({:.1} per second), {:.1} ms on average, slowest 1% {:.1} ms\n",
            self.secs,
            if self.from_km > 0.0 {
                format!(", from km {}", self.from_km)
            } else {
                String::new()
            },
            scale * 100.0,
            size.0,
            size.1,
            self.frames.len(),
            n / (total / 1000.0).max(1e-3),
            total / n,
            slow_ms,
        );
        if self.gpu.is_empty() {
            out += "Graphics card time: not available with this driver\n";
        } else {
            let k = self.gpu.len() as f32;
            let avg = |i: usize| self.gpu.iter().map(|g| g[i]).sum::<f32>() / k;
            let st: Vec<f32> = (0..crate::gfx::STAGES).map(avg).collect();
            let all: f32 = st.iter().sum();
            let scene = st[1] + st[2] + st[3] + st[4];
            out += &format!(
                "Graphics card time per frame: {all:.1} ms (shadows {:.1}, scene {scene:.1}, finishing {:.1})\n\
                 The scene: trees and objects {:.1}, ground and road {:.1}, grass {:.1}, sky, water and the rest {:.1}\n\
                 Graphics card busy: {:.0}% at 30 fps, {:.0}% at 60 fps\n",
                st[0],
                st[5],
                st[1],
                st[2],
                st[3],
                st[4],
                all * 30.0 / 10.0,
                all * 60.0 / 10.0,
            );
        }
        if !self.counts.is_empty() {
            let k = self.counts.len() as f64;
            let avg = |f: fn(&crate::gfx::StageCounts) -> u64| -> Vec<f64> {
                (0..crate::gfx::STAGES)
                    .map(|i| self.counts.iter().map(|c| f(&c[i]) as f64).sum::<f64>() / k)
                    .collect()
            };
            let tri: Vec<f64> = avg(|c| c.triangles).iter().map(|t| t / 1e3).collect();
            let px: Vec<f64> = avg(|c| c.samples).iter().map(|p| p / 1e6).collect();
            let frag: Vec<f64> = avg(|c| c.fragments.unwrap_or(0))
                .iter()
                .map(|p| p / 1e6)
                .collect();
            let mut rows = vec![
                ("Triangles drawn per frame", "thousand", &tri),
                ("Pixel samples drawn per frame", "million", &px),
            ];
            if self.counts.iter().any(|c| c[0].fragments.is_some()) {
                rows.push(("Pixels worked out per frame", "million", &frag));
            }
            for (what, unit, v) in rows {
                out += &format!(
                    "{what} ({unit}): {:.1} (shadows {:.1}, trees and objects {:.1}, \
                     ground and road {:.1}, grass {:.1}, sky, water and the rest {:.1}, finishing {:.1})\n",
                    v.iter().sum::<f64>(),
                    v[0],
                    v[1],
                    v[2],
                    v[3],
                    v[4],
                    v[5],
                );
            }
        }
        if !self.other.is_empty() {
            let k = self.other.len() as f32;
            let avg = |i: usize| self.other.iter().map(|o| o[i]).sum::<f32>() / k;
            out += &format!(
                "Graphics card outside the 3D view: the rest of the screen {:.1} ms, showing the frame {:.1} ms\n",
                avg(0),
                avg(1),
            );
        }
        if !self.cpu.is_empty() {
            let k = self.cpu.len() as f32;
            let avg = |i: usize| self.cpu.iter().map(|c| c[i]).sum::<f32>() / k;
            out += &format!(
                "Computer time per frame: the app {:.1} ms (of which preparing the 3D view {:.1}), \
                 3D drawing commands {:.1}, the rest of the screen and showing it {:.1}\n",
                avg(0),
                avg(1),
                avg(2),
                avg(3),
            );
        }
        out += &self.slow_frames();
        out
    }

    /// What the frames that missed the screen's pace had around them.
    fn slow_frames(&self) -> String {
        let rows = &self.rows;
        if rows.len() < 10 {
            return String::new();
        }
        let mut ms: Vec<f32> = rows.iter().map(|r| r.ms).collect();
        ms.sort_by(f32::total_cmp);
        let usual = ms[ms.len() / 2];
        // Slow: at least one more refresh of the screen than usual.
        let limit = usual * 1.4;
        let slow: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].ms > limit).collect();
        if slow.is_empty() {
            return format!("Slow frames: none (usual frame {usual:.1} ms)\n");
        }
        let near = |i: usize, back: usize, ahead: usize| {
            rows[i.saturating_sub(back)..(i + ahead + 1).min(rows.len())].iter()
        };
        let mut land = 0;
        let mut card = 0;
        let mut computer = 0;
        let mut worst_app = 0.0f32;
        let mut worst_card = 0.0f32;
        for &i in &slow {
            let first = rows[i.saturating_sub(3)].land;
            if near(i, 3, 1).any(|r| r.land != first) {
                land += 1;
            }
            // The graphics card's time for a frame arrives a few frames on.
            let g = near(i, 0, 4).map(|r| r.gpu).fold(0.0, f32::max);
            worst_card = worst_card.max(g);
            if g > usual {
                card += 1;
            }
            let c = near(i, 0, 1)
                .map(|r| r.cpu[0] + r.cpu[2])
                .fold(0.0, f32::max);
            worst_app = worst_app.max(c);
            if c > usual * 0.5 {
                computer += 1;
            }
        }
        let gaps: Vec<f32> = slow
            .windows(2)
            .map(|w| rows[w[1]].t - rows[w[0]].t)
            .collect();
        let mut sorted_gaps = gaps.clone();
        sorted_gaps.sort_by(f32::total_cmp);
        let gap = sorted_gaps
            .get(sorted_gaps.len() / 2)
            .copied()
            .unwrap_or(0.0);
        format!(
            "Slow frames: {} over {limit:.0} ms (usual frame {usual:.1} ms), typically {gap:.1} s apart\n\
             Around them: land uploaded {land}, graphics card over {usual:.0} ms {card} (worst {worst_card:.1}), \
             computer over {:.0} ms {computer} (worst {worst_app:.1}), land uploads in the run {}\n",
            slow.len(),
            usual * 0.5,
            rows.last().map_or(0, |r| r.land) - rows[0].land,
        )
    }

    /// Every measured frame, for a closer look in a spreadsheet.
    fn frames_csv(&self) -> String {
        let mut out = String::from("seconds,frame_ms,app_ms,prepare_ms,commands_ms,rest_and_show_ms,gpu_3d_ms,gpu_show_ms,land_uploads\n");
        for r in &self.rows {
            out += &format!(
                "{:.3},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{}\n",
                r.t, r.ms, r.cpu[0], r.cpu[1], r.cpu[2], r.cpu[3], r.gpu, r.show, r.land
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bench_finds_what_slow_frames_have_around_them() {
        let mut b = Bench::new(60.0);
        for i in 0..300 {
            let mut r = BenchRow {
                t: i as f32 / 30.0,
                ms: 33.3,
                gpu: 25.0,
                land: if i < 150 { 3 } else { 4 },
                ..Default::default()
            };
            // One frame right after the land is uploaded, one where the
            // computer was busy.
            if i == 151 {
                r.ms = 50.0;
            }
            if i == 40 {
                r.ms = 50.0;
                r.cpu[0] = 30.0;
            }
            b.rows.push(r);
        }
        let s = b.slow_frames();
        assert!(s.starts_with("Slow frames: 2 over 47 ms"), "{s}");
        assert!(s.contains("land uploaded 1,"), "{s}");
        assert!(s.contains("computer over 17 ms 1 (worst 30.0)"), "{s}");
        assert!(s.contains("land uploads in the run 1"), "{s}");
        assert_eq!(b.frames_csv().lines().count(), 301);
    }
}
