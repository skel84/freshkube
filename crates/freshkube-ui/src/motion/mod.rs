//! Motion: the durations and easings every animation uses, the few
//! animations a page may draw, the change flash's burst rule, and the OS's
//! reduced-motion setting ([docs/DESIGN.md](../../../docs/DESIGN.md#motion)).
//!
//! `with_animation` is called only here (`scripts/check-style.sh`), so every
//! animation takes its timing from these tokens. GPUI draws each one still
//! while `cx.reduce_motion()` is set: a one-shot at its end, a loop at its
//! start, and asks no frames for it.
//!
//! An animation asks frames for the view that draws it, and GPUI marks that
//! view's ancestors dirty too. Draw one in a small view whose ancestors are
//! cheap or cached, never inside a page's table.

mod flash;
mod system;

pub use flash::{Flash, Flashes};
pub use system::{Choice, choice, choose, follow_system, system};

use gpui_kit::prelude::*;
use gpui_kit::{Animation, AnimationElement, AnimationExt as _, ElementId, bounce, ease_in_out};
use std::time::Duration;

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

/// The loading pulse: dims to [`PULSE_DIM`] and back over [`PULSE`], on
/// repeat. Synced, so every bar in a view dims together; still at full
/// strength under reduced motion.
pub fn pulse<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(
        id,
        Animation::new(PULSE)
            .repeat_synced()
            .with_easing(bounce(ease_in_out)),
        |element, delta| element.opacity(1. - delta * PULSE_DIM),
    )
}

/// The loading shimmer: `sweep` places the element at `phase` (0 to 1) of
/// its pass over [`SHIMMER`], on repeat. Synced, so every bar's band is
/// one sweep. Under reduced motion it stays at phase 0, so a band should
/// start out of sight.
pub fn shimmer<E>(
    id: impl Into<ElementId>,
    element: E,
    sweep: impl Fn(E, f32) -> E + 'static,
) -> AnimationElement<E>
where
    E: IntoElement + 'static,
{
    element.with_animation(
        id,
        Animation::new(SHIMMER)
            .repeat_synced()
            .with_easing(shimmer_easing),
        sweep,
    )
}

/// The change flash's fade: from full to clear over [`FADE`], once. The id
/// must be new for each change, or the fade won't start again. Under reduced
/// motion it draws its end, clear.
pub fn fade_out<E>(id: impl Into<ElementId>, element: E) -> AnimationElement<E>
where
    E: Styled + IntoElement + 'static,
{
    element.with_animation(
        id,
        Animation::new(FADE).with_easing(fade_easing),
        |element, delta| element.opacity(1. - delta),
    )
}

#[cfg(test)]
mod tests;
