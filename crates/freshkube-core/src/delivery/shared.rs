//! A [`Reader`] that asks each distinct question once: every later or
//! concurrent request for the same object, listing or discovery document
//! shares the first one's answer, failures included. A read that follows
//! many changes through one cluster wraps its reader in one, so the
//! project's Kargo kinds, Argo CD's Applications, the ConfigMaps and
//! discovery are each read once, however many changes need them.
//!
//! It lives as long as one read and no longer: what it holds is that read's
//! view of the cluster, never a cache across reads. It has no cap of its
//! own: it holds one answer per distinct question the read asks, and each
//! listing at most [`MAX_PAGES`](super::read::MAX_PAGES) pages, as an
//! unshared read would hold them while it joins.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;
use tokio::sync::OnceCell;

use super::read::{ListRequest, Listing, Reader, Resource, ServedGroup};
use crate::resources::Failure;

/// The answer to each question asked so far, or the one being read.
type Answers<K, V> = Mutex<HashMap<K, Arc<OnceCell<Result<V, Failure>>>>>;

pub struct SharedReader<'a, R> {
    reader: &'a R,
    gets: Answers<(Resource, Option<String>, String), Value>,
    lists: Answers<ListRequest, Listing>,
    groups: OnceCell<Result<Vec<ServedGroup>, Failure>>,
    plurals: Answers<(String, String), Vec<String>>,
}

impl<'a, R: Reader> SharedReader<'a, R> {
    pub fn new(reader: &'a R) -> Self {
        Self {
            reader,
            gets: Mutex::default(),
            lists: Mutex::default(),
            groups: OnceCell::new(),
            plurals: Mutex::default(),
        }
    }
}

/// The cell that holds `key`'s answer, made empty when it was never asked.
fn cell<K: Hash + Eq, V>(answers: &Answers<K, V>, key: K) -> Arc<OnceCell<Result<V, Failure>>> {
    answers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(key)
        .or_default()
        .clone()
}

impl<R: Reader> Reader for SharedReader<'_, R> {
    async fn get(
        &self,
        resource: &Resource,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Value, Failure> {
        let key = (
            resource.clone(),
            namespace.map(str::to_owned),
            name.to_owned(),
        );
        cell(&self.gets, key)
            .get_or_init(|| self.reader.get(resource, namespace, name))
            .await
            .clone()
    }

    async fn list(&self, request: &ListRequest) -> Result<Listing, Failure> {
        cell(&self.lists, request.clone())
            .get_or_init(|| self.reader.list(request))
            .await
            .clone()
    }

    /// Not shared: a probe asks whether a collection can be read now, and
    /// is never what a listing answers.
    async fn probe(&self, request: &ListRequest) -> Result<(), Failure> {
        self.reader.probe(request).await
    }

    async fn groups(&self) -> Result<Vec<ServedGroup>, Failure> {
        self.groups
            .get_or_init(|| self.reader.groups())
            .await
            .clone()
    }

    async fn plurals(&self, group: &str, version: &str) -> Result<Vec<String>, Failure> {
        cell(&self.plurals, (group.to_owned(), version.to_owned()))
            .get_or_init(|| self.reader.plurals(group, version))
            .await
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::fixtures::FixtureReader;
    use crate::delivery::read::Scope;

    #[tokio::test]
    async fn each_question_is_asked_once() {
        let fixture = FixtureReader::default()
            .serves("kargo.akuity.io", "v1alpha1", &["stages"])
            .refusing("stages");
        let shared = SharedReader::new(&fixture);
        let request = ListRequest {
            resource: Resource::new("kargo.akuity.io", "v1alpha1", "stages", true),
            scope: Scope::Namespace("storefront".into()),
        };
        let (first, second) = futures::join!(shared.list(&request), shared.list(&request));
        assert!(
            first.is_err() && second.is_err(),
            "the refusal is shared too"
        );
        let _ = shared.groups().await;
        let _ = shared.groups().await;
        let _ = shared.plurals("kargo.akuity.io", "v1alpha1").await;
        let _ = shared.plurals("kargo.akuity.io", "v1alpha1").await;
        let other = ListRequest {
            scope: Scope::Namespace("checkout".into()),
            ..request.clone()
        };
        let _ = shared.list(&other).await;
        assert_eq!(
            *fixture.requests.borrow(),
            [
                "LIST kargo.akuity.io/v1alpha1/stages ns=Some(\"storefront\") selector=None",
                "GET /apis",
                "GET /apis/kargo.akuity.io/v1alpha1",
                "LIST kargo.akuity.io/v1alpha1/stages ns=Some(\"checkout\") selector=None",
            ]
        );
    }

    /// A probe goes to the reader every time, never to a listing's answer.
    #[tokio::test]
    async fn a_probe_is_never_shared() {
        let fixture = FixtureReader::default().serves("kargo.akuity.io", "v1alpha1", &["stages"]);
        let shared = SharedReader::new(&fixture);
        let request = ListRequest {
            resource: Resource::new("kargo.akuity.io", "v1alpha1", "stages", true),
            scope: Scope::Namespace("storefront".into()),
        };
        let _ = shared.list(&request).await;
        assert!(shared.probe(&request).await.is_ok());
        assert!(shared.probe(&request).await.is_ok());
        // The fixture probes by listing.
        assert_eq!(fixture.requests.borrow().len(), 3);
    }
}
