//! What the log says of every verified attestation of a change's builds.

use std::collections::BTreeMap;

use std::future::Future;

use freshkube_core::resources::Failure;

use super::{EntryKey, Fetch, LogRead, Outcome, entry_key, log_index, look_up};
use crate::chains::{Form, config::ChainsConfig};
use crate::tekton::Build;
use crate::verify::{Trust, Verification, Verified, verify};

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
        Address::FromChains(Ok(_)) => Err("chains-config names no transparency.url".to_owned()),
        Address::FromChains(Err(failure)) => Err(format!("chains-config {failure}")),
    }
}

/// An attestation that verified, and the entry its run names for it.
pub(super) struct Wanted {
    pub(super) key: EntryKey,
    pub(super) index: Option<u64>,
    pub(super) verified: Verified,
}

/// The attestations of `builds` that verify against `trust`, once each.
fn wanted(trust: &Trust, builds: &[Build]) -> Vec<Wanted> {
    let mut wanted: Vec<Wanted> = Vec::new();
    for build in builds {
        let images = build.images();
        for (object, attestation) in build.attestations() {
            let key = entry_key(&object, attestation);
            if attestation.form != Form::Run || wanted.iter().any(|w| w.key == key) {
                continue;
            }
            let verified =
                images
                    .iter()
                    .find_map(|image| match verify(trust, attestation, &image.digest) {
                        Verification::Verified(verified) => Some(verified),
                        _ => None,
                    });
            if let Some(verified) = verified {
                let index = attestation.log_entry.as_deref().and_then(log_index);
                wanted.push(Wanted {
                    key,
                    index,
                    verified,
                });
            }
        }
    }
    wanted
}

/// Asks the log for each entry in turn. One that didn't answer is the
/// answer for the rest: a silent log costs one deadline, not one each.
pub(super) async fn ask<F: Fetch>(
    fetch: Result<&F, &Failure>,
    log: &str,
    wanted: Vec<Wanted>,
) -> BTreeMap<EntryKey, Outcome> {
    let mut down: Option<String> = fetch.err().map(ToString::to_string);
    let mut entries = BTreeMap::new();
    for Wanted {
        key,
        index,
        verified,
    } in wanted
    {
        let outcome = match (&down, fetch) {
            (Some(why), _) => Outcome::Unreachable(why.clone()),
            (None, Ok(fetch)) => look_up(fetch, log, index, &verified).await,
            (None, Err(_)) => unreachable!("`down` holds a failed client"),
        };
        if let Outcome::Unreachable(why) = &outcome {
            down = Some(why.clone());
        }
        entries.insert(key, outcome);
    }
    entries
}

/// Asks `fetch`, or says why there is none, for each attestation of
/// `builds` that verifies against `trust`. Nothing is fetched for one that
/// doesn't, and `address` is not resolved unless one does.
pub(crate) async fn read_log<F: Fetch>(
    fetch: Result<&F, &Failure>,
    address: impl Future<Output = Address>,
    trust: &Trust,
    builds: &[Build],
) -> LogRead {
    let wanted = wanted(trust, builds);
    if wanted.is_empty() {
        return LogRead::Read(BTreeMap::new());
    }
    match resolve(address.await) {
        Ok(log) => LogRead::Read(ask(fetch, &log, wanted).await),
        Err(why) => LogRead::NoUrl(why),
    }
}
