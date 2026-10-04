//! Pure version interpretation used by Lifecycle.
//! The support table preserves the desktop rules; unknown versions stay unknown.

/// `v1.34.3`, `1.13.2-rc1` or `v1.30.0+k3s1` as numbers.
pub fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().trim_start_matches('v').split('.');
    let number = |part: Option<&str>| -> Option<u32> {
        let part = part?;
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let major = number(parts.next())?;
    let minor = number(parts.next())?;
    let patch = number(parts.next()).unwrap_or(0);
    Some((major, minor, patch))
}

/// The Kubernetes minors a Talos minor supports, from the support matrix at
/// <https://www.talos.dev/latest/introduction/support-matrix/>. Versions not
/// listed are unknown rather than unsupported.
pub fn kubernetes_support(talos: &str) -> Option<(u32, u32)> {
    match parse_version(talos)? {
        (1, 12, _) => Some((30, 35)),
        (1, 11, _) => Some((29, 34)),
        (1, 10, _) => Some((28, 33)),
        (1, 9, _) => Some((27, 32)),
        (1, 8, _) => Some((26, 31)),
        (1, 7, _) => Some((25, 30)),
        (1, 6, _) => Some((24, 29)),
        _ => None,
    }
}

/// Whether a kubelet trails the newest observed major/minor or only its patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KubeletSkew {
    Minor,
    Patch,
}

/// Classifies a reported version against the newest parsed kubelet version.
/// Missing or unparseable versions must not establish skew.
pub fn kubelet_skew(version: &str, newest: (u32, u32, u32)) -> Option<KubeletSkew> {
    classify_skew(parse_version(version)?, newest)
}

fn classify_skew(parsed: (u32, u32, u32), newest: (u32, u32, u32)) -> Option<KubeletSkew> {
    if parsed >= newest {
        None
    } else if (parsed.0, parsed.1) != (newest.0, newest.1) {
        Some(KubeletSkew::Minor)
    } else {
        Some(KubeletSkew::Patch)
    }
}

/// The newest parseable kubelet version among those that were reported.
pub fn newest_kubelet<'a>(versions: impl IntoIterator<Item = &'a str>) -> Option<(u32, u32, u32)> {
    versions.into_iter().filter_map(parse_version).max()
}

/// One node's parseable kubelet evidence. The caller owns its name and text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KubeletVersion<'a> {
    node: &'a str,
    text: &'a str,
    parsed: (u32, u32, u32),
}

impl<'a> KubeletVersion<'a> {
    pub fn node(self) -> &'a str {
        self.node
    }

    pub fn text(self) -> &'a str {
        self.text
    }
}

/// Pure facts for a caller-selected set of kubelet observations. No I/O, source
/// freshness, UI formatting or retained state lives here. A caller must supply
/// only current evidence when using these facts to raise cluster alerts.
#[derive(Clone, Debug)]
pub struct KubeletVersions<'a> {
    observed: Vec<KubeletVersion<'a>>,
    newest: Option<KubeletVersion<'a>>,
}

impl<'a> KubeletVersions<'a> {
    /// Keeps parseable observations in input order; missing versions can be
    /// omitted by the caller, and malformed versions are ignored.
    pub fn new(versions: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let observed: Vec<_> = versions
            .into_iter()
            .filter_map(|(node, text)| {
                Some(KubeletVersion {
                    node,
                    text,
                    parsed: parse_version(text)?,
                })
            })
            .collect();
        let newest = observed
            .iter()
            .copied()
            .max_by_key(|version| version.parsed);
        Self { observed, newest }
    }

    pub fn observed(&self) -> &[KubeletVersion<'a>] {
        &self.observed
    }

    /// Equal parsed versions keep the last reported spelling, as the screen's
    /// original max_by_key comparison did.
    pub fn newest(&self) -> Option<KubeletVersion<'a>> {
        self.newest
    }

    /// Nodes behind the newest observed version, preserving evidence order.
    pub fn behind(&self) -> impl Iterator<Item = KubeletVersion<'a>> + '_ {
        let newest = self.newest();
        self.observed
            .iter()
            .copied()
            .filter(move |version| newest.is_some_and(|newest| version.parsed < newest.parsed))
    }

    /// Minor skew takes precedence when any trailing node differs in major or
    /// minor; otherwise the difference is patch-only.
    pub fn skew(&self) -> Option<KubeletSkew> {
        let newest = self.newest()?;
        let mut skew = None;
        for version in &self.observed {
            match classify_skew(version.parsed, newest.parsed) {
                Some(KubeletSkew::Minor) => return Some(KubeletSkew::Minor),
                Some(KubeletSkew::Patch) => skew = Some(KubeletSkew::Patch),
                None => {}
            }
        }
        skew
    }

    /// Nodes outside the existing Talos support table. An unlisted Talos
    /// version establishes no unsupported nodes. A kubelet major other than
    /// one is outside every known range.
    pub fn outside_support(&self, talos: &str) -> impl Iterator<Item = KubeletVersion<'a>> + '_ {
        let support = kubernetes_support(talos);
        self.observed.iter().copied().filter(move |version| {
            support.is_some_and(|(low, high)| {
                version.parsed.0 != 1 || version.parsed.1 < low || version.parsed.1 > high
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_with_suffixes() {
        assert_eq!(parse_version("v1.34.3"), Some((1, 34, 3)));
        assert_eq!(parse_version("1.13.2-rc1"), Some((1, 13, 2)));
        assert_eq!(parse_version("v1.30.0+k3s1"), Some((1, 30, 0)));
        assert_eq!(parse_version("garbage"), None);
    }

    #[test]
    fn support_table_keeps_existing_ranges_and_unknown_versions() {
        for (talos, low, high) in [
            ("v1.12.4", 30, 35),
            ("v1.11.0", 29, 34),
            ("v1.10.9", 28, 33),
            ("v1.9.1", 27, 32),
            ("v1.8.0", 26, 31),
            ("v1.7.0", 25, 30),
            ("v1.6.0", 24, 29),
        ] {
            assert_eq!(kubernetes_support(talos), Some((low, high)));
        }
        let versions = KubeletVersions::new([("node", "v2.0.0")]);
        for talos in ["v1.13.2", "v1.5.0", "v2.0.0", "garbage", ""] {
            assert_eq!(kubernetes_support(talos), None);
            assert_eq!(versions.outside_support(talos).count(), 0);
        }
    }

    #[test]
    fn patch_skew_keeps_newest_text_and_evidence_order() {
        let versions = KubeletVersions::new([
            ("worker-b", "v1.34.2"),
            ("control-plane", "v1.34.3+k3s1"),
            ("worker-a", "v1.34.1"),
        ]);
        assert_eq!(versions.skew(), Some(KubeletSkew::Patch));
        assert_eq!(versions.newest().unwrap().text(), "v1.34.3+k3s1");
        assert_eq!(
            versions.behind().map(|v| v.node()).collect::<Vec<_>>(),
            ["worker-b", "worker-a"]
        );
        assert_eq!(
            versions
                .observed()
                .iter()
                .map(|v| v.text())
                .collect::<Vec<_>>(),
            ["v1.34.2", "v1.34.3+k3s1", "v1.34.1"]
        );
    }

    #[test]
    fn major_or_minor_skew_takes_precedence_over_patch_skew() {
        for older in ["v1.33.9", "v0.34.9"] {
            let versions = KubeletVersions::new([
                ("patch", "v1.34.2"),
                ("newest", "v1.34.3"),
                ("minor", older),
            ]);
            assert_eq!(versions.skew(), Some(KubeletSkew::Minor));
        }
        let versions = KubeletVersions::new([("old", "v1.99.0"), ("new", "v2.0.0")]);
        assert_eq!(versions.skew(), Some(KubeletSkew::Minor));
    }

    #[test]
    fn empty_or_unparseable_evidence_establishes_no_skew() {
        for input in [
            vec![],
            vec![
                ("missing", ""),
                ("malformed", "garbage"),
                ("incomplete", "v1"),
            ],
        ] {
            let versions = KubeletVersions::new(input);
            assert!(versions.observed().is_empty());
            assert!(versions.newest().is_none());
            assert!(versions.skew().is_none());
            assert_eq!(versions.behind().count(), 0);
            assert_eq!(versions.outside_support("v1.12.0").count(), 0);
        }
        let versions = KubeletVersions::new([("bad", "garbage"), ("known", "v1.34.3")]);
        assert_eq!(versions.observed().len(), 1);
        assert!(versions.skew().is_none());
        assert_eq!(newest_kubelet(["", "garbage"]), None);
        assert_eq!(
            newest_kubelet(["garbage", "v1.34.1", "v1.34.3"]),
            Some((1, 34, 3))
        );
        assert_eq!(kubelet_skew("garbage", (1, 34, 3)), None);
    }

    #[test]
    fn equal_parsed_versions_keep_last_spelling_without_skew() {
        let versions = KubeletVersions::new([("first", "v1.34.3"), ("last", "1.34.3+k3s1")]);
        assert_eq!(versions.newest().unwrap().node(), "last");
        assert_eq!(versions.newest().unwrap().text(), "1.34.3+k3s1");
        assert!(versions.skew().is_none());
        assert_eq!(versions.behind().count(), 0);
        // Preserve the permissive parser, including a missing/unparseable patch.
        assert_eq!(parse_version("v1.34"), Some((1, 34, 0)));
        assert_eq!(parse_version("v1.34.bad"), Some((1, 34, 0)));
    }

    #[test]
    fn support_boundaries_are_inclusive_and_require_major_one() {
        let versions = KubeletVersions::new([
            ("low", "v1.30.0"),
            ("high", "v1.35.99"),
            ("below", "v1.29.99"),
            ("above", "v1.36.0"),
            ("major-zero", "v0.32.0"),
            ("major-two", "v2.32.0"),
            ("bad", "not-a-version"),
        ]);
        assert_eq!(
            versions
                .outside_support("v1.12.3")
                .map(|v| v.node())
                .collect::<Vec<_>>(),
            ["below", "above", "major-zero", "major-two"]
        );
    }

    #[test]
    fn per_node_skew_matches_cluster_facts() {
        let input = [
            ("patch", "v1.34.2"),
            ("minor", "v1.33.9"),
            ("new", "v1.34.3"),
            ("bad", "garbage"),
        ];
        let versions = KubeletVersions::new(input);
        let newest = newest_kubelet(input.map(|(_, text)| text)).unwrap();
        for (node, text) in input {
            assert_eq!(
                kubelet_skew(text, newest).is_some(),
                versions.behind().any(|version| version.node() == node)
            );
        }
    }
}
