//! Offline native interaction fixture; no cluster credentials are loaded.
fn main() -> color_eyre::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    freshkube_desktop::run(
        freshkube_desktop::GpuiOptions::fixture(),
        runtime.handle().clone(),
    )
}
