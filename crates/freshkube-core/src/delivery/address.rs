//! The address of a tool's own page, made only from what the cluster
//! reports: an annotation, a ConfigMap's key, a Freight's repository.
//!
//! Only an http or https address with a host is one. Whatever else the
//! cluster wrote, it never reaches the browser or the clipboard whole: the
//! user info, the query and the fragment are dropped, since a token or a
//! session may ride in any of them. A page whose route is its fragment, as
//! the Tekton Dashboard's are (`#/namespaces/…`), keeps that route alone
//! ([`Address::parse_route`]), without anything from a `?` on. Why an
//! address isn't one never quotes it, for the same reason.

use url::Url;

/// An http or https address with a host, and no user info, query or
/// fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address(Url);

impl Address {
    /// Reads `raw` as written by the cluster. `Err` says why it can't be
    /// opened, without quoting it.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let mut url = Url::parse(raw.trim()).map_err(|_| "it isn't an address".to_owned())?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("it isn't an http or https address".into());
        }
        if url.host_str().is_none_or(str::is_empty) {
            return Err("it names no host".into());
        }
        // Neither fails on an http or https address with a host.
        _ = url.set_username("");
        _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        Ok(Self(url))
    }

    /// As [`parse`](Self::parse), keeping a fragment that is a route
    /// (`#/…`), up to any `?` in it: a fragment never reaches the server, and
    /// a single-page console finds its page there.
    pub fn parse_route(raw: &str) -> Result<Self, String> {
        let route = Url::parse(raw.trim())
            .ok()
            .and_then(|url| url.fragment().map(str::to_owned))
            .filter(|fragment| fragment.starts_with('/'))
            .map(|fragment| match fragment.split_once('?') {
                Some((route, _)) => route.to_owned(),
                None => fragment,
            });
        let mut address = Self::parse(raw)?;
        address.0.set_fragment(route.as_deref());
        Ok(address)
    }

    /// The host, as the parser wrote it: lower case, an IDN in punycode.
    pub fn host(&self) -> &str {
        self.0.host_str().unwrap_or_default()
    }

    /// This address with `segments` after its path, each one escaped, so a
    /// name from the cluster stays one segment.
    pub fn join<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> Self {
        let mut url = self.0.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        Self(url)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_and_https_are_addresses() {
        for raw in ["https://kargo.example", "http://argocd.test:8080/base/"] {
            assert!(Address::parse(raw).is_ok(), "{raw}");
        }
    }

    #[test]
    fn other_schemes_are_not_and_say_so_without_quoting() {
        for raw in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://files.example/x",
            "data:text/html,hi",
            "ssh://git@git.example/acme/checkout",
            "mailto:ops@example.test",
        ] {
            let why = Address::parse(raw).unwrap_err();
            assert_eq!(why, "it isn't an http or https address", "{raw}");
        }
        for raw in ["", "checkout", "/project/checkout", "git.example/acme"] {
            assert_eq!(
                Address::parse(raw).unwrap_err(),
                "it isn't an address",
                "{raw}"
            );
        }
        assert_eq!(
            Address::parse("https://").unwrap_err(),
            "it isn't an address"
        );
    }

    #[test]
    fn user_info_query_and_fragment_are_dropped() {
        let address =
            Address::parse("https://bot:s3cr3t@git.example/acme/checkout?token=abc&x=1#frag")
                .unwrap();
        assert_eq!(address.as_str(), "https://git.example/acme/checkout");
        let address = Address::parse("http://user@argocd.test/").unwrap();
        assert_eq!(address.as_str(), "http://argocd.test/");
    }

    /// What the WHATWG parser makes of addresses written oddly: the scheme
    /// and host lower-cased, a backslash taken for a slash, an IDN host in
    /// punycode, and an escaped `@` still user info, so dropped.
    #[test]
    fn oddly_written_addresses_parse_as_a_browser_would() {
        for (raw, parsed) in [
            (
                "HTTP://Kargo.Example/Project",
                "http://kargo.example/Project",
            ),
            ("  https://kargo.example/x", "https://kargo.example/x"),
            ("https:kargo.example/x", "https://kargo.example/x"),
            ("https:\\\\kargo.example\\x", "https://kargo.example/x"),
            (
                "https://bot%40acme:pw@git.example/acme",
                "https://git.example/acme",
            ),
            ("https://bücher.example/", "https://xn--bcher-kva.example/"),
        ] {
            assert_eq!(
                Address::parse(raw).map(|a| a.to_string()),
                Ok(parsed.into()),
                "{raw}"
            );
        }
        // An escaped `@` in the host is no host at all.
        assert!(Address::parse("https://git.example%40evil.example/").is_err());
        // Without a scheme there's no address, only a path.
        assert_eq!(
            Address::parse("//kargo.example/x").unwrap_err(),
            "it isn't an address"
        );
    }

    #[test]
    fn a_route_fragment_is_kept_without_its_query() {
        let raw =
            "https://dashboard.example.test/#/namespaces/ci/pipelineruns/run-1?pipelineTask=build";
        assert_eq!(
            Address::parse_route(raw).unwrap().as_str(),
            "https://dashboard.example.test/#/namespaces/ci/pipelineruns/run-1"
        );
        for raw in [
            "https://dashboard.example.test/run#access_token=abc",
            "https://bot:pw@dashboard.example.test/run?token=abc",
        ] {
            assert_eq!(
                Address::parse_route(raw).unwrap().as_str(),
                "https://dashboard.example.test/run",
                "{raw}"
            );
        }
        assert!(Address::parse_route("javascript:x#/a").is_err());
    }

    #[test]
    fn joined_segments_are_escaped_one_by_one() {
        let base = Address::parse("https://kargo.example/").unwrap();
        assert_eq!(
            base.join(["project", "checkout", "stage", "prod-ams"])
                .as_str(),
            "https://kargo.example/project/checkout/stage/prod-ams"
        );
        assert_eq!(
            base.join(["project", "a/b?c#d"]).as_str(),
            "https://kargo.example/project/a%2Fb%3Fc%23d"
        );
        let below = Address::parse("https://tools.example/argo/").unwrap();
        assert_eq!(
            below.join(["applications", "argocd", "checkout"]).as_str(),
            "https://tools.example/argo/applications/argocd/checkout"
        );
    }
}
