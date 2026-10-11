//! **The icon, which shows Rivet Monitor's OWN staleness** — `SPEC-01` §5.4.
//!
//! *A tool that watches everything is the one thing nothing else watches*, and
//! a tray icon that has gone quiet looks exactly like one with nothing to
//! report. So the icon's state is the age of the **last successful backup**,
//! plus whatever is stopping the next one.
//!
//! # The shape is the Rivet mark; the colour is the state
//!
//! The silhouette — a rounded hexagonal frame with the rivet at its centre —
//! is Rivet's mark, from `Rivet ADR-037`. It says which fleet this program
//! belongs to. The **colour** is the state, and it tints the whole mark rather
//! than a badge in one corner: at 16 px the centre disc is under five pixels
//! across, so a two-tone icon would put the one thing that has to be read at
//! a glance into the part of the icon too small to read.
//!
//! **The geometry is drawn here rather than loaded**, from the same numbers as
//! `docs/brand/rivet-mark-small.svg` in the Rivet repository: circumradius 270
//! and corner radius 30 outside, 186.9 and 20.8 inside, a rivet of radius 90,
//! all on that file's 600-unit square. Signed distances, so it is crisp at any
//! size a taskbar asks for, and there is no asset file to fall out of step
//! with the states it has to carry.
//!
//! # Health is a problem like any other
//!
//! **A machine's health, as its supervisor folds it** (`Rivet ADR-045` §4), is
//! read in the same pass, and a machine that is not healthy is a standing
//! problem — so it turns the icon red the way every problem does, and the
//! menu's problems list says which (`SPEC-01` §5.4.1, §5.8). The icon does not
//! grow a state for it: one colour for "something is wrong", and the list for
//! what.

/// What the icon is saying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Backups are current and nothing is complaining.
    Current,
    /// Running, but the last success is older than it should be.
    Stale,
    /// Something stops a backup happening at all — no folder, no tool, an
    /// override matching nothing.
    Refused,
    /// Nothing has been decided yet: the first pass has not finished.
    Unknown,
    /// **This program's own backup worker has stopped**, so every other colour
    /// here would be a stale judgement rendered as a live one.
    ///
    /// Added 2026-09-18, the day it happened. The worker runs on its own
    /// thread; it died, and the tray went on showing the colour it held at that
    /// moment for hours, while the machine it was
    /// reporting on came back and ran perfectly. **A red that cannot change is
    /// indistinguishable from a red that is correct**, which is worse than no
    /// colour at all — and it is the failure this program had just finished
    /// diagnosing in `rivet-sysd`: the watcher stopped, everything it watched
    /// kept running, and the only thing that would have noticed was the thing
    /// that stopped.
    ///
    /// **No pass computes this.** It is decided by the tray's own event loop,
    /// which is alive whether or not the worker is, from how long it has been
    /// since a pass last published anything.
    Stopped,
}

impl State {
    const fn colour(self) -> [u8; 3] {
        match self {
            Self::Current => [63, 185, 80],
            Self::Stale => [210, 153, 34],
            Self::Refused => [248, 81, 73],
            Self::Unknown => [139, 148, 158],
            // **Black**, ruled by the maintainer 2026-09-18. Not another shade
            // of a colour already here: this one does not mean *the fleet is
            // unwell*, it means *do not believe this icon*, and a reader must
            // not be able to mistake it for a reading. A dead mark reads as
            // dead.
            Self::Stopped => [0, 0, 0],
        }
    }
}

/// The icon's edge, in pixels.
const SIZE: u32 = 32;

/// How much of the mark's 600-unit square is shown. Less than 600, because the
/// mark is 540 units tall inside it and a tray icon that leaves a tenth of its
/// height as margin has thrown away the pixels it needed most.
const VIEW: f32 = 560.0;

/// The outer hexagon, as the flat-edge distance of the *unrounded* shape the
/// corner radius is then swelled out from: apothem 233.827 less the radius.
const OUTER_CORE: f32 = 203.827;
/// The outer corner radius, from the mark.
const OUTER_CORNER: f32 = 30.0;
/// The inner hexagon — the hole — by the same construction: apothem 161.827
/// less its corner radius.
const INNER_CORE: f32 = 141.065;
/// The inner corner radius, from the mark.
const INNER_CORNER: f32 = 20.762;
/// The rivet at the centre.
const RIVET: f32 = 90.0;

/// Signed distance to a regular hexagon centred on the origin with **vertical**
/// flat edges at `±apothem` and vertices top and bottom — the mark's
/// orientation. Negative inside.
///
/// Inigo Quilez's hexagon distance, with the axes swapped: his is flat top and
/// bottom, and the mark is flat left and right.
fn hexagon_sd(x: f32, y: f32, apothem: f32) -> f32 {
    // The hexagon's three mirror symmetries reduce the whole plane to one
    // sixth of it, which is why this is a fold and then a single edge.
    const KX: f32 = -0.866_025_4;
    const KY: f32 = 0.5;
    const KZ: f32 = 0.577_350_3;

    let mut px = y.abs();
    let mut py = x.abs();
    let fold = (KX * px + KY * py).min(0.0);
    px -= 2.0 * fold * KX;
    py -= 2.0 * fold * KY;
    px -= px.clamp(-KZ * apothem, KZ * apothem);
    py -= apothem;
    px.hypot(py) * if py < 0.0 { -1.0 } else { 1.0 }
}

/// How much of a pixel a signed distance covers, one pixel of feather at the
/// edge so the mark does not look like a sprite from 1994 on a high-DPI
/// taskbar. `unit` is the pixel's width in the mark's own units.
fn coverage(distance: f32, unit: f32) -> f32 {
    (0.5 - distance / unit).clamp(0.0, 1.0)
}

/// **The Rivet mark in the state's colour**, anti-aliased.
///
/// Returned as straight RGBA, which is what `tray_icon::Icon::from_rgba` takes.
#[must_use]
pub fn rgba(state: State) -> Vec<u8> {
    let [r, g, b] = state.colour();
    let edge = SIZE as f32;
    let unit = VIEW / edge;
    let half = edge / 2.0;
    let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let ux = ((x as f32) + 0.5 - half) * unit;
            let uy = ((y as f32) + 0.5 - half) * unit;
            let outer = hexagon_sd(ux, uy, OUTER_CORE) - OUTER_CORNER;
            let inner = hexagon_sd(ux, uy, INNER_CORE) - INNER_CORNER;
            // The frame is inside the outer hexagon AND outside the inner one;
            // the rivet is added to it, which is what `fill-rule="evenodd"`
            // means for these three subpaths.
            let frame = coverage(outer, unit).min(coverage(-inner, unit));
            let alpha = frame.max(coverage(ux.hypot(uy) - RIVET, unit));
            out.extend_from_slice(&[r, g, b, (alpha * 255.0) as u8]);
        }
    }
    out
}

/// The icon's edge, for the caller that has to say it twice.
#[must_use]
pub const fn size() -> u32 {
    SIZE
}

/// **Decide what the icon says**, from the pass and the ledger.
///
/// `late` is whether any application the pass watched is late by
/// `schedule::Watched::late` — the same rule the announcement uses, so the two
/// cannot disagree. A scheduled application never backed up is late, so
/// *nothing ever stored* is still amber rather than green.
#[must_use]
pub fn state_for(refused: bool, alerts: usize, late: bool) -> State {
    if refused || alerts > 0 {
        State::Refused
    } else if late {
        State::Stale
    } else {
        State::Current
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    /// The alpha of one pixel, by its coordinates.
    fn alpha_at(pixels: &[u8], x: u32, y: u32) -> u8 {
        pixels[((y * SIZE + x) * 4 + 3) as usize]
    }

    #[test]
    fn the_buffer_is_the_size_from_rgba_promises() {
        assert_eq!(rgba(State::Current).len(), (SIZE * SIZE * 4) as usize);
    }

    /// **It has to be the mark, not a disc**, and the three features that make
    /// it one are the rivet, the gap around it, and the frame outside the gap.
    /// A solid disc passes the first and third and fails the second.
    #[test]
    fn the_rivet_the_gap_and_the_frame_are_all_there() {
        let pixels = rgba(State::Current);
        // The rivet, dead centre.
        assert_eq!(alpha_at(&pixels, 16, 16), 255);
        // The gap between the rivet and the frame: 113.75 units out, which is
        // past the rivet's 90 and well inside the hole's 161.8.
        assert_eq!(alpha_at(&pixels, 22, 16), 0);
        // The frame itself: 201.25 units out, between the two hexagons.
        assert_eq!(alpha_at(&pixels, 27, 16), 255);
        // Outside the mark altogether. A hexagon has corners a disc does not.
        assert_eq!(alpha_at(&pixels, 0, 0), 0);
    }

    /// **The state is carried by colour alone**, so every state must draw the
    /// same shape — otherwise the icon changing would read as a different
    /// program rather than a different situation.
    #[test]
    fn every_state_draws_the_same_silhouette() {
        let reference: Vec<u8> = rgba(State::Current)
            .chunks_exact(4)
            .map(|p| p[3])
            .collect();
        for state in [State::Stale, State::Refused, State::Unknown] {
            let alphas: Vec<u8> = rgba(state).chunks_exact(4).map(|p| p[3]).collect();
            assert_eq!(alphas, reference, "{state:?} drew a different shape");
        }
    }

    /// **A refusal outranks everything**, because a backup that cannot run is
    /// worse news than one that ran a while ago.
    #[test]
    fn a_refusal_is_shown_even_when_a_backup_is_recent() {
        assert_eq!(state_for(true, 0, false), State::Refused);
        assert_eq!(state_for(false, 1, false), State::Refused);
        assert_eq!(state_for(true, 0, true), State::Refused);
    }

    #[test]
    fn anything_late_is_stale_and_nothing_late_is_current() {
        assert_eq!(state_for(false, 0, true), State::Stale);
        assert_eq!(state_for(false, 0, false), State::Current);
    }
}
