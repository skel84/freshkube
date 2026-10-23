use super::*;

#[tokio::test]
async fn a_refused_proxy_is_refused_not_missing() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (200, list(vec![operated()])),
        _ => (
            403,
            status(
                403,
                "services \"prometheus-operated:9090\" is forbidden: User \"alice\" cannot get resource \"services/proxy\"",
            ),
        ),
    });
    let error = discover(&client, None).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(
        error.message,
        "Not allowed to query Prometheus (services/proxy)"
    );
    assert!(error.is_permanent());
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_discovery_proxy_403_keeps_the_refusal_kind_and_words() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (200, list(vec![operated()])),
        _ => (403, status(403, "the proxy says no")),
    });
    let error = discover(&client, None).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Refused);
    assert_eq!(
        error.message,
        "Not allowed to query Prometheus (services/proxy)"
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_discovery_proxy_404_keeps_the_not_found_kind_and_words() {
    let (client, seen) = fake(|_, path, _| match path {
        "/api/v1/services" => (200, list(vec![operated()])),
        _ => (404, "the proxy says no".into()),
    });
    let Discovery::Missing { candidates, tried } = discover(&client, None).await.unwrap() else {
        panic!("an unserved endpoint must remain a discovery candidate, never ready");
    };
    assert_eq!(candidates.len(), 1);
    assert_eq!(tried.len(), 1);
    assert_eq!(tried[0].error.kind, ErrorKind::NotFound);
    assert_eq!(tried[0].error.message, "Not found: the proxy says no");
    assert_eq!(tried[0].service, candidates[0].service);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

/// A cluster whose client never builds, counting what it was told to
/// forget.
#[derive(Default)]
struct Forgets(std::sync::atomic::AtomicUsize);

impl crate::cluster_source::KubeClientSource for Forgets {
    fn client(&self) -> futures::future::BoxFuture<'_, Result<kube::Client, Failure>> {
        Box::pin(async { Err(Failure::new(FailureKind::Config, "No kubeconfig")) })
    }

    fn forget(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Forgets {
    fn count(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[tokio::test]
async fn a_401_forgets_the_client_and_a_403_keeps_it() {
    for (code, kind, forgets) in [
        (401, ErrorKind::Unauthorized, 1),
        (403, ErrorKind::Refused, 0),
    ] {
        let (client, _) = fake(move |_, path, _| match path {
            "/api/v1/services" => (200, list(vec![operated()])),
            _ => (code, status(code, "no")),
        });
        let error = discover(&client, None).await.err().unwrap();
        assert_eq!(error.kind, kind, "{code}");
        // Both say the same and stop discovery: neither is tried again.
        assert_eq!(
            error.message,
            "Not allowed to query Prometheus (services/proxy)"
        );
        assert!(error.is_refused() && error.is_permanent(), "{code}");
        let source = Arc::new(Forgets::default());
        forget_after(&ClusterAccess::Live(source.clone()), &error);
        assert_eq!(source.count(), forgets, "{code}");
    }

    // The Service list refused for the credentials is unauthorized too.
    let (client, _) = fake(|_, _, _| (401, status(401, "Unauthorized")));
    let error = discover(&client, None).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Unauthorized);
    assert!(error.is_refused());
}

#[tokio::test]
async fn a_client_that_fails_or_a_lost_connection_forgets_it() {
    let source = Arc::new(Forgets::default());
    let access = ClusterAccess::Live(source.clone());
    let error = cluster_client(&access).await.err().unwrap();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.message, "No kubeconfig");
    assert_eq!(source.count(), 1);

    for kind in [ErrorKind::Unavailable, ErrorKind::TimedOut] {
        forget_after(&access, &QueryError::new(kind, "x"));
    }
    assert_eq!(source.count(), 3);
    // Prometheus's own answers, and what the Service lacks, keep it.
    for kind in [
        ErrorKind::NotFound,
        ErrorKind::BadAnswer,
        ErrorKind::Rejected,
        ErrorKind::Unsupported,
    ] {
        forget_after(&access, &QueryError::new(kind, "x"));
    }
    assert_eq!(source.count(), 3);
    // Example data has nothing to forget.
    forget_after(&ClusterAccess::EXAMPLE, &QueryError::unauthorized());
}
