//! Security: the TUI's security view. Certificate expiry for the Talos and
//! Kubernetes PKI, the Talos RBAC role, and volume encryption on the target
//! node. Read-only.
//!
//! Every part of the audit is its own source. A part that didn't answer
//! shows as unknown (and is named in the partial notice), never as failed or
//! insecure.
use chrono::{DateTime, Duration, Utc};
use freshkube_core::HealthIndicator;
use freshkube_core::security_lifecycle::{
    CertificateAudit, CertificateExpiryStatus, CertificateSource, ClusterIdentity,
    EncryptionProvider, RbacAudit, SecurityAuditCollector, SecurityAuditSnapshot, SourceSnapshot,
    VolumeEncryptionAudit,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use tokio::runtime::Handle;

use super::{
    Loader, Scope, ScreenEvent, ScreenPanel, ScreenSource, content_width, failure_banner, field,
    gated_page, header, mono, panel, partial_notice, stat,
};
use crate::palette::palette;
use crate::ui::{self, MONO_FONT, Tone, dp};

const CONTEXT: &str = "TalosSecurity";
const ROW_HEIGHT: f32 = 34.;
/// Below this content width the details pane moves under the list.
const SIDE_DETAILS: f32 = 920.;
const LIST_MIN_HEIGHT: f32 = 240.;
const DETAILS_HEIGHT: f32 = 260.;

actions!(
    talos_security,
    [NextItem, PreviousItem, FirstItem, LastItem]
);

/// The audit's groups, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    TalosCertificates,
    KubernetesCertificates,
    Rbac,
    Volumes,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::TalosCertificates => "Talos certificates",
            Section::KubernetesCertificates => "Kubernetes certificates",
            Section::Rbac => "RBAC",
            Section::Volumes => "Volume encryption",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Section::TalosCertificates => IconName::KeyRound,
            Section::KubernetesCertificates => IconName::ShieldCheck,
            Section::Rbac => IconName::Shield,
            Section::Volumes => IconName::Lock,
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Section::TalosCertificates => "talos-certs",
            Section::KubernetesCertificates => "kubernetes-certs",
            Section::Rbac => "rbac",
            Section::Volumes => "volumes",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
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

    fn tone(self) -> (Tone, IconName) {
        match self {
            Verdict::Good => (Tone::Good, IconName::CircleCheck),
            Verdict::Warn => (Tone::Warn, IconName::CircleAlert),
            Verdict::Crit => (Tone::Crit, IconName::ShieldAlert),
            Verdict::Info => (Tone::Accent, IconName::Info),
            Verdict::Unknown => (Tone::Unknown, IconName::CircleDashed),
        }
    }
}

/// One selectable row: a certificate, the RBAC role, a volume, or a section
/// that couldn't be read (shown as unknown).
#[derive(Clone, Debug)]
struct Item {
    key: String,
    section: Section,
    name: String,
    verdict: Verdict,
    /// The pill: the verdict in words.
    status: String,
    /// The right-hand column of the row.
    summary: String,
    fields: Vec<(&'static str, String)>,
    note: Option<String>,
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
        ),
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
        key: format!("{}:unknown", section.slug()),
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
        key: format!("volumes:{}", volume.volume_name),
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

/// Every row of the audit, in display order.
fn items(snapshot: &SecurityAuditSnapshot) -> Vec<Item> {
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
        match source {
            SourceSnapshot::Available(_) => {}
            SourceSnapshot::Partial { warnings, .. } => {
                out.push(format!("{label} (partial): {}", warnings.join("; ")))
            }
            SourceSnapshot::Unavailable { reason } => out.push(format!("{label}: {reason}")),
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

pub(crate) struct SecurityScreen {
    runtime: Handle,
    source: Option<ScreenSource>,
    loader: Loader<SecurityAuditSnapshot>,
    /// Selection survives refreshes by key; the first row shows until one is chosen.
    selected: Option<String>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<ScreenEvent> for SecurityScreen {}

impl ScreenPanel for SecurityScreen {
    fn new(runtime: Handle, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.bind_keys([
            KeyBinding::new("down", NextItem, Some(CONTEXT)),
            KeyBinding::new("up", PreviousItem, Some(CONTEXT)),
            KeyBinding::new("home", FirstItem, Some(CONTEXT)),
            KeyBinding::new("end", LastItem, Some(CONTEXT)),
        ]);
        Self {
            runtime,
            source: None,
            loader: Loader::default(),
            selected: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    fn set_source(&mut self, source: Option<ScreenSource>, _: &mut Window, cx: &mut Context<Self>) {
        let changed = self.source.as_ref().map(|source| &source.target)
            != source.as_ref().map(|source| &source.target);
        if changed {
            self.loader.reset();
            self.selected = None;
        }
        self.source = source;
        cx.notify();
    }

    fn activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loader.data().is_none() && !self.loader.is_loading() {
            self.refresh(window, cx);
        }
    }

    fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn refresh(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.loader.is_loading() {
            return;
        }
        let Some(live) = source.live.clone() else {
            self.loader
                .resolve(source.target.clone(), example(&source, Utc::now()));
            cx.notify();
            return;
        };
        let target = source.target.clone();
        self.loader.load(
            source.target.clone(),
            &self.runtime,
            "the security audit",
            async move {
                // A worker can't serve Kubernetes credentials, so pin that
                // part to a control plane; volumes still use the target node.
                let pinned = live.cluster.control_plane_ip();
                let client = pinned.as_deref().map_or_else(
                    || live.client.clone(),
                    |address| live.client.with_node(address),
                );
                let mut snapshot = SecurityAuditCollector::new(live.config_path.clone())
                    .collect_for_context(&client, &target.context, &target.address)
                    .await;
                if pinned.is_none() {
                    snapshot.talos_kubeconfig_certificates = SourceSnapshot::Unavailable {
                        reason:
                            "No control plane is known to serve the Talos kubeconfig; no ambient kubeconfig was audited"
                                .into(),
                    };
                }
                Ok(snapshot)
            },
            |screen: &mut Self| &mut screen.loader,
            cx,
        );
        cx.notify();
    }
}

impl SecurityScreen {
    fn items(&self) -> Vec<Item> {
        self.loader.data().map(items).unwrap_or_default()
    }

    /// Index of the selected row; the first row until the user picks one.
    fn selected_index(&self, items: &[Item]) -> Option<usize> {
        match &self.selected {
            Some(key) => items.iter().position(|item| &item.key == key),
            None => (!items.is_empty()).then_some(0),
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.items();
        if items.is_empty() {
            return;
        }
        let current = self.selected_index(&items).unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(items.len() - 1);
        self.selected = Some(items[next].key.clone());
        cx.notify();
    }

    fn identity_line(
        &self,
        identity: &SourceSnapshot<ClusterIdentity>,
        cx: &App,
    ) -> Option<Stateful<Div>> {
        let identity = identity.value()?;
        let p = palette(cx);
        let join = |values: &[String]| {
            if values.is_empty() {
                "none".to_owned()
            } else {
                values.join(", ")
            }
        };
        Some(
            h_flex()
                .id("security-identity")
                .gap_x_5()
                .gap_y_1()
                .flex_wrap()
                .text_size(dp(12.5))
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(div().text_color(p.muted).child("Context"))
                        .child(mono(identity.context_name.clone())),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(div().text_color(p.muted).child("Endpoints"))
                        .child(mono(join(&identity.endpoint_addresses))),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .child(div().text_color(p.muted).child("Volume target"))
                        .child(mono(join(&identity.target_addresses))),
                ),
        )
    }

    fn summary(&self, snapshot: &SecurityAuditSnapshot, cx: &App) -> Stateful<Div> {
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
        let valid = count(|status| status == CertificateExpiryStatus::Valid);
        let expiring = count(|status| {
            matches!(
                status,
                CertificateExpiryStatus::ExpiringSoon | CertificateExpiryStatus::ExpiringVerySoon
            )
        });
        let expired = count(|status| status == CertificateExpiryStatus::Expired);
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
        h_flex()
            .id("security-summary")
            .gap_2p5()
            .flex_wrap()
            .child(stat("Valid certificates", valid, cx))
            .child(stat("Expiring soon", expiring, cx))
            .child(stat("Expired", expired, cx))
            .child(stat("Volumes encrypted", volumes, cx))
    }

    fn render_row(
        &self,
        ix: usize,
        item: &Item,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let p = palette(cx);
        let (tone, _) = item.verdict.tone();
        let key = item.key.clone();
        h_flex()
            .id(("security-item", ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(format!(
                "{} · {} · {}",
                item.name, item.status, item.summary
            ))
            .w_full()
            .h(dp(ROW_HEIGHT))
            .flex_none()
            .gap_3()
            .px_3()
            .text_size(dp(12.5))
            .cursor_pointer()
            .when(selected, |this| this.bg(p.accent_soft).text_color(p.accent))
            .when(!selected, |this| this.hover(|style| style.bg(p.hover)))
            .child(div().w(dp(112.)).flex_none().child(ui::tag(
                tone,
                None,
                item.status.clone(),
                cx,
            )))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(item.name.clone()),
            )
            .child(
                div()
                    .max_w(dp(220.))
                    .truncate()
                    .font_family(MONO_FONT)
                    .text_size(dp(12.))
                    .when(!selected, |this| this.text_color(p.muted))
                    .child(item.summary.clone()),
            )
            .on_click(cx.listener(move |view, _, window, cx| {
                view.selected = Some(key.clone());
                window.focus(&view.focus, cx);
                cx.notify();
            }))
    }

    fn section_head(section: Section, cx: &App) -> impl IntoElement + use<> {
        let p = palette(cx);
        h_flex()
            .id(SharedString::from(format!(
                "security-section-{}",
                section.slug()
            )))
            .test_support()
            .aria_label(section.title())
            .flex_none()
            .gap_2()
            .px_3()
            .pt_3()
            .pb_1()
            .border_b_1()
            .border_color(p.line)
            .child(Icon::new(section.icon()).size(dp(13.)).text_color(p.muted))
            .child(ui::caption(section.title(), cx))
    }

    fn details(&self, item: Option<&Item>, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let Some(item) = item else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child("Select an item to see its details.");
        };
        let (tone, icon) = item.verdict.tone();
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .id("security-detail-title")
                            .test_support()
                            .aria_label(item.name.clone())
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(item.name.clone()),
                    )
                    .child(ui::tag(tone, Some(icon), item.status.clone(), cx)),
            )
            .children(
                item.fields.iter().map(|(label, value)| {
                    field(label, mono(value.clone()).whitespace_normal(), cx)
                }),
            )
            .when_some(item.note.clone(), |this, note| {
                this.child(
                    div()
                        .id("security-detail-note")
                        .test_support()
                        .aria_label(note.clone())
                        .text_size(dp(12.5))
                        .text_color(p.muted)
                        .child(note),
                )
            })
    }
}

impl Render for SecurityScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(page) = gated_page(
            "security-page",
            "Security",
            Scope::Cluster,
            self.source.as_ref(),
            &self.loader,
            "the security audit",
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(snapshot)) = (self.source.clone(), self.loader.data()) else {
            return div().into_any_element();
        };
        let p = palette(cx);
        let all = items(snapshot);
        let selected_ix = self.selected_index(&all);
        let missing = missing(snapshot);
        let identity = self.identity_line(&snapshot.identity, cx);
        let summary = self.summary(snapshot, cx);

        // Rows are grouped under section captions; remember where the
        // selected row lands among the children so it can scroll into view.
        let mut children: Vec<AnyElement> = Vec::new();
        let mut selected_child = None;
        let mut section = None;
        for (ix, item) in all.iter().enumerate() {
            if section != Some(item.section) {
                section = Some(item.section);
                children.push(Self::section_head(item.section, cx).into_any_element());
            }
            if selected_ix == Some(ix) {
                selected_child = Some(children.len());
            }
            children.push(
                self.render_row(ix, item, selected_ix == Some(ix), cx)
                    .into_any_element(),
            );
        }
        if let Some(child) = selected_child {
            self.scroll.scroll_to_item(child);
        }

        let list = panel(cx)
            .flex_1()
            .min_h(dp(LIST_MIN_HEIGHT))
            .overflow_hidden()
            .child(
                v_flex()
                    .id("security-list")
                    .test_support()
                    .role(Role::ListBox)
                    .aria_label(
                        "Certificates, RBAC role and volume encryption; arrows select an item",
                    )
                    .key_context(CONTEXT)
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|view, _: &NextItem, _, cx| view.step(1, cx)))
                    .on_action(cx.listener(|view, _: &PreviousItem, _, cx| view.step(-1, cx)))
                    .on_action(cx.listener(|view, _: &FirstItem, _, cx| view.step(isize::MIN, cx)))
                    .on_action(cx.listener(|view, _: &LastItem, _, cx| view.step(isize::MAX, cx)))
                    .flex_1()
                    .min_h_0()
                    .pb_2()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .when(all.is_empty(), |this| {
                        this.child(
                            div()
                                .px_3()
                                .py_3p5()
                                .text_size(dp(12.5))
                                .text_color(p.muted)
                                .child("The audit reported nothing."),
                        )
                    })
                    .children(children),
            );
        let details = self.details(selected_ix.map(|ix| &all[ix]), cx);
        let wide = content_width(window) >= SIDE_DETAILS;
        let split = if wide {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT))
                .items_stretch()
                .gap(dp(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id("security-details")
                        .w(dp(380.))
                        .flex_none()
                        .overflow_y_scroll()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(dp(LIST_MIN_HEIGHT + 14. + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(dp(14.)).child(list).child(
                        div()
                            .id("security-details")
                            .h(dp(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .child(details),
                    ),
                )
        };
        v_flex()
            .id("security-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(header(
                "Security",
                &source,
                Scope::Cluster,
                &self.loader,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .children(identity)
            .child(summary)
            .child(split)
            .into_any_element()
    }
}

#[allow(clippy::too_many_arguments)]
fn certificate(
    name: &str,
    source: CertificateSource,
    subject: &str,
    issuer: &str,
    days_valid: i64,
    lifetime_days: i64,
    is_ca: bool,
    now: DateTime<Utc>,
) -> CertificateAudit {
    let not_after = now + Duration::days(days_valid) + Duration::hours(3);
    CertificateAudit {
        name: name.into(),
        source,
        subject: subject.into(),
        issuer: issuer.into(),
        not_before: not_after - Duration::days(lifetime_days),
        not_after,
        days_remaining: days_valid,
        expiry: CertificateExpiryStatus::from_days_remaining(days_valid),
        is_ca,
    }
}

/// Example audit for `--fixture`: healthy PKI with one Kubernetes client
/// certificate close to expiry, an admin role, and one worker whose
/// EPHEMERAL volume isn't encrypted. Dates are relative to `now`.
fn example(source: &ScreenSource, now: DateTime<Utc>) -> Result<SecurityAuditSnapshot, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let context = &source.target.context;
    let control_plane = node.role == crate::presentation::Role::ControlPlane;
    let unencrypted_worker = node.name.contains("wk-fra1-02");
    let endpoints: Vec<String> = source
        .nodes
        .iter()
        .filter(|node| node.role == crate::presentation::Role::ControlPlane)
        .map(|node| node.address.clone())
        .collect();
    let volume = |name: &str, provider| VolumeEncryptionAudit {
        volume_name: name.into(),
        provider,
    };
    let volumes = if unencrypted_worker {
        vec![
            volume("STATE", EncryptionProvider::Tpm),
            volume("EPHEMERAL", EncryptionProvider::None),
        ]
    } else if control_plane {
        vec![
            volume("STATE", EncryptionProvider::Tpm),
            volume("EPHEMERAL", EncryptionProvider::Tpm),
        ]
    } else {
        vec![
            volume("STATE", EncryptionProvider::Kms),
            volume("EPHEMERAL", EncryptionProvider::Kms),
        ]
    };
    let subject = "O=os:admin";
    let ca_subject = format!("O=talos, CN={context}");
    let k8s_ca = format!("O=kubernetes, CN={context}");
    Ok(SecurityAuditSnapshot {
        identity: SourceSnapshot::Available(ClusterIdentity {
            context_name: context.clone(),
            endpoint_addresses: endpoints,
            target_addresses: vec![source.target.address.clone()],
        }),
        talosconfig_certificates: SourceSnapshot::Available(vec![
            certificate(
                "Talos CA",
                CertificateSource::TalosConfigCa,
                &ca_subject,
                &ca_subject,
                3_410,
                3_650,
                true,
                now,
            ),
            certificate(
                "talosconfig",
                CertificateSource::TalosConfigClient,
                subject,
                &ca_subject,
                318,
                365,
                false,
                now,
            ),
        ]),
        talos_kubeconfig_certificates: SourceSnapshot::Available(vec![
            certificate(
                "Kubernetes CA (cluster)",
                CertificateSource::TalosKubeconfigCa,
                &k8s_ca,
                &k8s_ca,
                3_322,
                3_650,
                true,
                now,
            ),
            certificate(
                "kubeconfig (admin)",
                CertificateSource::TalosKubeconfigClient,
                "O=system:masters, CN=admin",
                &k8s_ca,
                21,
                365,
                false,
                now,
            ),
        ]),
        rbac: SourceSnapshot::Available(RbacAudit {
            role: "os:admin".into(),
            certificate_subject: subject.into(),
        }),
        volume_encryption: SourceSnapshot::Available(volumes),
    })
}

#[cfg(test)]
mod ui_tests {
    use std::sync::Arc;

    use chrono::Utc;
    use freshkube_core::security_lifecycle::SourceSnapshot;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Entity, TestAppContext, WindowHandle, px, size};
    use tokio::runtime::{Builder, Runtime};

    // Not `super::*`: gpui_kit's glob would shadow the built-in `#[test]`.
    use super::{ScreenPanel, ScreenSource, Section, SecurityScreen, Verdict, example, items};
    use crate::backend::Target;
    use crate::{fixture, presentation};

    fn source(node: &str) -> ScreenSource {
        let nodes = presentation::node_summaries(&fixture::cluster("prod-fra", 1));
        let summary = nodes.iter().find(|summary| summary.name == node).unwrap();
        ScreenSource {
            target: Target {
                epoch: 1,
                context: "prod-fra".into(),
                node: summary.name.clone(),
                address: summary.address.clone(),
            },
            nodes: Arc::new(nodes),
            live: None,
        }
    }

    fn mount(
        cx: &mut TestAppContext,
        node: &str,
    ) -> (Runtime, Entity<SecurityScreen>, WindowHandle<Root>) {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
        });
        let source = source(node);
        let mut screen = None;
        let handle = cx.open_window(size(px(1100.), px(760.)), |window, cx| {
            let view = cx.new(|cx| {
                let mut view = SecurityScreen::new(runtime.handle().clone(), window, cx);
                view.set_source(Some(source), window, cx);
                view.activate(window, cx);
                view
            });
            screen = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        (runtime, screen.unwrap(), handle)
    }

    #[gpui_kit::test]
    fn keyboard_selection_updates_the_details(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find(("security-item", 0usize)).selected(),
                Some(true)
            );
            window.click(("security-item", 0usize), cx);
            window.press("down", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find(("security-item", 1usize)).selected(),
                Some(true)
            );
            assert_eq!(
                window.find("security-detail-title").label(),
                Some("talosconfig")
            );
            window.press("end", cx);
            window.render_frame(cx);
            let count = screen.read(cx).items().len();
            assert_eq!(
                window.find(("security-item", count - 1)).selected(),
                Some(true)
            );
            assert_eq!(
                window.find("security-detail-title").label(),
                Some("EPHEMERAL volume")
            );
            window.press("home", cx);
            window.render_frame(cx);
            assert_eq!(
                window.find(("security-item", 0usize)).selected(),
                Some(true)
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn expiring_certificate_is_a_warning(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let all = screen.read(cx).items();
            let expiring: Vec<_> = all
                .iter()
                .filter(|item| item.section == Section::KubernetesCertificates)
                .filter(|item| item.verdict == Verdict::Warn)
                .collect();
            assert_eq!(expiring.len(), 1);
            assert_eq!(expiring[0].status, "Expiring soon");
            assert!(
                all.iter()
                    .filter(|item| item.section != Section::Volumes)
                    .all(|item| item.verdict != Verdict::Crit),
                "only the expiring certificate is flagged, and only as a warning"
            );
            let ix = all
                .iter()
                .position(|item| item.verdict == Verdict::Warn)
                .unwrap();
            let label = window
                .find(("security-item", ix))
                .label()
                .map(str::to_owned);
            assert!(label.unwrap().contains("Expiring soon"));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn unencrypted_worker_volume_is_a_warning_not_a_failure(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-02");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let all = screen.read(cx).items();
            let ephemeral = all
                .iter()
                .find(|item| item.section == Section::Volumes && item.name.starts_with("EPHEMERAL"))
                .unwrap();
            assert_eq!(ephemeral.verdict, Verdict::Warn);
            assert!(all.iter().all(|item| item.verdict != Verdict::Crit));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn unavailable_section_is_named_and_unknown(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("partial-notice").is_none());
            screen.update(cx, |screen, _| {
                let source = screen.source.clone().unwrap();
                let mut snapshot = example(&source, Utc::now()).unwrap();
                snapshot.talos_kubeconfig_certificates = SourceSnapshot::Unavailable {
                    reason: "Talos did not provide a kubeconfig: timed out".into(),
                };
                screen.loader.resolve(source.target.clone(), Ok(snapshot));
            });
            window.render_frame(cx);
            window.find("partial-notice");
            let snapshot = screen.read(cx).loader.data().unwrap().clone();
            let rows: Vec<_> = items(&snapshot)
                .into_iter()
                .filter(|item| item.section == Section::KubernetesCertificates)
                .collect();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].verdict, Verdict::Unknown);
            assert_eq!(rows[0].status, "Unknown");
            assert!(window.try_find(("security-item", 2usize)).is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn silent_target_offers_retry_without_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-wk-fra1-03");
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(screen.read(cx).loader.data().is_none());
            window.find("screen-retry");
            assert!(window.try_find("security-list").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn changing_target_drops_old_data(cx: &mut TestAppContext) {
        let (_runtime, screen, handle) = mount(cx, "talos-cp-fra1-01");
        cx.update_window(handle.into(), |_, window, cx| {
            screen.update(cx, |screen, cx| {
                screen.selected = Some("rbac:role".into());
                assert!(screen.loader.data().is_some());
                screen.set_source(Some(source("talos-wk-fra1-02")), window, cx);
                assert!(screen.loader.data().is_none());
                assert!(screen.selected.is_none());
            });
        })
        .unwrap();
    }
}
