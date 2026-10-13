//! `freshkube-workbench`: the shared components, one story at a time
//! (docs/WORKBENCH.md). A developer tool; the app bundle never holds it.

fn main() {
    // A debug build reads its knobs from the environment; a release build
    // ignores them.
    #[cfg(debug_assertions)]
    let options = {
        freshkube_probe::first_frame::start();
        freshkube_workbench::Options::parse(|name| std::env::var(name).ok())
    };
    #[cfg(not(debug_assertions))]
    let options = freshkube_workbench::Options::default();
    freshkube_workbench::run(options);
}
