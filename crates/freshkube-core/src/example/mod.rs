//! UI-independent example builders for node inspection.

mod processes;

pub use processes::processes;

/// The node facts example data is built from.
pub struct ExampleNode {
    name: String,
    control_plane: bool,
    cores: Option<usize>,
}

impl ExampleNode {
    /// Creates the node facts needed by the example builders.
    pub fn new(name: impl Into<String>, control_plane: bool, cores: Option<usize>) -> Self {
        Self {
            name: name.into(),
            control_plane,
            cores,
        }
    }
}
