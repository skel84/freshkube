//! Where a link goes (#583): the objects, tabs and requests a page hands the
//! shell, so a page crate can ask for navigation without naming desktop.
//! Identities and requests only; what the shell does with them, and how a
//! pane or the Nodes table draws a tab, stay in desktop.

mod object;
mod request;
mod tab;

pub use object::{DetailTarget, ObjectRef, ResourceIdentity};
pub use request::{LogsAt, LogsRequest, ResourceLink, ShellRequest};
pub use tab::{NodeTab, Tab};
