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
pub fn is_full_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The repository of an image reference: no tag, no digest.
pub fn repository(reference: &str) -> String {
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
    fn tags_and_repositories_are_split_for_display_only() {
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
