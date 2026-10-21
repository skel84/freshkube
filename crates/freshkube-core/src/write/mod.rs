//! How the app decides what a person may change in a cluster
//! (`docs/WRITE_MODEL.md`, #852). Today this holds the protection level a
//! cluster entry sets; the tiers and the write framework are #853's.

mod protection;

pub use protection::Protection;
