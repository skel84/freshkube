//! What the log says of every verified attestation of a change's builds.

use std::collections::BTreeMap;

use freshkube_core::resources::Failure;

use super::{EntryKey, Fetch, LogRead, entry_key, log_index, look_up};
use crate::chains::{Form, config::ChainsConfig};
use crate::tekton::Build;
use crate::verify::{Trust, Verification, verify};

/// Where the log is, as far as it is known.
pub(crate) enum Address {
    /// The setting names it.
    Set(String),
    /// The setting names none; Chains' configuration is read for it.
    FromChains(Result<ChainsConfig, Failure>),
}

/// The log's address: the setting, else what Chains is configured to upload
/// to. A configuration that can't be read is no address; the public log is
/// never assumed.
pub(super) fn resolve(address: Address) -> Result<String, String> {
    match address {
        Address::Set(url) => Ok(url),
        Address::FromChains(Ok(config)) if config.transparency.url_set => {
            Ok(config.transparency.url)
        }
        Address::FromChains(Ok(_)) => {
            Err("chains-config names no transparency.url".to_owned())
        }
        Address::FromChains(Err(failure)) => Err(format!("chains-config {failure}")),
    }
}

/// Asks `fetch` for the entry of each attestation of `builds` that verifies
/// against `trust`. Nothing is fetched for one that doesn't.
pub(crate) async fn read_log<F: Fetch>(
    fetch: &F,
    address: Address,
    trust: &Trust,
    builds: &[Build],
) -> LogRead {
    let log = match resolve(address) {
        Ok(log) => log,
        Err(why) => return LogRead::NoUrl(why),
    };
    let mut entries = BTreeMap::new();
    for build in builds {
        let images = build.images();
        for (object, attestation) in build.attestations() {
            if attestation.form != Form::Run {
                continue;
            }
            let key: EntryKey = entry_key(&object, attestation);
            if entries.contains_key(&key) {
                continue;
            }
            let verified = images.iter().find_map(|image| {
                match verify(trust, attestation, &image.digest) {
                    Verification::Verified(verified) => Some(verified),
                    _ => None,
                }
            });
            let Some(verified) = verified else { continue };
            let index = attestation.log_entry.as_deref().and_then(log_index);
            entries.insert(key, look_up(fetch, &log, index, &verified).await);
        }
    }
    LogRead::Read(entries)
}
