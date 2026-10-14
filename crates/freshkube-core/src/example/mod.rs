//! UI-independent example builders for node inspection.

use crate::diagnostic_runner::{LoadAverageSnapshot, MemorySnapshot, ServiceSnapshot};

mod diagnostics;
mod processes;

pub use diagnostics::diagnostics;
pub use processes::processes;

/// The node facts example data is built from.
pub struct ExampleNode {
    name: String,
    control_plane: bool,
    cores: Option<usize>,
    address: String,
    memory: Option<MemorySnapshot>,
    load: Option<LoadAverageSnapshot>,
    services: Vec<ServiceSnapshot>,
}

impl ExampleNode {
    /// Creates the node facts needed by the example builders.
    pub fn new(name: impl Into<String>, control_plane: bool, cores: Option<usize>) -> Self {
        Self {
            name: name.into(),
            control_plane,
            cores,
            address: String::new(),
            memory: None,
            load: None,
            services: Vec::new(),
        }
    }

    /// Adds the node-local facts needed by the Diagnostics example builder.
    pub fn with_diagnostics(
        mut self,
        address: impl Into<String>,
        memory: Option<MemorySnapshot>,
        load: Option<LoadAverageSnapshot>,
        services: Vec<ServiceSnapshot>,
    ) -> Self {
        self.address = address.into();
        self.memory = memory;
        self.load = load;
        self.services = services;
        self
    }
}
