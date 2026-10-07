//! Example audit for `--fixture`.
use super::*;

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
pub(super) fn example(
    source: &ScreenSource,
    now: DateTime<Utc>,
) -> Result<SecurityAuditSnapshot, String> {
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
