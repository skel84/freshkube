//! The audit's rows: a certificate, the RBAC role, a volume, or a part that
//! couldn't be read, from the snapshot core collected.
use super::*;

/// The audit's groups, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    TalosCertificates,
    KubernetesCertificates,
    Rbac,
    Volumes,
}

impl Section {
    pub(super) fn title(self) -> &'static str {
        match self {
            Section::TalosCertificates => "Talos certificates",
            Section::KubernetesCertificates => "Kubernetes certificates",
            Section::Rbac => "RBAC",
            Section::Volumes => "Volume encryption",
        }
    }

    /// The group row it sits under: both kinds of certificate share one.
    pub(super) fn group(self) -> Group {
        match self {
            Section::TalosCertificates | Section::KubernetesCertificates => Group::Certificates,
            Section::Rbac => Group::Rbac,
            Section::Volumes => Group::Volumes,
        }
    }

    pub(super) fn slug(self) -> &'static str {
        match self {
            Section::TalosCertificates => "talos-certs",
            Section::KubernetesCertificates => "kubernetes-certs",
            Section::Rbac => "rbac",
            Section::Volumes => "volumes",
        }
    }
}

/// The table's groups, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    Certificates,
    Rbac,
    Volumes,
}

impl Group {
    pub(super) fn label(self) -> &'static str {
        match self {
            Group::Certificates => "Certificates",
            Group::Rbac => "RBAC role",
            Group::Volumes => "Volume encryption",
        }
    }

    pub(super) fn slug(self) -> &'static str {
        match self {
            Group::Certificates => "certificates",
            Group::Rbac => "rbac",
            Group::Volumes => "volumes",
        }
    }

    fn noun(self, count: usize) -> &'static str {
        match (self, count) {
            (Group::Certificates, 1) => "certificate",
            (Group::Certificates, _) => "certificates",
            (Group::Rbac, 1) => "role",
            (Group::Rbac, _) => "roles",
            (Group::Volumes, 1) => "volume",
            (Group::Volumes, _) => "volumes",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    Good,
    Warn,
    Crit,
    Info,
    Unknown,
}

impl Verdict {
    fn from_health(health: HealthIndicator) -> Self {
        match health {
            HealthIndicator::Healthy => Verdict::Good,
            HealthIndicator::Warning => Verdict::Warn,
            HealthIndicator::Error => Verdict::Crit,
            HealthIndicator::Info => Verdict::Info,
            _ => Verdict::Unknown,
        }
    }

    /// The row's status glyph.
    pub(super) fn glyph(self) -> Tone {
        match self {
            Verdict::Good => Tone::Good,
            Verdict::Warn => Tone::Warn,
            Verdict::Crit => Tone::Crit,
            Verdict::Info => Tone::Info,
            Verdict::Unknown => Tone::Unknown,
        }
    }

    /// How much it asks for attention: a group takes its worst row's.
    fn weight(self) -> u8 {
        match self {
            Verdict::Good => 0,
            Verdict::Info => 1,
            Verdict::Unknown => 2,
            Verdict::Warn => 3,
            Verdict::Crit => 4,
        }
    }

    /// The tag's tone, and an icon for Info, which carries no status.
    pub(super) fn tone(self) -> (Tone, Option<IconName>) {
        match self {
            Verdict::Good => (Tone::Good, None),
            Verdict::Warn => (Tone::Warn, None),
            Verdict::Crit => (Tone::Crit, None),
            Verdict::Info => (Tone::Accent, Some(IconName::Info)),
            Verdict::Unknown => (Tone::Unknown, None),
        }
    }
}

/// One selectable row: a certificate, the RBAC role, a volume, or a section
/// that couldn't be read (shown as unknown).
#[derive(Clone, Debug)]
pub(crate) struct Item {
    pub(super) key: SharedString,
    pub(super) section: Section,
    pub(super) name: String,
    pub(super) verdict: Verdict,
    /// The pill: the verdict in words.
    pub(super) status: String,
    /// The right-hand column of the row.
    pub(super) summary: String,
    pub(super) fields: Vec<(&'static str, String)>,
    pub(super) note: Option<String>,
}

fn source_label(source: CertificateSource) -> &'static str {
    match source {
        CertificateSource::TalosConfigCa => "Talosconfig CA",
        CertificateSource::TalosConfigClient => "Talosconfig client certificate",
        CertificateSource::TalosKubeconfigCa => "Talos-served kubeconfig CA",
        CertificateSource::TalosKubeconfigClient => "Talos-served kubeconfig client certificate",
    }
}

fn date(time: DateTime<Utc>) -> String {
    time.format("%Y-%m-%d %H:%M UTC").to_string()
}

fn certificate_item(section: Section, ix: usize, cert: &CertificateAudit) -> Item {
    let (verdict, status) = match cert.expiry {
        CertificateExpiryStatus::Valid => (Verdict::Good, "Valid"),
        CertificateExpiryStatus::ExpiringSoon => (Verdict::Warn, "Expiring soon"),
        CertificateExpiryStatus::ExpiringVerySoon => (Verdict::Crit, "Expires very soon"),
        CertificateExpiryStatus::Expired => (Verdict::Crit, "Expired"),
    };
    let summary = if cert.days_remaining <= 0 {
        match cert.days_remaining.unsigned_abs() {
            0 => "Expired today".to_owned(),
            1 => "Expired 1 day ago".to_owned(),
            days => format!("Expired {days} days ago"),
        }
    } else if cert.days_remaining == 1 {
        "Valid for 1 day".to_owned()
    } else {
        format!("Valid for {} days", cert.days_remaining)
    };
    let note = match cert.expiry {
        CertificateExpiryStatus::Valid => None,
        CertificateExpiryStatus::ExpiringSoon => {
            Some("Expires within 30 days. Plan the certificate renewal.".to_owned())
        }
        CertificateExpiryStatus::ExpiringVerySoon => {
            Some("Expires within 7 days. Renew it now.".to_owned())
        }
        CertificateExpiryStatus::Expired => {
            Some("This certificate has expired and will be rejected by peers.".to_owned())
        }
    };
    Item {
        key: format!(
            "{}:{ix}:{}:{}",
            section.slug(),
            cert.name,
            cert.not_after.timestamp()
        )
        .into(),
        section,
        name: cert.name.clone(),
        verdict,
        status: status.to_owned(),
        summary,
        fields: vec![
            ("Source", source_label(cert.source).to_owned()),
            ("Subject", cert.subject.clone()),
            ("Issuer", cert.issuer.clone()),
            ("Not before", date(cert.not_before)),
            ("Not after", date(cert.not_after)),
            ("Days remaining", cert.days_remaining.to_string()),
            (
                "Certificate authority",
                if cert.is_ca { "Yes" } else { "No" }.to_owned(),
            ),
        ],
        note,
    }
}

/// What to show for a section whose source is unknown: one unknown row, so
/// the page never implies the certificates are fine or bad.
fn unknown_item(section: Section, what: &str, reason: &str) -> Item {
    Item {
        key: format!("{}:unknown", section.slug()).into(),
        section,
        name: what.to_owned(),
        verdict: Verdict::Unknown,
        status: "Unknown".into(),
        summary: "Not reported".into(),
        fields: vec![("Reason", reason.to_owned())],
        note: Some("Nothing is known about this source, so nothing is shown as failed.".into()),
    }
}

fn certificate_items(
    section: Section,
    source: &SourceSnapshot<Vec<CertificateAudit>>,
) -> Vec<Item> {
    match source {
        SourceSnapshot::Unavailable { reason } => {
            vec![unknown_item(section, section.title(), reason)]
        }
        SourceSnapshot::Available(certs) | SourceSnapshot::Partial { value: certs, .. } => {
            if certs.is_empty() {
                let reason = match source {
                    SourceSnapshot::Partial { warnings, .. } if !warnings.is_empty() => {
                        warnings.join("; ")
                    }
                    _ => "The source returned no certificates".to_owned(),
                };
                return vec![unknown_item(section, "No certificates found", &reason)];
            }
            certs
                .iter()
                .enumerate()
                .map(|(ix, cert)| certificate_item(section, ix, cert))
                .collect()
        }
    }
}

fn rbac_items(source: &SourceSnapshot<RbacAudit>) -> Vec<Item> {
    match source {
        SourceSnapshot::Unavailable { reason } => {
            vec![unknown_item(Section::Rbac, "Current role", reason)]
        }
        SourceSnapshot::Available(rbac) | SourceSnapshot::Partial { value: rbac, .. } => {
            let mut note = String::from(
                "Derived from the talosconfig client certificate subject, not a live authorization probe.",
            );
            if rbac.role == "os:admin" {
                note.push_str(
                    " os:admin grants full control of every node; use os:reader for read-only monitoring where possible.",
                );
            }
            vec![Item {
                key: "rbac:role".into(),
                section: Section::Rbac,
                name: "Current role".into(),
                verdict: Verdict::Info,
                status: "Info".into(),
                summary: rbac.role.clone(),
                fields: vec![
                    ("Role", rbac.role.clone()),
                    ("Certificate subject", rbac.certificate_subject.clone()),
                    ("RBAC enabled", "Yes".into()),
                ],
                note: Some(note),
            }]
        }
    }
}

fn provider_name(provider: &EncryptionProvider) -> String {
    match provider {
        EncryptionProvider::None => "None".into(),
        EncryptionProvider::Static => "Static key".into(),
        EncryptionProvider::NodeId => "Node ID".into(),
        EncryptionProvider::Tpm => "TPM".into(),
        EncryptionProvider::Kms => "KMS".into(),
        EncryptionProvider::Other(name) => name.clone(),
    }
}

fn provider_strength(provider: &EncryptionProvider) -> &'static str {
    match provider {
        EncryptionProvider::None => "Unencrypted",
        EncryptionProvider::Static | EncryptionProvider::NodeId => "Weak",
        EncryptionProvider::Tpm | EncryptionProvider::Kms => "Strong",
        EncryptionProvider::Other(_) => "Unknown",
    }
}

fn volume_item(volume: &VolumeEncryptionAudit) -> Item {
    let verdict = Verdict::from_health(volume.provider.health());
    let advice = match &volume.provider {
        EncryptionProvider::None => "No encryption is reported for this volume.",
        EncryptionProvider::Static => {
            "Static key stored in the machine config. Consider TPM for better security."
        }
        EncryptionProvider::NodeId => {
            "Key derived from the node UUID. Provides minimal protection."
        }
        EncryptionProvider::Tpm => "Key sealed to the TPM. Hardware-backed encryption.",
        EncryptionProvider::Kms => "Key managed by an external KMS. Strong encryption.",
        EncryptionProvider::Other(_) => {
            "Talos reported a provider this view doesn't recognise, so its strength is unknown."
        }
    };
    Item {
        key: format!("volumes:{}", volume.volume_name).into(),
        section: Section::Volumes,
        name: format!("{} volume", volume.volume_name),
        verdict,
        status: provider_strength(&volume.provider).to_owned(),
        summary: provider_name(&volume.provider),
        fields: vec![
            ("Volume", volume.volume_name.clone()),
            ("Provider", provider_name(&volume.provider)),
            ("Strength", provider_strength(&volume.provider).to_owned()),
        ],
        note: Some(advice.to_owned()),
    }
}

fn volume_items(source: &SourceSnapshot<Vec<VolumeEncryptionAudit>>) -> Vec<Item> {
    match source {
        SourceSnapshot::Unavailable { reason } => {
            vec![unknown_item(
                Section::Volumes,
                "STATE and EPHEMERAL",
                reason,
            )]
        }
        SourceSnapshot::Available(volumes) | SourceSnapshot::Partial { value: volumes, .. } => {
            if volumes.is_empty() {
                let reason = match source {
                    SourceSnapshot::Partial { warnings, .. } if !warnings.is_empty() => {
                        warnings.join("; ")
                    }
                    _ => "Talos returned no STATE or EPHEMERAL volume status".to_owned(),
                };
                return vec![unknown_item(
                    Section::Volumes,
                    "STATE and EPHEMERAL",
                    &reason,
                )];
            }
            volumes.iter().map(volume_item).collect()
        }
    }
}

/// What the page shows of one audit: its rows, the sources that didn't
/// fully answer and the counts over the list. Derived when the audit
/// arrives, so drawing only reads it.
#[derive(Default)]
pub(super) struct Display {
    pub(super) items: Vec<Item>,
    /// The table's lines: each group's row, then its items.
    pub(super) lines: Vec<Entry>,
    /// The group rows, by the index an [`Entry::Group`] carries.
    pub(super) groups: Vec<GroupLine>,
    pub(super) missing: Vec<String>,
    pub(super) stats: Stats,
}

impl Display {
    pub(super) fn new(snapshot: &SecurityAuditSnapshot) -> Self {
        let items = items(snapshot);
        let (lines, groups) = lines(&items);
        Self {
            items,
            lines,
            groups,
            missing: missing(snapshot),
            stats: Stats::new(snapshot),
        }
    }
}

/// One line of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Group(usize),
    Row(usize),
}

/// A group's row: its worst verdict and what it holds, in words.
pub(super) struct GroupLine {
    pub(super) group: Group,
    pub(super) verdict: Verdict,
    pub(super) detail: Vec<String>,
}

/// The items under their group rows, in the order they come.
fn lines(items: &[Item]) -> (Vec<Entry>, Vec<GroupLine>) {
    let mut lines = Vec::new();
    let mut groups: Vec<GroupLine> = Vec::new();
    let mut counts = Vec::new();
    for (ix, item) in items.iter().enumerate() {
        let group = item.section.group();
        if groups.last().is_none_or(|line| line.group != group) {
            lines.push(Entry::Group(groups.len()));
            groups.push(GroupLine {
                group,
                verdict: item.verdict,
                detail: Vec::new(),
            });
            counts.push((0, 0));
        }
        let line = groups.last_mut().unwrap();
        if item.verdict.weight() > line.verdict.weight() {
            line.verdict = item.verdict;
        }
        let (known, unknown) = counts.last_mut().unwrap();
        if item.verdict == Verdict::Unknown {
            *unknown += 1;
        } else {
            *known += 1;
        }
        lines.push(Entry::Row(ix));
    }
    for (line, (known, unknown)) in groups.iter_mut().zip(counts) {
        if known > 0 {
            line.detail
                .push(format!("{known} {}", line.group.noun(known)));
        }
        if unknown > 0 {
            line.detail.push("not all reported".to_owned());
        }
    }
    (lines, groups)
}

/// The counts over the list, in words: a number, or "unknown" when the
/// source didn't answer.
#[derive(Default)]
pub(super) struct Stats {
    pub(super) valid: String,
    pub(super) expiring: String,
    pub(super) expired: String,
    pub(super) volumes: String,
}

impl Stats {
    fn new(snapshot: &SecurityAuditSnapshot) -> Self {
        let certs: Vec<&CertificateAudit> = snapshot
            .talosconfig_certificates
            .value()
            .into_iter()
            .chain(snapshot.talos_kubeconfig_certificates.value())
            .flatten()
            .collect();
        let known = snapshot.talosconfig_certificates.value().is_some()
            || snapshot.talos_kubeconfig_certificates.value().is_some();
        let count = |pred: fn(CertificateExpiryStatus) -> bool| {
            if known {
                certs
                    .iter()
                    .filter(|cert| pred(cert.expiry))
                    .count()
                    .to_string()
            } else {
                "unknown".to_owned()
            }
        };
        let volumes = match snapshot.volume_encryption.value() {
            Some(volumes) if !volumes.is_empty() => format!(
                "{} of {}",
                volumes
                    .iter()
                    .filter(|volume| volume.provider != EncryptionProvider::None)
                    .count(),
                volumes.len()
            ),
            _ => "unknown".to_owned(),
        };
        Self {
            valid: count(|status| status == CertificateExpiryStatus::Valid),
            expiring: count(|status| {
                matches!(
                    status,
                    CertificateExpiryStatus::ExpiringSoon
                        | CertificateExpiryStatus::ExpiringVerySoon
                )
            }),
            expired: count(|status| status == CertificateExpiryStatus::Expired),
            volumes,
        }
    }
}

/// Every row of the audit, in display order.
pub(super) fn items(snapshot: &SecurityAuditSnapshot) -> Vec<Item> {
    let mut items = certificate_items(
        Section::TalosCertificates,
        &snapshot.talosconfig_certificates,
    );
    items.extend(certificate_items(
        Section::KubernetesCertificates,
        &snapshot.talos_kubeconfig_certificates,
    ));
    items.extend(rbac_items(&snapshot.rbac));
    items.extend(volume_items(&snapshot.volume_encryption));
    items
}

/// Sources that didn't fully answer, for the partial notice.
fn missing(snapshot: &SecurityAuditSnapshot) -> Vec<String> {
    fn note<T>(label: &str, source: &SourceSnapshot<T>, out: &mut Vec<String>) {
        if let Some(problem) = source.problem() {
            let partial = if matches!(source, SourceSnapshot::Partial { .. }) {
                " (partial)"
            } else {
                ""
            };
            out.push(format!("{label}{partial}: {problem}"));
        }
    }
    let mut out = Vec::new();
    note("Talos identity", &snapshot.identity, &mut out);
    note(
        "Talos certificates",
        &snapshot.talosconfig_certificates,
        &mut out,
    );
    note(
        "Kubernetes certificates",
        &snapshot.talos_kubeconfig_certificates,
        &mut out,
    );
    note("RBAC", &snapshot.rbac, &mut out);
    note("Volume encryption", &snapshot.volume_encryption, &mut out);
    out
}

/// Where the audit read from, after the context the meta line names first:
/// the Talos endpoints and the node whose volumes it read.
pub(super) fn identity_parts(identity: &SourceSnapshot<ClusterIdentity>) -> Vec<SharedString> {
    let Some(identity) = identity.value() else {
        return Vec::new();
    };
    let join = |values: &[String]| {
        if values.is_empty() {
            "none".to_owned()
        } else {
            values.join(", ")
        }
    };
    vec![
        format!("endpoints {}", join(&identity.endpoint_addresses)).into(),
        format!("volume target {}", join(&identity.target_addresses)).into(),
    ]
}
