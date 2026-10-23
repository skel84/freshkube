//! What the app keeps for every agent session: the saved per-cluster access
//! and the gate that enforces it. App-wide, like the follows, because the
//! grants outlive any one tab: turning access off from one place ends the
//! reads of every session on that cluster.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use freshkube_agent::Access;
use freshkube_core::agent_reads::AgentAccess;
use gpui_kit::{App, Global};

#[derive(Clone)]
pub(crate) struct AgentHost {
    pub(crate) access: Arc<Access>,
    pub(crate) gate: AgentAccess,
    /// The preferences file the app runs on; `agent.json` and the opencode
    /// data directory sit beside it. None without preferences.
    pub(crate) preferences: Option<PathBuf>,
}

impl Global for AgentHost {}

static UNSAVED: AtomicU32 = AtomicU32::new(0);

impl AgentHost {
    /// Opens `agent.json` beside `preferences`. Without preferences (a test,
    /// or a window that saves nothing) the grants live in a directory of
    /// their own under the system's temporary one, so nothing is written
    /// into the user's files.
    pub(crate) fn open(preferences: Option<&Path>) -> Self {
        let gate = AgentAccess::new();
        let file = match preferences {
            Some(path) => path.to_path_buf(),
            None => std::env::temp_dir()
                .join(format!(
                    "freshkube-agent-{}-{}",
                    std::process::id(),
                    UNSAVED.fetch_add(1, Ordering::SeqCst)
                ))
                .join("preferences.json"),
        };
        Self {
            access: Arc::new(Access::open(&file, &gate)),
            gate,
            preferences: preferences.map(Path::to_path_buf),
        }
    }

    /// The app's host, opened on first use without preferences if the shell
    /// installed none.
    pub(crate) fn get(cx: &mut App) -> Self {
        if cx.try_global::<Self>().is_none() {
            cx.set_global(Self::open(None));
        }
        cx.global::<Self>().clone()
    }

    /// Makes the app's host, and tries one more save at a clean quit if
    /// turning access off still hasn't been saved.
    pub(crate) fn install(preferences: Option<&Path>, cx: &mut App) {
        let host = Self::open(preferences);
        let access = host.access.clone();
        cx.on_app_quit(move |_| {
            if access.save_pending()
                && let Err(error) = access.retry_save()
            {
                tracing::warn!("agent access could not be saved at quit: {error}");
            }
            async {}
        })
        .detach();
        cx.set_global(host);
    }

    /// Where opencode keeps its data when Freshkube owns it: beside the
    /// preferences. None without preferences.
    pub(crate) fn own_data(&self) -> PathBuf {
        match &self.preferences {
            Some(path) => path.with_file_name("opencode"),
            None => std::env::temp_dir().join("freshkube-opencode"),
        }
    }
}
