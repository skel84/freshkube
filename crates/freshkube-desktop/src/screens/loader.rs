//! A screen's [`Loader`](freshkube_reads::Loader): its data for the target
//! node, read within [`SCREEN_DEADLINE`].

use std::{
    future::Future,
    ops::{Deref, DerefMut},
};

use gpui_kit::*;
use tokio::runtime::Handle;

use super::SCREEN_DEADLINE;
use crate::backend::Target;

/// One screen's data for one target. Everything but [`load`](Self::load)
/// is the shared loader's.
pub(crate) struct Loader<T>(freshkube_reads::Loader<T, Target>);

impl<T> Default for Loader<T> {
    fn default() -> Self {
        Self(freshkube_reads::Loader::default())
    }
}

impl<T> Deref for Loader<T> {
    type Target = freshkube_reads::Loader<T, Target>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Loader<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Send + 'static> Loader<T> {
    /// Runs `work` on Tokio for `identity` within [`SCREEN_DEADLINE`]; see
    /// [`freshkube_reads::Loader::load`].
    pub(crate) fn load<V: 'static>(
        &mut self,
        identity: Target,
        runtime: &Handle,
        what: &str,
        work: impl Future<Output = Result<T, String>> + Send + 'static,
        slot: fn(&mut V) -> &mut Loader<T>,
        cx: &mut Context<V>,
    ) {
        self.0.load(
            identity,
            runtime,
            SCREEN_DEADLINE,
            what,
            work,
            move |view| &mut slot(view).0,
            cx,
        );
    }
}
