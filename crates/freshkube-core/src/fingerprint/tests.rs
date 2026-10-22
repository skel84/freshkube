use super::*;
use crate::fake_api;

const CERT_A: &str = "-----BEGIN CERTIFICATE-----\nMIIBgzCCASmgAwIBAgIUIQ12bd746Tnjj/2F6NiRJXuS0jEwCgYIKoZIzj0EAwIw\nFzEVMBMGA1UEAwwMZXhhbXBsZS1jYS1hMB4XDTI2MTAyMTIyMDk1NFoXDTM2MTAx\nODIyMDk1NFowFzEVMBMGA1UEAwwMZXhhbXBsZS1jYS1hMFkwEwYHKoZIzj0CAQYI\nKoZIzj0DAQcDQgAEEGj85l3MytzlxqInTbbmayp1X6zB8t1Dp7eA3d/4WRaSLPSN\nbdkHT3ieXg0xz40N19igezoDdi6ZYKo3RmcfYqNTMFEwHQYDVR0OBBYEFFm4BmBa\ngN9lRFRgpvHEWx1DSJP+MB8GA1UdIwQYMBaAFFm4BmBagN9lRFRgpvHEWx1DSJP+\nMA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDSAAwRQIhAKjoG9/G+aDHfFAM\nUkFCTyVgVZfC6A284nZ3YJ7n7277AiBdaIQ8vOk90I7vgJ/AmYALzRBQZ2IDK7rM\nRmBqmy+0TQ==\n-----END CERTIFICATE-----\n";
const CERT_B: &str = "-----BEGIN CERTIFICATE-----\nMIIBgjCCASmgAwIBAgIUOaOowgU0Q087QEwnzTxx9nFxpG4wCgYIKoZIzj0EAwIw\nFzEVMBMGA1UEAwwMZXhhbXBsZS1jYS1iMB4XDTI2MTAyMTIyMDk1NFoXDTM2MTAx\nODIyMDk1NFowFzEVMBMGA1UEAwwMZXhhbXBsZS1jYS1iMFkwEwYHKoZIzj0CAQYI\nKoZIzj0DAQcDQgAERk7+Ufo+TL+DJ/2bQOiFSmso1uARrHZXli238zyYff0PngXw\nE5soUclIBiVLTNMiNysHhAS4Cb+ERBs6Yi14WqNTMFEwHQYDVR0OBBYEFIxOk74c\ncexkKJyv1x22KeqhKWsoMB8GA1UdIwQYMBaAFIxOk74ccexkKJyv1x22KeqhKWso\nMA8GA1UdEwEB/wQFMAMBAf8wCgYIKoZIzj0EAwIDRwAwRAIgG/fYZaLqjyooCCHX\nX80eqcJfTZFMa1w0YWtFAp4NPCoCIDDyA7wn4I8Nupig2B2jHIgnBA+bIUihyKyi\ntIiwShRs\n-----END CERTIFICATE-----\n";

fn fp(strong: Option<&str>, ca: Option<&str>) -> Fingerprint {
    Fingerprint {
        strong: strong.map(str::to_owned),
        ca: ca.map(str::to_owned),
    }
}

#[test]
fn a_token_is_26_characters_of_lower_case_base32_and_stable() {
    let uid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    let token = strong_from_uid(uid);
    assert_eq!(token.len(), 26);
    assert!(
        token
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    );
    assert_eq!(strong_from_uid(uid), token);
    assert_ne!(strong_from_uid("another"), token);
    // Pinned: a change of salt, domain or encoding would split every pair
    // of people who matched.
    assert_eq!(token, "qltztfoyzwvmpnall5ukjnrtdq");
}

#[test]
fn the_uid_is_never_in_the_token() {
    let uid = "0f8fad5b-d9cb-469f-a165-70867728950e";
    assert!(!strong_from_uid(uid).contains("0f8f"));
}

#[test]
fn the_same_input_in_another_domain_gives_another_token() {
    let text = "shared-text";
    let by_uid = strong_from_uid(text);
    let by_override = strong_from_override(text).unwrap();
    assert_ne!(by_uid, by_override);
    // The override string itself is not recoverable from, or in, the token.
    assert!(!by_override.contains(text));
}

#[test]
fn an_override_is_one_to_128_characters_without_whitespace() {
    assert!(strong_from_override("prod-eu").is_some());
    assert!(strong_from_override(&"a".repeat(128)).is_some());
    for bad in [
        String::new(),
        "a".repeat(129),
        "a b".into(),
        "a\tb".into(),
        "a\nb".into(),
        " ".into(),
    ] {
        assert_eq!(strong_from_override(&bad), None, "{bad:?}");
    }
}

#[test]
fn the_ca_value_ignores_order_and_repeats() {
    let ab = format!("{CERT_A}{CERT_B}");
    let ba = format!("{CERT_B}{CERT_A}");
    let aab = format!("{CERT_A}{CERT_A}{CERT_B}");
    let value = ca_from_pem(ab.as_bytes()).unwrap();
    assert_eq!(ca_from_pem(ba.as_bytes()).as_deref(), Some(value.as_str()));
    assert_eq!(ca_from_pem(aab.as_bytes()).as_deref(), Some(value.as_str()));
    assert_ne!(
        ca_from_pem(CERT_A.as_bytes()),
        ca_from_pem(CERT_B.as_bytes())
    );
    assert_ne!(
        ca_from_pem(CERT_A.as_bytes()).as_deref(),
        Some(value.as_str())
    );
}

#[test]
fn a_bundle_without_certificates_or_too_large_has_no_ca_value() {
    assert_eq!(ca_from_pem(b""), None);
    assert_eq!(ca_from_pem(b"not a pem"), None);
    let key = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
    assert_eq!(ca_from_pem(key.as_bytes()), None);
    assert_eq!(ca_from_pem(&vec![b'x'; CA_MAX_BYTES + 1]), None);
}

fn kubeconfig(cluster: &str) -> Kubeconfig {
    serde_yaml::from_str(&format!(
        "apiVersion: v1\nkind: Config\nclusters:\n- name: c1\n  cluster:\n    server: https://127.0.0.1:1\n{cluster}contexts:\n- name: ctx\n  context:\n    cluster: c1\n- name: orphan\n  context:\n    cluster: missing\nusers: []\n"
    ))
    .unwrap()
}

#[test]
fn a_contexts_ca_is_read_from_inline_data_or_a_file() {
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(CERT_A);
    let inline = kubeconfig(&format!("    certificate-authority-data: {data}\n"));
    assert_eq!(
        ca_of_context(&inline, "ctx"),
        ca_from_pem(CERT_A.as_bytes())
    );

    let directory =
        std::env::temp_dir().join(format!("freshkube-fingerprint-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("ca.pem");
    std::fs::write(&file, CERT_B).unwrap();
    let by_file = kubeconfig(&format!("    certificate-authority: {}\n", file.display()));
    assert_eq!(
        ca_of_context(&by_file, "ctx"),
        ca_from_pem(CERT_B.as_bytes())
    );
    std::fs::remove_dir_all(&directory).unwrap();

    // No CA, an unknown context, a context with no cluster, or bad data.
    assert_eq!(ca_of_context(&kubeconfig(""), "ctx"), None);
    assert_eq!(ca_of_context(&inline, "nope"), None);
    assert_eq!(ca_of_context(&inline, "orphan"), None);
    let bad = kubeconfig("    certificate-authority-data: '!!!'\n");
    assert_eq!(ca_of_context(&bad, "ctx"), None);
}

#[test]
fn matching_follows_the_four_rules() {
    let strong = |s: &str| Some(s.to_owned());
    // 1. Both strong and equal: exact, with or without a CA.
    assert_eq!(
        matches(&fp(Some("a"), None), &fp(Some("a"), None)),
        Some(Strength::Exact)
    );
    assert_eq!(
        matches(&fp(Some("a"), Some("x")), &fp(Some("a"), Some("y"))),
        Some(Strength::Exact)
    );
    // 2. Both strong and different: no match, even with the same CA.
    assert_eq!(
        matches(&fp(Some("a"), Some("x")), &fp(Some("b"), Some("x"))),
        None
    );
    // 3. A side lacks a strong value and the CA is equal: CA only.
    assert_eq!(
        matches(&fp(strong("a").as_deref(), Some("x")), &fp(None, Some("x"))),
        Some(Strength::CaOnly)
    );
    assert_eq!(
        matches(&fp(None, Some("x")), &fp(Some("b"), Some("x"))),
        Some(Strength::CaOnly)
    );
    assert_eq!(
        matches(&fp(None, Some("x")), &fp(None, Some("x"))),
        Some(Strength::CaOnly)
    );
    // 4. Otherwise none.
    assert_eq!(matches(&fp(None, Some("x")), &fp(None, Some("y"))), None);
    assert_eq!(matches(&fp(Some("a"), None), &fp(None, None)), None);
    assert_eq!(matches(&fp(None, None), &fp(None, None)), None);
    assert_eq!(matches(&fp(Some("a"), None), &fp(None, Some("x"))), None);
}

#[test]
fn a_shared_ca_behind_a_proxy_never_makes_two_clusters_one() {
    let link = fp(Some("cluster-one"), Some("proxy-ca"));
    let other = fp(Some("cluster-two"), Some("proxy-ca"));
    assert_eq!(matches(&link, &other), None);
    // Unchecked, the same CA only suggests it: it asks.
    let unchecked = fp(None, Some("proxy-ca"));
    assert_eq!(matches(&link, &unchecked), Some(Strength::CaOnly));
}

#[test]
fn exact_ranks_before_ca_only() {
    assert!(Strength::Exact < Strength::CaOnly);
}

fn namespace_answer(status: u16) -> (u16, String) {
    if status == 200 {
        fake_api::json(
            200,
            r#"{"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadata","metadata":{"name":"kube-system","uid":"11111111-2222-3333-4444-555555555555"}}"#,
        )
    } else {
        fake_api::json(
            status,
            format!(
                r#"{{"kind":"Status","status":"Failure","reason":"Forbidden","message":"no","code":{status}}}"#
            ),
        )
    }
}

#[tokio::test]
async fn the_strong_value_is_one_metadata_get_of_kube_system() {
    let (client, recorder) = fake_api::client(|_| Some(namespace_answer(200)));
    assert_eq!(
        read_strong(&client).await,
        StrongRead::Uid("11111111-2222-3333-4444-555555555555".into())
    );
    recorder.assert_lines(&["GET /api/v1/namespaces/kube-system"]);
    assert_eq!(
        recorder.requests()[0].accept,
        "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1"
    );
}

#[tokio::test]
async fn a_refusal_falls_back_to_the_ca_and_nothing_else_is_asked() {
    for status in [401, 403] {
        let (client, recorder) = fake_api::client(move |_| Some(namespace_answer(status)));
        assert_eq!(read_strong(&client).await, StrongRead::Refused, "{status}");
        recorder.assert_lines(&["GET /api/v1/namespaces/kube-system"]);
    }
}

#[tokio::test]
async fn any_other_failure_is_shown_and_is_not_a_fallback() {
    for status in [404, 500] {
        let (client, recorder) = fake_api::client(move |_| Some(namespace_answer(status)));
        assert!(
            matches!(read_strong(&client).await, StrongRead::Failed(_)),
            "{status}"
        );
        recorder.assert_lines(&["GET /api/v1/namespaces/kube-system"]);
    }
    // A namespace the API gave no UID for.
    let (client, _) = fake_api::client(|_| {
        Some(fake_api::json(
            200,
            r#"{"apiVersion":"meta.k8s.io/v1","kind":"PartialObjectMetadata","metadata":{"name":"kube-system"}}"#,
        ))
    });
    assert!(matches!(read_strong(&client).await, StrongRead::Failed(_)));
}
