use super::*;

#[test]
fn association_requires_exact_access_cluster_and_kubernetes_kind() {
    let provider = Provider::new("https://coroot.example.com", Credentials::None).unwrap();
    let source = source(&provider);
    let id = AppId::new("cluster-a:prod:Deployment:api");
    assert!(ObjectSubject::for_app(&source, &id).is_none());
    let source = source.with_association(Some(Association::new(
        "access:opaque-a".into(),
        "cluster-a".into(),
    )));
    let subject = ObjectSubject::for_app(&source, &id).unwrap();
    assert_eq!(subject.access(), "access:opaque-a");
    assert_eq!(subject.namespace(), "prod");
    assert_eq!(subject.name(), "api");
    assert_eq!(subject.kind().kind, "Deployment");
    for id in [
        "cluster-a:prod:Deployment:../api",
        "cluster-a:prod/other:Pod:api",
        "cluster-a:prod:Pod:",
    ] {
        assert!(ObjectSubject::for_app(&source, &AppId::new(id)).is_none());
    }
    assert!(
        ObjectSubject::for_app(&source, &AppId::new("cluster-b:prod:Deployment:api")).is_none()
    );
    assert!(
        ObjectSubject::for_app(
            &source,
            &AppId::new("external:external:ExternalService:api:443")
        )
        .is_none()
    );
}

#[test]
fn empty_credentials_do_not_silently_select_anonymous_access() {
    for credentials in [
        Credentials::ApiKey(String::new()),
        Credentials::Session(" ".into()),
    ] {
        assert!(matches!(
            Provider::new("https://coroot.example.com", credentials),
            Err(ReadError::Authentication)
        ));
    }
    assert!(Provider::new("https://coroot.example.com", Credentials::None).is_ok());
}

#[test]
fn url_policy_and_bounds_fail_without_leaking_inputs() {
    for url in [
        "coroot.example.com",
        "https://user:secret@example.com",
        "https://example.com/?secret=x",
        "file:///etc/passwd",
    ] {
        assert!(matches!(
            Provider::new(url, Credentials::None),
            Err(ReadError::InvalidUrl)
        ));
    }
    let app = Application {
        id: AppId::new("c:prod:Deployment:api"),
        cluster: String::new(),
        category: String::new(),
        app_type: String::new(),
        status: Status::Unknown,
        signals: Default::default(),
    };
    assert_eq!(
        limits::applications(&vec![app.clone(); 2001]),
        Err(ReadError::Limit)
    );
    assert_eq!(
        limits::applications(&[app.clone(), app]),
        Err(ReadError::InvalidResponse)
    );
}
