//! A recording fake of the Kubernetes API, for tests.
//!
//! `kube::Client::new` runs a service inside a tower `Buffer` worker task. A
//! `panic!` or `assert!` in a fake service kills only that worker, so the read
//! under test sees an ordinary transport error and the test can still pass: a
//! guard written inside a service guards nothing. A fake therefore records each
//! request (method, path, query and `Accept`) before it answers, answers a
//! route it does not know with a 500, and lets the test assert on
//! [`Recorder::requests`] after the read.
use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};

use http::{Method, Request, Response, header};
use kube::client::Body;

/// One request the fake saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub method: Method,
    pub path: String,
    pub query: String,
    pub accept: String,
    /// The request's `Content-Type`: which kind of patch it is.
    pub content_type: String,
    /// The request body as text; empty where the fake never read it (see
    /// [`client`], which does).
    pub body: String,
}

impl Recorded {
    /// `METHOD path?query`, the form a test compares whole lists in.
    pub fn line(&self) -> String {
        if self.query.is_empty() {
            format!("{} {}", self.method, self.path)
        } else {
            format!("{} {}?{}", self.method, self.path, self.query)
        }
    }
}

/// The shared list of requests, cloned into the service and kept by the test.
#[derive(Clone, Default)]
pub struct Recorder(Arc<Mutex<Vec<Recorded>>>);

impl Recorder {
    /// Records a request, never panics, and returns what was recorded.
    pub fn record<B>(&self, request: &Request<B>) -> Recorded {
        let recorded = Recorded {
            method: request.method().clone(),
            path: request.uri().path().to_owned(),
            query: request.uri().query().unwrap_or("").to_owned(),
            accept: request
                .headers()
                .get(header::ACCEPT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_owned(),
            content_type: request
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_owned(),
            body: String::new(),
        };
        self.0.lock().unwrap().push(recorded.clone());
        recorded
    }

    /// Records a request with its body, which only the caller could read.
    pub fn record_with_body<B>(&self, request: &Request<B>, body: String) -> Recorded {
        let mut recorded = self.record(request);
        recorded.body = body.clone();
        if let Some(last) = self.0.lock().unwrap().last_mut() {
            last.body = body;
        }
        recorded
    }

    /// Every request so far, in order.
    pub fn requests(&self) -> Vec<Recorded> {
        self.0.lock().unwrap().clone()
    }

    /// Every request so far as `METHOD path?query`.
    pub fn lines(&self) -> Vec<String> {
        self.requests().iter().map(Recorded::line).collect()
    }

    /// Fails unless the requests are exactly `expected`, in order.
    #[track_caller]
    pub fn assert_lines(&self, expected: &[&str]) {
        assert_eq!(self.lines(), expected, "the requests the fake API saw");
    }

    /// Fails if any request is not `allowed`, naming the first that is not.
    #[track_caller]
    pub fn assert_all(&self, what: &str, allowed: impl Fn(&Recorded) -> bool) {
        let requests = self.requests();
        if let Some(refused) = requests.iter().find(|request| !allowed(request)) {
            panic!(
                "forbidden request {} ({what}); all requests: {:?}",
                refused.line(),
                self.lines()
            );
        }
    }
}

/// A status and a JSON body.
pub fn json(status: u16, body: impl ToString) -> (u16, String) {
    (status, body.to_string())
}

/// The answer to a route the fake does not know: a 500 Status, never a panic.
/// The request is already recorded, so the test's list shows it.
pub fn unknown(request: &Recorded) -> (u16, String) {
    json(
        500,
        serde_json::json!({"kind":"Status","apiVersion":"v1","status":"Failure","code":500,
            "reason":"InternalError","message":format!("the fake API has no route for {}", request.line())}),
    )
}

/// The response for a status and body, as the client reads it.
pub fn response((status, body): (u16, String)) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.into_bytes()))
        .unwrap()
}

/// A client over a synchronous fake. `route` returns `None` for a path it does
/// not know, which answers [`unknown`].
pub fn client(
    route: impl Fn(&Recorded) -> Option<(u16, String)> + Send + Sync + 'static,
) -> (kube::Client, Recorder) {
    let recorder = Recorder::default();
    let log = recorder.clone();
    let route = Arc::new(route);
    let service = tower::service_fn(move |request: Request<Body>| {
        let log = log.clone();
        let route = route.clone();
        async move {
            let (parts, body) = request.into_parts();
            let bytes = http_body_util::BodyExt::collect(body)
                .await
                .map(|collected| collected.to_bytes())
                .unwrap_or_default();
            let request = Request::from_parts(parts, ());
            let recorded =
                log.record_with_body(&request, String::from_utf8_lossy(&bytes).into_owned());
            let answer = route(&recorded).unwrap_or_else(|| unknown(&recorded));
            Ok::<_, Infallible>(response(answer))
        }
    });
    (kube::Client::new(service, "default"), recorder)
}

/// A client whose server takes every request and never answers, for a test
/// of a read that must end without one. The requests are recorded, and the
/// requests whose wait was given up (their future dropped) are counted.
pub struct Held {
    pub client: kube::Client,
    pub recorder: Recorder,
    abandoned: Arc<std::sync::atomic::AtomicUsize>,
}

impl Held {
    /// How many requests were given up on.
    pub fn abandoned(&self) -> usize {
        self.abandoned.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// Counts a request given up on when its future is dropped.
struct GivenUp(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for GivenUp {
    fn drop(&mut self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

pub fn held() -> Held {
    let recorder = Recorder::default();
    let log = recorder.clone();
    let abandoned = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = abandoned.clone();
    let service = tower::service_fn(move |request: Request<Body>| {
        log.record(&request);
        let given_up = GivenUp(counter.clone());
        async move {
            std::future::pending::<()>().await;
            drop(given_up);
            Ok::<Response<Body>, Infallible>(response((500, String::new())))
        }
    });
    Held {
        client: kube::Client::new(service, "default"),
        recorder,
        abandoned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unknown_route_is_recorded_and_answers_500_without_panicking() {
        let (client, recorder) = client(|request| {
            (request.path == "/version")
                .then(|| json(200, r#"{"major":"1","minor":"32","gitVersion":"v1.32.0"}"#))
        });
        assert!(client.apiserver_version().await.is_ok());
        let secret = client
            .request_text(
                Request::get("/api/v1/namespaces/a/secrets/b?x=1")
                    .body(vec![])
                    .unwrap(),
            )
            .await;
        assert!(secret.is_err());
        recorder.assert_lines(&["GET /version", "GET /api/v1/namespaces/a/secrets/b?x=1"]);
    }

    #[test]
    #[should_panic(expected = "forbidden request GET /api/v1/secrets")]
    fn a_forbidden_request_fails_the_assertion() {
        let recorder = Recorder::default();
        recorder.record(&Request::get("/api/v1/secrets").body(()).unwrap());
        recorder.assert_all("pods only", |request| request.path == "/api/v1/pods");
    }
}
