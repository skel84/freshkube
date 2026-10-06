//! The OS's reduced-motion setting, read at start and whenever it changes,
//! and handed to `cx.set_reduce_motion`, so every `with_animation` and Kit's
//! own animations follow it. A [`Choice`] other than [`Choice::System`]
//! overrides it: the workbench's strip uses that to view either way.
//!
//! Only an app's `run` calls [`follow_system`]; test harnesses open their
//! windows without it and set reduced motion themselves, so nothing here
//! changes it under them.

use gpui_kit::{App, Global};

/// What decides reduced motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// The OS's setting; motion stays on where none can be read.
    #[default]
    System,
    /// Reduced, whatever the OS says.
    Reduced,
    /// Full motion, whatever the OS says.
    Full,
}

#[derive(Default)]
struct Motion {
    /// The OS's last answer: `Some(true)` asks for reduced motion.
    system: Option<bool>,
    choice: Choice,
    following: bool,
}

impl Global for Motion {}

/// Reads the OS's setting now and follows its changes, for the app's life.
/// Calling it again does nothing.
pub fn follow_system(cx: &mut App) {
    super::start_clock(cx);
    let motion = cx.default_global::<Motion>();
    if motion.following {
        return;
    }
    motion.following = true;
    platform::follow(cx);
}

/// Overrides the OS's setting, or follows it again with [`Choice::System`].
pub fn choose(choice: Choice, cx: &mut App) {
    let motion = cx.default_global::<Motion>();
    if motion.choice != choice {
        motion.choice = choice;
        apply(cx);
    }
}

/// The choice in force.
pub fn choice(cx: &App) -> Choice {
    cx.try_global::<Motion>()
        .map(|motion| motion.choice)
        .unwrap_or_default()
}

/// What the OS last said: `Some(true)` for reduced motion, `None` before
/// an answer or where none can be read.
pub fn system(cx: &App) -> Option<bool> {
    cx.try_global::<Motion>().and_then(|motion| motion.system)
}

/// A reader's answer. One that repeats the last changes nothing: macOS
/// announces every accessibility display change, such as contrast, alike.
pub(super) fn reported(reduce: bool, cx: &mut App) {
    let motion = cx.default_global::<Motion>();
    if motion.system != Some(reduce) {
        motion.system = Some(reduce);
        apply(cx);
    }
}

fn apply(cx: &mut App) {
    let motion = cx.default_global::<Motion>();
    let reduce = match motion.choice {
        Choice::System => motion.system.unwrap_or(false),
        Choice::Reduced => true,
        Choice::Full => false,
    };
    if cx.reduce_motion() != reduce {
        // Redraws every window.
        cx.set_reduce_motion(reduce);
    } else {
        // The choice or the OS's answer changed without changing reduced
        // motion; a control showing either draws again.
        cx.refresh_windows();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use futures::StreamExt as _;
    use gpui_kit::App;
    use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
    use objc2_foundation::{NSNotification, NSOperationQueue};
    use std::ptr::NonNull;

    /// `accessibilityDisplayShouldReduceMotion`, and its change notification
    /// on the workspace's notification centre, delivered on the main queue.
    pub(super) fn follow(cx: &mut App) {
        let workspace = NSWorkspace::sharedWorkspace();
        super::reported(workspace.accessibilityDisplayShouldReduceMotion(), cx);
        let (changed, mut changes) = futures::channel::mpsc::unbounded();
        let block = block2::RcBlock::new(move |_: NonNull<NSNotification>| {
            let reduce = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
            changed.unbounded_send(reduce).ok();
        });
        // SAFETY: the name is AppKit's constant, no object is filtered on,
        // and the main queue runs the block on the main thread, where the
        // sender stays.
        let observer = unsafe {
            workspace
                .notificationCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                    None,
                    Some(&NSOperationQueue::mainQueue()),
                    &block,
                )
        };
        // The observer lives as long as the app.
        std::mem::forget(observer);
        cx.spawn(async move |cx| {
            while let Some(reduce) = changes.next().await {
                cx.update(|cx| super::reported(reduce, cx));
            }
        })
        .detach();
    }
}

#[cfg(windows)]
mod platform {
    use gpui_kit::App;
    use gpui_kit::component::Root;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
    };

    /// "Show animations in Windows": `SPI_GETCLIENTAREAANIMATION`.
    fn read() -> Option<bool> {
        let mut animate: i32 = 1;
        // SAFETY: the action writes one BOOL through the pointer it is given.
        let read = unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                (&mut animate as *mut i32).cast(),
                0,
            )
        };
        (read != 0).then_some(animate == 0)
    }

    /// Reads the setting now, and again whenever one of the app's windows
    /// becomes active. Windows announces a change only to a window's
    /// procedure (`WM_SETTINGCHANGE`), which GPUI keeps, and the setting is
    /// changed in another app, so coming back is when it may have changed.
    /// Every Kit window's root is a `Root`.
    pub(super) fn follow(cx: &mut App) {
        if let Some(reduce) = read() {
            super::reported(reduce, cx);
        }
        cx.observe_new::<Root>(|_, window, cx| {
            let Some(window) = window else {
                return;
            };
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active()
                    && let Some(reduce) = read()
                {
                    super::reported(reduce, cx);
                }
            })
            .detach();
        })
        .detach();
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod platform {
    use ashpd::desktop::settings::{ReducedMotion, Settings};
    use futures::future::{Either, select};
    use futures::{FutureExt as _, StreamExt as _};
    use gpui_kit::{App, AsyncApp};
    use std::time::Duration;

    /// How long a portal call may take before motion stays on.
    const PATIENCE: Duration = Duration::from_secs(1);
    const GNOME: (&str, &str) = ("org.gnome.desktop.interface", "enable-animations");

    /// The settings portal, off the UI thread: the appearance namespace's
    /// `reduced-motion`, else GNOME's `enable-animations`, each within
    /// [`PATIENCE`]; then the portal's change signal. With no answer motion
    /// stays on.
    pub(super) fn follow(cx: &mut App) {
        cx.spawn(async move |cx| {
            let Some(settings) = within(cx, Settings::new()).await else {
                return;
            };
            let appearance = within(cx, settings.reduced_motion()).await;
            let reduce = match appearance {
                Some(reduced) => Some(reduced == ReducedMotion::ReducedMotion),
                None => within(cx, settings.read::<bool>(GNOME.0, GNOME.1))
                    .await
                    .map(|animate| !animate),
            };
            let Some(reduce) = reduce else {
                return;
            };
            cx.update(|cx| super::reported(reduce, cx));
            let Some(changes) = within(cx, settings.receive_setting_changed()).await else {
                return;
            };
            let mut changes = std::pin::pin!(changes);
            while let Some(setting) = changes.next().await {
                let reduce = match (setting.namespace(), setting.key()) {
                    ("org.freedesktop.appearance", "reduced-motion") => setting
                        .value()
                        .downcast_ref::<u32>()
                        .ok()
                        .map(|value| value == 1),
                    (namespace, key) if (namespace, key) == GNOME && appearance.is_none() => {
                        setting
                            .value()
                            .downcast_ref::<bool>()
                            .ok()
                            .map(|animate| !animate)
                    }
                    _ => None,
                };
                if let Some(reduce) = reduce {
                    cx.update(|cx| super::reported(reduce, cx));
                }
            }
        })
        .detach();
    }

    /// The call's answer, or `None` when it fails or takes too long.
    async fn within<T, E>(
        cx: &AsyncApp,
        call: impl std::future::Future<Output = Result<T, E>>,
    ) -> Option<T> {
        let timer = cx.background_executor().timer(PATIENCE);
        match select(std::pin::pin!(call.fuse()), timer).await {
            Either::Left((answer, _)) => answer.ok(),
            Either::Right(_) => None,
        }
    }
}

#[cfg(not(any(
    target_os = "macos",
    windows,
    target_os = "linux",
    target_os = "freebsd"
)))]
mod platform {
    /// Nothing to read: motion stays on.
    pub(super) fn follow(_: &mut gpui_kit::App) {}
}
