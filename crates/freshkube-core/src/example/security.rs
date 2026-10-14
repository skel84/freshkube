//! Security example data for a Talos node.
use chrono::{DateTime, Duration, Utc};

use super::ExampleNode;
use crate::security_lifecycle::{
    CertificateAudit, CertificateExpiryStatus, CertificateSource, ClusterIdentity,
    EncryptionProvider, RbacAudit, SecurityAuditSnapshot, SourceSnapshot, VolumeEncryptionAudit,
};

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
pub fn security(
    node: &ExampleNode,
    context: &str,
    target_address: &str,
    endpoints: Vec<String>,
    now: DateTime<Utc>,
) -> SecurityAuditSnapshot {
    let control_plane = node.control_plane;
    let unencrypted_worker = node.name.contains("wk-fra1-02");
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
    SecurityAuditSnapshot {
        identity: SourceSnapshot::Available(ClusterIdentity {
            context_name: context.to_owned(),
            endpoint_addresses: endpoints,
            target_addresses: vec![target_address.to_owned()],
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap()
    }

    fn snapshot(name: &str, control_plane: bool, now: DateTime<Utc>) -> SecurityAuditSnapshot {
        security(
            &ExampleNode::new(name, control_plane, None),
            "example",
            "192.0.2.30:50000",
            vec!["192.0.2.10:50000".into(), "192.0.2.11:50000".into()],
            now,
        )
    }

    fn volumes(snapshot: &SecurityAuditSnapshot) -> Vec<(&str, EncryptionProvider)> {
        snapshot
            .volume_encryption
            .value()
            .unwrap()
            .iter()
            .map(|volume| (volume.volume_name.as_str(), volume.provider.clone()))
            .collect()
    }

    #[test]
    fn control_plane_and_worker_keep_identity_and_use_their_encryption_providers() {
        let control = snapshot("example-control", true, now());
        let mut worker = snapshot("example-worker", false, now());
        assert_eq!(
            control.identity,
            SourceSnapshot::Available(ClusterIdentity {
                context_name: "example".into(),
                endpoint_addresses: vec!["192.0.2.10:50000".into(), "192.0.2.11:50000".into()],
                target_addresses: vec!["192.0.2.30:50000".into()],
            })
        );
        assert_eq!(
            volumes(&control),
            vec![
                ("STATE", EncryptionProvider::Tpm),
                ("EPHEMERAL", EncryptionProvider::Tpm)
            ]
        );
        assert_eq!(
            volumes(&worker),
            vec![
                ("STATE", EncryptionProvider::Kms),
                ("EPHEMERAL", EncryptionProvider::Kms)
            ]
        );
        worker.volume_encryption = control.volume_encryption.clone();
        assert_eq!(worker, control);
    }

    #[test]
    fn unencrypted_worker_keeps_state_encrypted_and_only_ephemeral_unencrypted() {
        let healthy = snapshot("example-worker", false, now());
        let mut unencrypted = snapshot("example-wk-fra1-02", false, now());
        assert_eq!(
            volumes(&unencrypted),
            vec![
                ("STATE", EncryptionProvider::Tpm),
                ("EPHEMERAL", EncryptionProvider::None)
            ]
        );
        unencrypted.volume_encryption = healthy.volume_encryption.clone();
        assert_eq!(unencrypted, healthy);
    }

    #[test]
    fn certificates_keep_their_lifetimes_and_near_expiry_relative_to_now() {
        for now in [now(), now() + Duration::days(60) + Duration::hours(7)] {
            let snapshot = snapshot("example-control", true, now);
            let talos = snapshot.talosconfig_certificates.value().unwrap();
            let kubernetes = snapshot.talos_kubeconfig_certificates.value().unwrap();
            assert_eq!(talos.len(), 2);
            assert_eq!(kubernetes.len(), 2);
            for (cert, name, source, days, lifetime, expiry, is_ca) in [
                (
                    &talos[0],
                    "Talos CA",
                    CertificateSource::TalosConfigCa,
                    3_410,
                    3_650,
                    CertificateExpiryStatus::Valid,
                    true,
                ),
                (
                    &talos[1],
                    "talosconfig",
                    CertificateSource::TalosConfigClient,
                    318,
                    365,
                    CertificateExpiryStatus::Valid,
                    false,
                ),
                (
                    &kubernetes[0],
                    "Kubernetes CA (cluster)",
                    CertificateSource::TalosKubeconfigCa,
                    3_322,
                    3_650,
                    CertificateExpiryStatus::Valid,
                    true,
                ),
                (
                    &kubernetes[1],
                    "kubeconfig (admin)",
                    CertificateSource::TalosKubeconfigClient,
                    21,
                    365,
                    CertificateExpiryStatus::ExpiringSoon,
                    false,
                ),
            ] {
                assert_eq!(cert.name, name);
                assert_eq!(cert.source, source);
                assert_eq!(cert.days_remaining, days);
                assert_eq!(cert.expiry, expiry);
                assert_eq!(cert.is_ca, is_ca);
                assert_eq!(
                    cert.not_after,
                    now + Duration::days(days) + Duration::hours(3)
                );
                assert_eq!(cert.not_before, cert.not_after - Duration::days(lifetime));
            }
            assert_eq!(talos[0].subject, "O=talos, CN=example");
            assert!(
                talos
                    .iter()
                    .all(|cert| cert.issuer == "O=talos, CN=example")
            );
            assert_eq!(kubernetes[0].subject, "O=kubernetes, CN=example");
            assert!(
                kubernetes
                    .iter()
                    .all(|cert| cert.issuer == "O=kubernetes, CN=example")
            );
        }
    }

    #[test]
    fn admin_role_matches_the_talos_client_certificate_on_every_node() {
        for (name, control_plane) in [
            ("example-control", true),
            ("example-worker", false),
            ("example-wk-fra1-02", false),
        ] {
            let snapshot = snapshot(name, control_plane, now());
            assert_eq!(
                snapshot.rbac,
                SourceSnapshot::Available(RbacAudit {
                    role: "os:admin".into(),
                    certificate_subject: "O=os:admin".into(),
                })
            );
            let talos_client = &snapshot.talosconfig_certificates.value().unwrap()[1];
            assert_eq!(talos_client.subject, "O=os:admin");
            let kubernetes_client = &snapshot.talos_kubeconfig_certificates.value().unwrap()[1];
            assert_eq!(kubernetes_client.subject, "O=system:masters, CN=admin");
        }
    }
}
