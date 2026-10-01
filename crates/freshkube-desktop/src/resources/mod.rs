//! Kubernetes resource browsing: the server-printed table for a kind, kept
//! live by a watch and shown as virtualized rows.

pub(crate) mod direct;
pub(crate) mod example;
pub(crate) mod live;
pub(crate) mod model;
pub(crate) mod navigation;
pub(crate) mod projection;
mod screen;
pub(crate) mod store;

pub(crate) use screen::{KubeAccess, KubeSource, ResourcesScreen};
