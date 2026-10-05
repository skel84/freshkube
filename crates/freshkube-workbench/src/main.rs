//! `freshkube-workbench`: the shared components, one story at a time
//! (docs/WORKBENCH.md). A developer tool; the app bundle never holds it.

fn main() {
    #[cfg(debug_assertions)]
    freshkube_probe::first_frame::start();
    freshkube_workbench::run();
}
