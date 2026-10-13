//! Desktop's name for core's [`Snapshot`](freshkube_core::snapshot::Snapshot),
//! whose identity defaults to the applied configuration and whose error to a
//! [`Failure`](freshkube_core::failure::Failure).

use freshkube_core::failure::Failure;

use crate::backend::AppliedConfig;

pub(crate) use freshkube_core::snapshot::Request;

/// Retains the last successful value only while its target identity is
/// unchanged; by default, the applied configuration. A read that still words
/// its own errors names `String` as the third parameter.
pub(crate) type Snapshot<T, I = AppliedConfig, E = Failure> =
    freshkube_core::snapshot::Snapshot<T, I, E>;

#[cfg(test)]
mod tests {
    use crate::state::Snapshot;

    /// The agent guide's example, through desktop's name for it.
    #[test]
    fn the_agent_guide_example_runs_through_the_alias() {
        let mut state = Snapshot::<u32, &str, String>::default();
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
