//! macOS's reachability of the default route, through SystemConfiguration's
//! run-loop callback. One thread owns the run loop for the process.
//!
//! It could not be built or run where it was written; a real VPN or Wi-Fi
//! change is the check (#968).
use std::{net::SocketAddr, sync::Mutex};

use system_configuration::{
    core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes},
    network_reachability::SCNetworkReachability,
};

use super::{Flags, announce};

pub(super) fn start() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("network-reachability".into())
            .spawn(watch);
        if let Err(error) = spawned {
            tracing::warn!("the network watcher did not start: {error}");
        }
    });
}

fn watch() {
    // The address 0.0.0.0 asks about the default route: any network.
    let mut target = SCNetworkReachability::from(SocketAddr::from(([0, 0, 0, 0], 0)));
    let seed = target.reachability().map(|flags| flags.bits()).ok();
    let state = Mutex::new(seed.map_or_else(Flags::default, Flags::seeded));
    let callback = move |flags: system_configuration::network_reachability::ReachabilityFlags| {
        let changed = state
            .lock()
            .map(|mut state| state.observe(flags.bits()))
            .unwrap_or(false);
        if changed {
            announce();
        }
    };
    if target.set_callback(callback).is_err() {
        tracing::warn!("the network watcher could not set its callback");
        return;
    }
    let run_loop = CFRunLoop::get_current();
    // SAFETY: the mode is a valid run loop mode constant.
    if unsafe { target.schedule_with_runloop(&run_loop, kCFRunLoopCommonModes) }.is_err() {
        tracing::warn!("the network watcher could not join its run loop");
        return;
    }
    CFRunLoop::run_current();
}
