//! What the setup finders share (#821): the identity a find is for, and
//! what a person has ignored in this session.
use freshkube_core::workspace::{Chosen, Key};
use std::path::PathBuf;

/// What a find reads, so a find for one setup never answers for another:
/// the parts' sources, the kubeconfig, and whether it is example data. A
/// change to any drops the findings; a change to anything else about the
/// setup derives the suggestions again from the same findings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FindIdentity {
    pub chosen: Vec<Chosen>,
    pub kubeconfig: Option<PathBuf>,
    pub example: bool,
}

/// A suggestion the person ignored. It lasts for the session, across Find
/// and section changes, and is gone at the next launch; nothing is saved.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Ignored {
    /// A destination, by the key that identifies its item.
    Destination(Key),
}
