//! Motion: the durations and easings every animation uses, where a moving
//! thing is now, the change flash's burst rule, and the OS's reduced-motion
//! setting ([docs/DESIGN.md](../../../docs/DESIGN.md#motion)).
//!
//! Animations here run on the executor's clock, not GPUI's `with_animation`
//! (which times itself by the wall clock from its first frame): a view reads
//! where its motion is now ([`phase`], [`pulse_dim`], [`shimmer_at`],
//! [`fade_left`]) and asks for the next frame with [`next_frame`], which does
//! nothing while `cx.reduce_motion()` is set. Tests then step motion with
//! `advance_clock`, and `with_animation` appears nowhere else
//! (`scripts/check-style.sh`).
//!
//! A frame redraws the view that asked for it, and GPUI marks that view's
//! ancestors dirty too. Draw motion in a small view whose ancestors are cheap
//! or cached, never inside a page's table: the table's `FlashLayer` and
//! `LoadingMotion` are its siblings.

mod flash;
mod system;

pub use flash::{Flash, Flashes};
pub use system::{Choice, choice, choose, follow_system, system};

use gpui_kit::{App, Global, Window, bounce, ease_in_out};
use std::time::{Duration, Instant};

/// No motion: the change shows at once.
pub const INSTANT: Duration = Duration::ZERO;
/// A small acknowledgement: a pane sliding in, a check replacing Copy.
pub const QUICK: Duration = Duration::from_millis(150);
/// The change flash: a changed row's tint fades out over this long.
pub const FADE: Duration = Duration::from_millis(1500);
/// One loading pulse, dim and back: Kit's skeleton cadence.
pub const PULSE: Duration = Duration::from_secs(2);
/// One loading shimmer, a band crossing the rows once.
pub const SHIMMER: Duration = Duration::from_millis(1400);

/// How many changes may flash within one [`FADE`]. A batch that takes the
/// window past it doesn't flash at all: a burst is a reload, not news.
pub const FLASH_BURST: usize = 8;
/// The flash's tint at its start, as a status tint's opacity.
pub const FLASH_TINT: f32 = 0.16;
/// How far the pulse dims, as Kit's skeleton does.
pub const PULSE_DIM: f32 = 0.5;

/// [`QUICK`]'s easing: fast out, settling at the end.
pub fn quick_easing(delta: f32) -> f32 {
    1. - (1. - delta).powi(5)
}

/// [`FADE`]'s easing: the tint holds first, then lets go.
pub fn fade_easing(delta: f32) -> f32 {
    delta * delta
}

/// [`SHIMMER`]'s easing: an even sweep.
pub fn shimmer_easing(delta: f32) -> f32 {
    delta
}

/// The moment every loop counts from, so loops in different views move
/// together.
struct Epoch(Instant);

impl Global for Epoch {}

/// Where a loop of `period` is now, from 0 to 1, on the executor's clock.
pub fn phase(period: Duration, cx: &mut App) -> f32 {
    let now = cx.background_executor().now();
    let epoch = match cx.try_global::<Epoch>() {
        Some(epoch) => epoch.0,
        None => {
            cx.set_global(Epoch(now));
            now
        }
    };
    let period = period.as_secs_f64();
    let elapsed = now.saturating_duration_since(epoch).as_secs_f64();
    ((elapsed % period) / period) as f32
}

/// How far loading bars are dimmed now, from 0 to [`PULSE_DIM`] and back
/// over [`PULSE`], as Kit's skeleton pulses.
pub fn pulse_dim(cx: &mut App) -> f32 {
    bounce(ease_in_out)(phase(PULSE, cx)) * PULSE_DIM
}

/// Where the loading shimmer is in its sweep now, from 0 to 1 over
/// [`SHIMMER`].
pub fn shimmer_at(cx: &mut App) -> f32 {
    shimmer_easing(phase(SHIMMER, cx))
}

/// How much of the change flash is left `since` its change: 1 at the
/// change, 0 from [`FADE`] on.
pub fn fade_left(since: Duration) -> f32 {
    let delta = (since.as_secs_f32() / FADE.as_secs_f32()).min(1.);
    1. - fade_easing(delta)
}

/// Asks the next frame for the view drawing now, unless motion is reduced.
/// Call it from `render` while that view's motion moves.
pub fn next_frame(window: &Window, cx: &App) {
    if !cx.reduce_motion() {
        window.request_animation_frame();
    }
}

#[cfg(test)]
mod tests;
