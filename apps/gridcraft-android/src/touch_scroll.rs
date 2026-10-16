//! Two-finger scrolling for the shell: the platform sends touches, the apps want a wheel.
//!
//! winit's Android backend has no mouse and emits no wheel events. Touch arrives as
//! `egui::Event::Touch`, and egui-winit *also* turns the first finger into an emulated mouse
//! (`PointerButton` + `PointerMoved`), so one finger drags a selection and the page never moves.
//!
//! This turns a **two-finger drag** (any direction: vertical, horizontal or diagonal) into
//! `MouseWheel` events in points, ending with `TouchPhase::End` so the apps coast to a stop, and
//! keeps one finger for selecting:
//!
//! - **The first finger's press is held back** for [`HOLD`] (or until it moves [`SLOP`] points,
//!   or lifts). Fingers of a two-finger gesture land a few milliseconds apart; without the hold
//!   the first one would already have started a selection drag by the time the second arrives.
//!   A tap still selects at once (the press and release go out together on lift), and a drag
//!   starts after at most [`HOLD`].
//! - **A second finger cancels the one-finger drag.** When it lands after the press already went
//!   out (a slow second finger), a release is sent at the first finger's last position so the
//!   selection drag ends where it was.
//! - **During the gesture, and until every finger is up**, the emulated pointer's buttons and
//!   moves are dropped: nothing follows the fingers but the page. The pointer stays where the
//!   gesture started, over the canvas, which is where the apps accept wheel input.
//!
//! The touch events themselves stay in the batch, so egui's pinch (`zoom_delta`) still zooms.
//!
//! Call [`TouchScroll::apply`] from the shell's `raw_input_hook`, after the modifier tracking,
//! and repaint after [`TouchScroll::repaint_after`] so a held press goes out without new input.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a lone finger's press is held back waiting for a second finger.
pub const HOLD: Duration = Duration::from_millis(140);
/// How far (points) a lone finger may move before its press goes out anyway.
pub const SLOP: f32 = 14.0;

#[derive(Debug, Default)]
pub struct TouchScroll {
    /// Keyed by the raw id (`egui::TouchId` has no `Hash`).
    fingers: HashMap<u64, egui::Pos2>,
    /// The centroid the last scroll was measured from; `None` outside a gesture.
    last: Option<egui::Pos2>,
    /// A gesture has scrolled and not yet ended.
    scrolling: bool,
    /// The lone finger's press, held back: (event, when it came, where the finger started).
    held: Option<(egui::Event, Instant, egui::Pos2)>,
    /// The emulated pointer's press went out and has not been released.
    pressed: bool,
    /// The emulated pointer's last position.
    pointer: Option<egui::Pos2>,
    /// A gesture happened since the hand was last fully off the screen: keep the pointer quiet.
    quiet: bool,
}

impl TouchScroll {
    /// Rewrite one frame's events.
    pub fn apply(&mut self, raw: &mut egui::RawInput, modifiers: egui::Modifiers) {
        self.apply_at(raw, modifiers, Instant::now());
    }

    /// When to look again without new input: a held press is waiting for its timeout.
    pub fn repaint_after(&self) -> Option<Duration> {
        self.held.as_ref().map(|(_, at, _)| HOLD.saturating_sub(at.elapsed()) + Duration::from_millis(5))
    }

    fn apply_at(&mut self, raw: &mut egui::RawInput, modifiers: egui::Modifiers, now: Instant) {
        let events = std::mem::take(&mut raw.events);
        for event in &events {
            if let egui::Event::Touch { id, phase, pos, .. } = event {
                match phase {
                    egui::TouchPhase::Start | egui::TouchPhase::Move => {
                        self.fingers.insert(id.0, *pos);
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        self.fingers.remove(&id.0);
                    }
                }
            }
        }
        let gesture = self.fingers.len() >= 2;
        let centroid = self.centroid();
        let delta = match (gesture, centroid, self.last) {
            (true, Some(now), Some(prev)) => now - prev,
            _ => egui::Vec2::ZERO,
        };

        let mut out = Vec::with_capacity(events.len() + 2);
        if gesture && !self.quiet {
            // The gesture starts: whatever the first finger began is called off.
            self.quiet = true;
            self.held = None;
            if self.pressed {
                if let Some(pos) = self.pointer {
                    out.push(release(pos, modifiers));
                }
                self.pressed = false;
            }
        }

        for event in events {
            match &event {
                egui::Event::PointerMoved(pos) => {
                    if self.quiet {
                        continue;
                    }
                    self.pointer = Some(*pos);
                    out.push(event.clone());
                    // Moving far enough means a drag, not the start of a two-finger gesture.
                    if let Some((_, _, start)) = &self.held
                        && (*pos - *start).length() > SLOP
                    {
                        let (press, _, _) = self.held.take().expect("checked above");
                        out.insert(out.len() - 1, press);
                        self.pressed = true;
                    }
                }
                egui::Event::PointerButton { pressed, pos, .. } => {
                    if self.quiet {
                        continue;
                    }
                    if *pressed {
                        self.held = Some((event.clone(), now, *pos));
                    } else {
                        // A tap: the held press goes out right before its release.
                        if let Some((press, _, _)) = self.held.take() {
                            out.push(press);
                        }
                        out.push(event.clone());
                        self.pressed = false;
                    }
                }
                _ => out.push(event),
            }
        }

        // The hold ran out with the finger still down and alone: it is a drag (or long press).
        if let Some((_, at, _)) = &self.held
            && now.duration_since(*at) >= HOLD
            && !gesture
        {
            let (press, _, _) = self.held.take().expect("checked above");
            out.push(press);
            self.pressed = true;
        }

        if gesture && delta != egui::Vec2::ZERO {
            self.scrolling = true;
            out.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, phase: egui::TouchPhase::Move, modifiers });
        } else if self.scrolling && !gesture {
            self.scrolling = false;
            out.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::Vec2::ZERO, phase: egui::TouchPhase::End, modifiers });
        }

        self.last = if gesture { centroid } else { None };
        if self.fingers.is_empty() {
            self.quiet = false;
            self.held = None;
            self.pressed = false;
        }
        raw.events = out;
    }

    fn centroid(&self) -> Option<egui::Pos2> {
        let n = self.fingers.len();
        if n == 0 {
            return None;
        }
        let sum = self.fingers.values().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2());
        Some((sum / n as f32).to_pos2())
    }
}

fn release(pos: egui::Pos2, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: u64, phase: egui::TouchPhase, x: f32, y: f32) -> egui::Event {
        egui::Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId::from(id), phase, pos: egui::pos2(x, y), force: None }
    }
    fn moved(x: f32, y: f32) -> egui::Event {
        egui::Event::PointerMoved(egui::pos2(x, y))
    }
    fn button(x: f32, y: f32, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }
    fn wheels(raw: &egui::RawInput) -> Vec<(egui::Vec2, egui::TouchPhase)> {
        raw.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { delta, phase, .. } => Some((*delta, *phase)),
                _ => None,
            })
            .collect()
    }
    fn presses(raw: &egui::RawInput) -> Vec<bool> {
        raw.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerButton { pressed, .. } => Some(*pressed),
                _ => None,
            })
            .collect()
    }
    fn batch(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { events, ..Default::default() }
    }
    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    #[test]
    fn a_tap_selects() {
        let (mut s, t0) = (TouchScroll::default(), Instant::now());
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Start, 100.0, 100.0), moved(100.0, 100.0), button(100.0, 100.0, true)]);
        s.apply_at(&mut raw, Default::default(), t0);
        assert!(presses(&raw).is_empty(), "the press is held back for a moment");
        let mut raw = batch(vec![touch(1, egui::TouchPhase::End, 100.0, 100.0), button(100.0, 100.0, false), egui::Event::PointerGone]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 60));
        assert_eq!(presses(&raw), vec![true, false], "on lift the press and release go out together: a click");
    }

    #[test]
    fn one_finger_drag_still_selects() {
        let (mut s, t0) = (TouchScroll::default(), Instant::now());
        s.apply_at(&mut batch(vec![touch(1, egui::TouchPhase::Start, 100.0, 100.0), moved(100.0, 100.0), button(100.0, 100.0, true)]), Default::default(), t0);
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Move, 140.0, 220.0), moved(140.0, 220.0)]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 30));
        assert_eq!(presses(&raw), vec![true], "moving past the slop releases the press");
        let i_press = raw.events.iter().position(|e| matches!(e, egui::Event::PointerButton { .. })).unwrap();
        let i_move = raw.events.iter().position(|e| matches!(e, egui::Event::PointerMoved(_))).unwrap();
        assert!(i_press < i_move, "press first, then the drag");
        assert!(wheels(&raw).is_empty(), "one finger does not scroll");
    }

    #[test]
    fn a_held_finger_starts_a_drag_after_the_hold() {
        let (mut s, t0) = (TouchScroll::default(), Instant::now());
        s.apply_at(&mut batch(vec![touch(1, egui::TouchPhase::Start, 100.0, 100.0), moved(100.0, 100.0), button(100.0, 100.0, true)]), Default::default(), t0);
        assert!(s.repaint_after().is_some());
        let mut raw = batch(vec![]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 200));
        assert_eq!(presses(&raw), vec![true]);
    }

    #[test]
    fn two_fingers_scroll_both_ways_and_never_select() {
        let (mut s, t0) = (TouchScroll::default(), Instant::now());
        s.apply_at(&mut batch(vec![touch(1, egui::TouchPhase::Start, 100.0, 100.0), moved(100.0, 100.0), button(100.0, 100.0, true)]), Default::default(), t0);
        let mut raw = batch(vec![touch(2, egui::TouchPhase::Start, 200.0, 100.0)]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 40));
        assert!(presses(&raw).is_empty() && wheels(&raw).is_empty(), "the held press is dropped, nothing scrolls yet");
        // Diagonal: 30 right, 30 down.
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Move, 130.0, 130.0), touch(2, egui::TouchPhase::Move, 230.0, 130.0), moved(130.0, 130.0)]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 300));
        let w = wheels(&raw);
        assert_eq!(w.len(), 1);
        assert!((w[0].0 - egui::vec2(30.0, 30.0)).length() < 1e-3, "horizontal and vertical: {:?}", w[0].0);
        assert!(presses(&raw).is_empty() && !raw.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_))), "the pointer stays still");
        // Second finger lifts first; the first keeps moving: still no selection.
        let mut raw = batch(vec![touch(2, egui::TouchPhase::End, 230.0, 130.0)]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 320));
        assert_eq!(wheels(&raw), vec![(egui::Vec2::ZERO, egui::TouchPhase::End)], "the lift ends the scroll once");
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Move, 160.0, 200.0), moved(160.0, 200.0)]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 600));
        assert!(presses(&raw).is_empty() && !raw.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_))));
        let mut raw = batch(vec![touch(1, egui::TouchPhase::End, 160.0, 200.0), button(160.0, 200.0, false), egui::Event::PointerGone]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 620));
        assert!(presses(&raw).is_empty());
        // Next touch is a fresh start.
        let mut raw = batch(vec![touch(3, egui::TouchPhase::Start, 50.0, 50.0), moved(50.0, 50.0), button(50.0, 50.0, true), touch(3, egui::TouchPhase::End, 50.0, 50.0), button(50.0, 50.0, false)]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 900));
        assert_eq!(presses(&raw), vec![true, false]);
    }

    #[test]
    fn a_late_second_finger_ends_the_drag() {
        let (mut s, t0) = (TouchScroll::default(), Instant::now());
        s.apply_at(&mut batch(vec![touch(1, egui::TouchPhase::Start, 100.0, 100.0), moved(100.0, 100.0), button(100.0, 100.0, true)]), Default::default(), t0);
        s.apply_at(&mut batch(vec![]), Default::default(), ms(t0, 200));
        let mut raw = batch(vec![touch(2, egui::TouchPhase::Start, 200.0, 100.0)]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 400));
        assert_eq!(presses(&raw), vec![false], "the selection drag is released");
    }

    #[test]
    fn a_pinch_does_not_scroll() {
        let (mut s, t0) = (TouchScroll::default(), Instant::now());
        s.apply_at(&mut batch(vec![touch(1, egui::TouchPhase::Start, 400.0, 400.0), touch(2, egui::TouchPhase::Start, 500.0, 400.0)]), Default::default(), t0);
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Move, 300.0, 400.0), touch(2, egui::TouchPhase::Move, 600.0, 400.0)]);
        s.apply_at(&mut raw, Default::default(), ms(t0, 20));
        assert!(wheels(&raw).is_empty());
        assert!(raw.events.iter().any(|e| matches!(e, egui::Event::Touch { .. })), "egui still sees the touches for zoom");
    }
}
