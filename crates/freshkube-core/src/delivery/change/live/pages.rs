//! Where Argo CD and Kargo serve their own pages, as the cluster records it:
//! `argocd-cm`'s `url` in Argo CD's namespace, and `kargo-api`'s
//! `ADMIN_ACCOUNT_TOKEN_ISSUER` in Kargo's, which its chart sets to the API's
//! base address. Each is one GET of a ConfigMap, and only that key is kept;
//! no Secret is ever read. An address that isn't there, or isn't http or
//! https, is why its links are greyed out. Each GET has its own short
//! deadline: one that hangs greys its links out, and never holds up the
//! change.

use std::time::Duration;

use crate::delivery::address::Address;
use crate::delivery::read::{Reader, Resource};
use crate::delivery::source::Source;

/// How long an address's GET may take before its links are greyed out.
pub const DEADLINE: Duration = Duration::from_secs(5);

/// Kargo's chart installs into its release's namespace, `kargo` by default.
pub const KARGO_NAMESPACE: &str = "kargo";

/// The tools' addresses, or why each isn't known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pages {
    pub argocd: Result<Address, String>,
    pub kargo: Result<Address, String>,
}

impl Pages {
    /// Neither address was read, as in example data that names none.
    pub fn unread(why: &str) -> Self {
        Self {
            argocd: Err(why.to_owned()),
            kargo: Err(why.to_owned()),
        }
    }
}

pub(super) async fn read<R: Reader>(reader: &R, argocd_namespace: &str) -> Pages {
    let (argocd, kargo) = futures::join!(
        address(reader, argocd_namespace, "argocd-cm", "url"),
        address(
            reader,
            KARGO_NAMESPACE,
            "kargo-api",
            "ADMIN_ACCOUNT_TOKEN_ISSUER"
        ),
    );
    Pages {
        argocd: argocd.map_err(|why| format!("Argo CD's address isn't known: {why}")),
        kargo: kargo.map_err(|why| format!("Kargo's address isn't known: {why}")),
    }
}

/// `key` of ConfigMap `name` in `namespace`, read as an address.
async fn address<R: Reader>(
    reader: &R,
    namespace: &str,
    name: &str,
    key: &str,
) -> Result<Address, String> {
    let resource = Resource::new("", "v1", "configmaps", true);
    let read = tokio::time::timeout(DEADLINE, reader.get(&resource, Some(namespace), name)).await;
    let Ok(read) = read else {
        return Err(format!(
            "ConfigMap {name} in {namespace} didn't answer within {} s",
            DEADLINE.as_secs()
        ));
    };
    let value = match read {
        Ok(value) => value,
        Err(failure) => {
            return Err(match Source::<()>::from_failure(failure) {
                Source::NotInstalled(_) => format!("no ConfigMap {name} in {namespace}"),
                Source::Refused(why) => {
                    format!("ConfigMap {name} in {namespace} is refused: {why}")
                }
                Source::Unreadable(why) => {
                    format!("ConfigMap {name} in {namespace} couldn't be read: {why}")
                }
                Source::Read(()) | Source::Capped(..) => String::new(),
            });
        }
    };
    let raw = value
        .pointer("/data")
        .and_then(|data| data.get(key))
        .and_then(|raw| raw.as_str())
        .filter(|raw| !raw.trim().is_empty())
        .ok_or_else(|| format!("ConfigMap {name} in {namespace} has no {key}"))?;
    Address::parse(raw).map_err(|why| format!("{key} of ConfigMap {name} in {namespace}: {why}"))
}
