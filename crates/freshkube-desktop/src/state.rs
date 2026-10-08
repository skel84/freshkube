//! Desktop's name for core's [`Snapshot`](freshkube_core::snapshot::Snapshot),
//! whose identity defaults to the applied configuration.

use crate::backend::AppliedConfig;

pub(crate) use freshkube_core::snapshot::Request;

/// Retains the last successful value only while its target identity is
/// unchanged; by default, the applied configuration.
pub(crate) type Snapshot<T, I = AppliedConfig> = freshkube_core::snapshot::Snapshot<T, I>;

#[cfg(test)]
mod tests {
    use crate::state::Snapshot;

    /// The agent guide's example, through desktop's name for it.
    #[test]
    fn the_agent_guide_example_runs_through_the_alias() {
        let mut state = Snapshot::<u32, &str>::default();
        let initial = state.begin("node-a");
        assert!(state.apply(&initial, Ok(7)));
        let refresh = state.begin("node-a");
        assert!(state.apply(&refresh, Err("offline".into())));
        assert_eq!(state.data(), Some(&7));
        assert!(state.is_stale());

        let replacement = state.begin("node-b");
        assert!(state.data().is_none());
        assert!(!state.apply(&refresh, Ok(99)));
        assert!(state.apply(&replacement, Ok(8)));
    }
}
