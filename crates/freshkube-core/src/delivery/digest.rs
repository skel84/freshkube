//! Image digests, the one key that joins the build, the freight and the pod.
//! Tags are parsed out only to be shown.

use serde_json::Value;

/// A normalized `sha256:<64 lower-case hex>` content digest.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(String);

impl Digest {
    /// Accepts `sha256:<hex>` in any case; anything else is not a digest.
    pub fn parse(text: &str) -> Option<Self> {
        let (algorithm, hex) = text.trim().split_once(':')?;
        if !algorithm.eq_ignore_ascii_case("sha256")
            || hex.len() != 64
            || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        Some(Self(format!("sha256:{}", hex.to_ascii_lowercase())))
    }

    /// The digest of an image reference such as `registry.example/app@sha256:…`,
    /// or of a pod's `imageID` (`docker-pullable://…@sha256:…`). A reference
    /// with only a tag has no digest.
    pub fn from_reference(reference: &str) -> Option<Self> {
        let (_, digest) = reference.rsplit_once('@')?;
        Self::parse(digest)
    }

    /// The digest a pod's `imageID` reports, in any of the forms runtimes
    /// write: `docker-pullable://…@sha256:…`, `…@sha256:…`, or a bare
    /// `sha256:…`. Only the `sha256:` part is kept, so the forms compare
    /// equal.
    pub fn from_image_id(image_id: &str) -> Option<Self> {
        let id = image_id.trim();
        let id = id.split_once("://").map_or(id, |(_, rest)| rest);
        Self::parse(id.rsplit_once('@').map_or(id, |(_, digest)| digest))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A short form for tables: `sha256:` and the first 12 digits.
    pub fn short(&self) -> String {
        self.0.chars().take("sha256:".len() + 12).collect()
    }
}

impl std::fmt::Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A full commit SHA: 40 hexadecimal digits (SHA-1) or 64 (SHA-256). A
/// shortened one could name more than one commit, so nothing joins on it.
/// Builds are found only for SHA-1 commits; see
/// [`read_builds`](super::tekton::read_builds).
pub fn is_full_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The repository of an image reference, normalized as a container runtime
/// reads it, the one key two references are compared by: no tag, no digest,
/// Docker Hub's spellings folded into one. `acme/api`, `docker.io/acme/api`
/// and `index.docker.io/acme/api` are all `docker.io/acme/api`, and a
/// single-name image gains `library/`. Only the registry host is lowercased;
/// its port stays, so `registry.example:5000` is another registry. A name the
/// reference grammar rejects stays as written, since a false match is worse
/// than a miss.
pub fn repository(reference: &str) -> String {
    let written = written_repository(reference);
    normalized(&written).unwrap_or(written)
}

/// Docker Hub, as references name it when they name no registry.
const DOCKER_HUB: &str = "docker.io";

/// The repository in its normalized form, or `None` when the name isn't one
/// the reference grammar accepts.
fn normalized(name: &str) -> Option<String> {
    // The first segment names a registry when it has a dot or a port, or is
    // `localhost`; otherwise the image is on Docker Hub.
    let (registry, path) = match name.split_once('/') {
        Some((first, path)) if first.contains(['.', ':']) || first == "localhost" => {
            (registry_host(first)?, path)
        }
        _ => (DOCKER_HUB.to_owned(), name),
    };
    if !path.split('/').all(is_path_segment) {
        return None;
    }
    let registry = if registry == "index.docker.io" {
        DOCKER_HUB.to_owned()
    } else {
        registry
    };
    if registry == DOCKER_HUB && !path.contains('/') {
        return Some(format!("{DOCKER_HUB}/library/{path}"));
    }
    Some(format!("{registry}/{path}"))
}

/// A registry, `host[:port]`, with its host lowercased (DNS ignores case)
/// and its port as written.
fn registry_host(registry: &str) -> Option<String> {
    let (host, port) = match registry.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (registry, None),
    };
    let label = |label: &str| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if !host.split('.').all(label) {
        return None;
    }
    let host = host.to_ascii_lowercase();
    match port {
        Some(port) if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) => None,
        Some(port) => Some(format!("{host}:{port}")),
        None => Some(host),
    }
}

/// A path segment of a repository: lower-case letters and digits, joined
/// by one `.`, one or two `_`, or any number of `-`.
fn is_path_segment(segment: &str) -> bool {
    // Splitting on letters and digits leaves what joins them, and an empty
    // piece at each end when the segment starts and ends with one.
    let joins: Vec<&str> = segment
        .split(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    joins.len() > 1
        && joins.first().is_some_and(|join| join.is_empty())
        && joins.last().is_some_and(|join| join.is_empty())
        && joins
            .iter()
            .all(|join| matches!(*join, "." | "_" | "__") || join.bytes().all(|b| b == b'-'))
}

/// The repository of an image reference as written: no tag, no digest. It
/// is for showing; compare with [`repository`].
pub fn written_repository(reference: &str) -> String {
    let without_digest = reference.split('@').next().unwrap_or(reference);
    // A colon after the last slash starts a tag; one before it is a port.
    match without_digest.rsplit_once('/') {
        Some((host, last)) => match last.split_once(':') {
            Some((name, _)) => format!("{host}/{name}"),
            None => without_digest.to_owned(),
        },
        None => without_digest
            .split(':')
            .next()
            .unwrap_or(without_digest)
            .to_owned(),
    }
}

/// The tag of an image reference, to be shown and never joined on.
pub fn tag(reference: &str) -> Option<String> {
    let without_digest = reference.split('@').next().unwrap_or(reference);
    let last = without_digest.rsplit('/').next()?;
    last.split_once(':').map(|(_, tag)| tag.to_owned())
}

/// A string at a JSON pointer.
pub(super) fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    #[test]
    fn digests_come_from_references_and_image_ids() {
        let expected = Digest::parse(D).unwrap();
        assert_eq!(
            Digest::from_reference(&format!("registry.example/app-a@{D}")),
            Some(expected.clone())
        );
        assert_eq!(
            Digest::from_reference(&format!("docker-pullable://registry.example/app-a@{D}")),
            Some(expected.clone())
        );
        // Case doesn't matter; the stored form is lower case.
        assert_eq!(
            Digest::from_reference(&format!("registry.example/app-a:v1@{}", D.to_uppercase())),
            Some(expected.clone())
        );
        assert_eq!(
            Digest::from_reference("registry.example/app-a@md5:abc"),
            None
        );
        assert_eq!(Digest::from_reference("registry.example/app-a:v1"), None);
        assert_eq!(Digest::parse("sha256:abc"), None);
        assert_eq!(expected.short(), "sha256:111111111111");
    }

    #[test]
    fn an_image_id_compares_by_its_sha256_part_in_every_form() {
        let expected = Some(Digest::parse(D).unwrap());
        for id in [
            format!("docker-pullable://registry.example/app-a@{D}"),
            format!("registry.example/app-a@{D}"),
            D.to_owned(),
            format!(" {} ", D.to_uppercase()),
        ] {
            assert_eq!(Digest::from_image_id(&id), expected, "{id}");
        }
        for id in [
            "registry.example/app-a:v1",
            "docker://registry.example/app-a:v1",
            "registry.example/app-a@md5:abc",
            "",
        ] {
            assert_eq!(Digest::from_image_id(id), None, "{id}");
        }
    }

    #[test]
    fn docker_hubs_spellings_are_one_repository() {
        for spelling in [
            "acme/api",
            "docker.io/acme/api",
            "index.docker.io/acme/api",
            "Docker.IO/acme/api",
            "acme/api:v1",
            &format!("acme/api@{D}"),
            &format!("index.docker.io/acme/api:v1@{D}"),
        ] {
            assert_eq!(repository(spelling), "docker.io/acme/api", "{spelling}");
        }
        for spelling in [
            "api",
            "api:v1",
            "docker.io/api",
            "index.docker.io/api",
            "docker.io/library/api",
            &format!("library/api@{D}"),
        ] {
            assert_eq!(repository(spelling), "docker.io/library/api", "{spelling}");
        }
    }

    #[test]
    fn a_registry_keeps_its_port_and_only_its_host_ignores_case() {
        for spelling in [
            "registry.example/acme/api",
            "Registry.Example/acme/api:v1",
            &format!("registry.example/acme/api@{D}"),
            &format!("registry.example/acme/api:v1@{D}"),
        ] {
            assert_eq!(
                repository(spelling),
                "registry.example/acme/api",
                "{spelling}"
            );
        }
        assert_eq!(
            repository("registry.example:5000/acme/api:v1"),
            "registry.example:5000/acme/api"
        );
        // A registry's single-name image is not Docker Hub's official one.
        assert_eq!(repository("registry.example/api"), "registry.example/api");
        assert_eq!(repository("localhost/api:v1"), "localhost/api");
        assert_eq!(repository("localhost:5000/api"), "localhost:5000/api");
    }

    #[test]
    fn different_repositories_registries_and_ports_stay_apart() {
        let api = repository("acme/api");
        for other in [
            "acme/web",
            "acme/api.git",
            "acme/api/worker",
            "other/api",
            "api",
            "registry.example/acme/api",
            "registry-1.docker.io/acme/api",
            "docker.io:443/acme/api",
            "localhost/acme/api",
        ] {
            assert_ne!(repository(other), api, "{other}");
        }
        let hosted = repository("registry.example/acme/api");
        for other in [
            "registry.example:5000/acme/api",
            "registry.example:443/acme/api",
            "mirror.registry.example/acme/api",
        ] {
            assert_ne!(repository(other), hosted, "{other}");
        }
    }

    #[test]
    fn a_name_the_grammar_rejects_stays_as_written() {
        for (reference, written) in [
            ("acme/API:v1", "acme/API"),
            ("Acme/api", "Acme/api"),
            ("registry.example/Acme/api", "registry.example/Acme/api"),
            ("registry.example//api", "registry.example//api"),
            ("registry.example/acme/-api", "registry.example/acme/-api"),
            ("registry.example/acme/a..pi", "registry.example/acme/a..pi"),
            (
                "registry.example/acme/a___pi",
                "registry.example/acme/a___pi",
            ),
            ("reg_istry.example/acme/api", "reg_istry.example/acme/api"),
            ("registry.example:/acme/api", "registry.example:/acme/api"),
            ("[::1]:5000/acme/api", "[::1]:5000/acme/api"),
            (
                "oci://registry.example/acme/api",
                "oci://registry.example/acme/api",
            ),
        ] {
            assert_eq!(repository(reference), written, "{reference}");
        }
        // Accepted separators: one dot, one or two underscores, any dashes.
        assert_eq!(
            repository("registry.example/acme/a.b_c__d---e"),
            "registry.example/acme/a.b_c__d---e"
        );
        assert_ne!(repository("ACME/api"), repository("acme/api"));
    }

    #[test]
    fn tags_and_repositories_are_split_for_display_only() {
        assert_eq!(
            written_repository("index.docker.io/acme/api:v1"),
            "index.docker.io/acme/api"
        );
        assert_eq!(written_repository("api:v1"), "api");
        assert_eq!(
            repository("registry.example:5000/app-a:v1"),
            "registry.example:5000/app-a"
        );
        assert_eq!(tag("registry.example:5000/app-a:v1"), Some("v1".into()));
        assert_eq!(tag("registry.example:5000/app-a"), None);
        assert_eq!(
            repository(&format!("registry.example/app-a:v1@{D}")),
            "registry.example/app-a"
        );
        assert_eq!(
            tag(&format!("registry.example/app-a:v1@{D}")),
            Some("v1".into())
        );
    }
}
