//! The node pane's System services tab: a filterable list of the target
//! node's services beside the selected one's details (`view.rs`), and the
//! restart its details offer (`restart.rs`).
use gpui_kit::SharedString;

mod restart;
mod view;

#[derive(Default)]
pub(super) struct ServiceDisplay {
    pub(super) visible: Vec<talos_rs::ServiceInfo>,
    pub(super) labels: [SharedString; 3],
}
