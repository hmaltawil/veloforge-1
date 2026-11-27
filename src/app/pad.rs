//! Finding your way round the app from the bike, with a Zwift Click: the
//! left pad's arrows (or, without them, Z, A, Y and B on the right half)
//! move a white ring from one control to the next, + presses the lit one
//! and − steps back to the left. It drives egui's own keyboard focus, so
//! the arrow keys and Enter do the same. Not on the ride screen or during a
//! ride, where + and − shift.

use super::*;
use crate::zwift;

/// Where the right edge of the screen's sidebar is, this frame.
pub(super) const SIDE_KEY: &str = "pad-side-right";

/// What a Click press asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PadKey {
    Up,
    Down,
    Left,
    Right,
    Press,
    Back,
}

impl App {
    /// The Click finds its way round the app now.
    pub(super) fn pad_nav(&self) -> bool {
        self.tab != Tab::Ride
            && !self.ride_active()
            && self.music_learn.is_none()
            && self.finish_ask.is_none()
            && self.calib.is_none()
    }

    /// Show the hint line: once a Click is connected, or the pad used.
    pub(super) fn pad_hints(&self) -> bool {
        self.pad_shown
            || self
                .links
                .values()
                .any(|l| l.link == Link::Connected && l.caps.contains(&Capability::Shifter))
    }

    /// A Click button went down while finding the way: the ones that steer,
    /// and what is left over (for the music).
    pub(super) fn pad_buttons(&mut self, bits: u64) -> u64 {
        if bits & zwift::PAD != 0 && !self.store.settings.click_pad {
            // A working left pad: steer with it from now on.
            self.store.settings.click_pad = true;
            self.mark_dirty();
        }
        let mut keys = vec![
            (zwift::PAD_LEFT, PadKey::Left),
            (zwift::PAD_UP, PadKey::Up),
            (zwift::PAD_RIGHT, PadKey::Right),
            (zwift::PAD_DOWN, PadKey::Down),
        ];
        if !self.store.settings.click_pad {
            keys.extend([
                (zwift::Z, PadKey::Left),
                (zwift::A, PadKey::Right),
                (zwift::Y, PadKey::Up),
            ]);
        }
        let mut used = 0;
        for (b, k) in keys {
            if bits & b != 0 {
                used |= b;
                self.pad_key(k);
            }
        }
        bits & !used
    }

    /// + or − (or B) while finding the way.
    pub(super) fn pad_shift(&mut self, up: bool) {
        self.pad_key(match (up, self.store.settings.click_pad) {
            (true, _) => PadKey::Press,
            (false, true) => PadKey::Back,
            // Without the pad, B is the way down.
            (false, false) => PadKey::Down,
        });
    }

    fn pad_key(&mut self, k: PadKey) {
        self.pad_shown = true;
        let focused = self.ctx_focus;
        // On Home's workout and route lists, ▲ ▼ step through them.
        let list = self
            .home_lists
            .iter()
            .position(|l| l.is_some() && *l == focused);
        if let (true, Some(list)) = (matches!(k, PadKey::Up | PadKey::Down), list) {
            return self.home_step(list, k == PadKey::Down);
        }
        // + with nothing lit on Home: off to ride.
        if k == PadKey::Press && focused.is_none() && self.tab == Tab::Home {
            self.tab = Tab::Ride;
            return;
        }
        self.pad_queue.push(k);
    }

    /// Before each frame: the Click's presses (and the keyboard's arrows)
    /// carried out, and the ring put away when the mouse is used.
    pub(super) fn pad_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if raw
            .events
            .iter()
            .any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. }))
        {
            self.pad_shown = false;
        }
        // The keyboard's arrows find the way the same as the pad.
        // (Not while typing, or on something dragged with the arrows.)
        let typing = ctx.memory(|m| m.focused()).is_some_and(|id| {
            ctx.viewport(|v| {
                v.prev_pass
                    .widgets
                    .get(id)
                    .is_some_and(|w| w.sense.senses_drag())
            })
        });
        if self.pad_nav() && !typing {
            let mut moves = Vec::new();
            raw.events.retain(|e| {
                let k = match e {
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } if modifiers.is_none() => match key {
                        egui::Key::ArrowUp => Some(PadKey::Up),
                        egui::Key::ArrowDown => Some(PadKey::Down),
                        egui::Key::ArrowLeft => Some(PadKey::Left),
                        egui::Key::ArrowRight => Some(PadKey::Right),
                        _ => None,
                    }
                    .map(|k| (k, *pressed)),
                    _ => None,
                };
                match k {
                    Some((k, true)) => {
                        moves.push(k);
                        false
                    }
                    Some((_, false)) => false,
                    None => true,
                }
            });
            for k in moves {
                self.pad_key(k);
            }
        }
        let keys = std::mem::take(&mut self.pad_queue);
        for k in keys {
            if k == PadKey::Press {
                for pressed in [true, false] {
                    raw.events.push(egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            } else if let Some(id) = self.pad_next(ctx, k) {
                ctx.memory_mut(|m| m.request_focus(id));
                self.ctx_focus = Some(id);
                self.pad_scroll = 2;
            }
        }
    }

    /// Which part of the screen `x` is in: 0 the menu, 1 the screen's
    /// sidebar, 2 the rest.
    fn pad_zone(&self, ctx: &egui::Context, x: f32) -> u8 {
        let menu = super::menu::width(self.tab);
        let side = ctx
            .data(|d| d.get_temp::<f32>(egui::Id::unique(SIDE_KEY)))
            .unwrap_or(0.0);
        if x < menu {
            0
        } else if x < side {
            1
        } else {
            2
        }
    }

    /// Where a press of the pad leads from what is lit: up and down within
    /// a part of the screen, left and right along a row and then into the
    /// next part. Controls you drag (sliders, value boxes, text) are left
    /// out: those are for the mouse.
    fn pad_next(&self, ctx: &egui::Context, k: PadKey) -> Option<egui::Id> {
        let screen = ctx.content_rect();
        let targets: Vec<(egui::Id, Rect)> = ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, ws)| ws.iter())
                .filter(|w| {
                    w.enabled
                        && w.visible
                        && w.sense.senses_click()
                        && !w.sense.senses_drag()
                        && w.rect.is_positive()
                        && w.rect.width() < screen.width() * 0.8
                })
                .map(|w| (w.id, w.rect))
                .collect()
        });
        let zone = |r: &Rect| self.pad_zone(ctx, r.center().x);
        let current = self
            .ctx_focus
            .and_then(|id| targets.iter().find(|(t, _)| *t == id).copied());
        let Some((cur_id, c)) = current else {
            // Nothing lit: the top of the sidebar, else of the screen.
            for z in [1, 2, 0] {
                if let Some(t) = targets
                    .iter()
                    .filter(|(_, r)| zone(r) == z)
                    .min_by(|a, b| a.1.top().total_cmp(&b.1.top()))
                {
                    return Some(t.0);
                }
            }
            return None;
        };
        let z = zone(&c);
        let others = || targets.iter().filter(move |(id, _)| *id != cur_id);
        let gap = |a: f32, b: f32, lo: f32, hi: f32| {
            // How far apart two ranges are (0 when they overlap).
            (lo - b).max(a - hi).max(0.0)
        };
        let best = |it: Vec<&(egui::Id, Rect)>, score: &dyn Fn(&Rect) -> f32| {
            it.into_iter()
                .min_by(|a, b| score(&a.1).total_cmp(&score(&b.1)))
                .map(|t| t.0)
        };
        match k {
            PadKey::Up | PadKey::Down => {
                let down = k == PadKey::Down;
                let cands: Vec<_> = others()
                    .filter(|(_, r)| zone(r) == z)
                    .filter(|(_, r)| {
                        if down {
                            r.center().y > c.center().y + 4.0
                        } else {
                            r.center().y < c.center().y - 4.0
                        }
                    })
                    .collect();
                best(cands, &|r| {
                    (r.center().y - c.center().y).abs()
                        + 3.0 * gap(r.left(), r.right(), c.left(), c.right())
                })
            }
            _ => {
                let right = k == PadKey::Right;
                // Along the row first.
                let row: Vec<_> = others()
                    .filter(|(_, r)| zone(r) == z)
                    .filter(|(_, r)| gap(r.top(), r.bottom(), c.top(), c.bottom()) < 6.0)
                    .filter(|(_, r)| {
                        if right {
                            r.center().x > c.center().x + 4.0
                        } else {
                            r.center().x < c.center().x - 4.0
                        }
                    })
                    .collect();
                if let Some(id) = best(row, &|r| (r.center().x - c.center().x).abs()) {
                    return Some(id);
                }
                // Then into the next part that has anything, at the same
                // height as near as can be.
                let parts: Vec<u8> = if right {
                    (z + 1..=2).collect()
                } else {
                    (0..z).rev().collect()
                };
                for p in parts {
                    let cands: Vec<_> = others().filter(|(_, r)| zone(r) == p).collect();
                    if let Some(id) = best(cands, &|r| {
                        gap(r.top(), r.bottom(), c.top(), c.bottom())
                            + 0.05 * (r.center().x - c.center().x).abs()
                    }) {
                        return Some(id);
                    }
                }
                None
            }
        }
    }

    /// At the start of each frame: what is lit, and keep it in view.
    pub(super) fn pad_begin(&mut self, ctx: &egui::Context) {
        self.ctx_focus = ctx.memory(|m| m.focused());
        ctx.data_mut(|d| d.insert_temp(egui::Id::unique(SIDE_KEY), 0.0f32));
        if self.pad_scroll > 0 {
            self.pad_scroll -= 1;
            if let Some(r) = self.ctx_focus.and_then(|id| ctx.read_response(id)) {
                r.scroll_to_me(None);
            }
        }
    }

    /// After each frame: the white ring round what is lit.
    pub(super) fn pad_ring(&self, ctx: &egui::Context) {
        if !self.pad_shown || self.tab == Tab::Ride {
            return;
        }
        let Some(r) = ctx
            .memory(|m| m.focused())
            .and_then(|id| ctx.read_response(id))
        else {
            return;
        };
        let rect = r.rect;
        if !rect.is_positive() || rect.width() > 2000.0 {
            return;
        }
        super::home_click::ring(ctx, rect, (rect.height() / 2.0).min(18.0));
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
