//! Offline native interaction fixture; no cluster credentials are loaded.
fn main() -> color_eyre::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    talos_pilot_gpui::run(
        talos_pilot_gpui::GpuiOptions::fixture(),
        runtime.handle().clone(),
    )
}
