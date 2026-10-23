use super::*;

#[tokio::test]
async fn connection_id_is_shared_by_clones_and_node_copies_only() {
    let first = create_test_client(vec![]);
    let second = create_test_client(vec![]);
    assert_eq!(first.connection_id(), first.clone().connection_id());
    assert_eq!(
        first.connection_id(),
        first.with_node("10.0.0.1").connection_id()
    );
    assert_ne!(first.connection_id(), second.connection_id());
}

#[tokio::test]
async fn overview_decoders_reject_embedded_proxy_failures_and_empty_envelopes() {
    let client = create_test_client(vec!["selected-node".into()]);
    for (error, code) in [
        ("node unavailable", None),
        ("", Some(14)),
        ("denied", Some(0)),
    ] {
        let metadata = crate::proto::common::Metadata {
            hostname: "selected-node".into(),
            error: error.into(),
            status: code.map(|code| crate::proto::google::rpc::Status {
                code,
                message: "proxy failure".into(),
                details: vec![],
            }),
        };
        assert!(
            client
                .decode_version(crate::proto::machine::VersionResponse {
                    messages: vec![crate::proto::machine::Version {
                        metadata: Some(metadata.clone()),
                        ..Default::default()
                    }],
                })
                .is_err()
        );
        assert!(
            client
                .decode_services(crate::proto::machine::ServiceListResponse {
                    messages: vec![crate::proto::machine::ServiceList {
                        metadata: Some(metadata.clone()),
                        services: vec![],
                    }],
                })
                .is_err()
        );
        // Memory, load, and CPU replies use this same pre-decode validator.
        assert!(validate_read_responses([Some(&metadata)].into_iter()).is_err());
    }
    assert!(
        client
            .decode_version(crate::proto::machine::VersionResponse::default())
            .is_err()
    );
    assert!(
        client
            .decode_services(crate::proto::machine::ServiceListResponse::default())
            .is_err()
    );
    assert!(
        client
            .decode_version(crate::proto::machine::VersionResponse {
                messages: vec![crate::proto::machine::Version::default()],
            })
            .is_err()
    );
}

#[tokio::test]
async fn overview_decoders_accept_direct_node_and_successful_proxy_replies() {
    let client = create_test_client(vec!["selected-node".into()]);
    for metadata in [
        None,
        Some(crate::proto::common::Metadata {
            hostname: "proxy-node".into(),
            error: String::new(),
            status: Some(crate::proto::google::rpc::Status::default()),
        }),
    ] {
        let versions = client
            .decode_version(crate::proto::machine::VersionResponse {
                messages: vec![crate::proto::machine::Version {
                    metadata: metadata.clone(),
                    version: Some(crate::proto::machine::VersionInfo {
                        tag: "v1.11.0".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                }],
            })
            .unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, "v1.11.0");
        let catalog = client
            .decode_services(crate::proto::machine::ServiceListResponse {
                messages: vec![crate::proto::machine::ServiceList {
                    metadata,
                    services: vec![crate::proto::machine::ServiceInfo {
                        id: "ExactCaseService".into(),
                        state: "Running".into(),
                        ..Default::default()
                    }],
                }],
            })
            .unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].services[0].id, "ExactCaseService");
    }
}

struct MutationAckFixture {
    metadata: Option<crate::proto::common::Metadata>,
    payload: String,
}

fn mutation_ack(error: &str, code: Option<i32>, payload: &str) -> MutationAckFixture {
    MutationAckFixture {
        metadata: Some(crate::proto::common::Metadata {
            hostname: "fixture-node".to_string(),
            error: error.to_string(),
            status: code.map(|code| crate::proto::google::rpc::Status {
                code,
                message: "permission denied".to_string(),
                details: Vec::new(),
            }),
        }),
        payload: payload.to_string(),
    }
}

/// All fixtures exercise only response decoding: no RPC is made.
fn assert_mutation_response_protocol(
    action: &str,
    decode: impl Fn(Vec<MutationAckFixture>) -> Result<(), TalosError>,
) {
    for (error, code) in [
        ("upstream denied", None),
        ("", Some(7)),
        ("upstream denied", Some(7)),
        ("upstream denied", Some(0)),
    ] {
        // An error/status-only response and one with plausible but undefined
        // success payload must both be rejected.
        for payload in ["", "accepted"] {
            let TalosError::Grpc(status) =
                decode(vec![mutation_ack(error, code, payload)]).unwrap_err()
            else {
                panic!("expected upstream protocol error");
            };
            assert_eq!(
                status.code(),
                if code.is_some_and(|code| code != 0) {
                    tonic::Code::PermissionDenied
                } else {
                    tonic::Code::Unknown
                }
            );
            assert!(status.message().contains(action));
            assert!(status.message().contains("fixture-node"));
            assert!(status.message().contains(if error.is_empty() {
                "permission denied"
            } else {
                "upstream denied"
            }));
        }
    }

    // A valid first message must not hide a failed later target. Test both
    // orders and multiple failures so the full response is validated.
    for payload in ["", "accepted"] {
        for failed_first in [false, true] {
            let mut messages = vec![
                mutation_ack("", None, payload),
                mutation_ack("", Some(7), "undefined payload"),
            ];
            if failed_first {
                messages.reverse();
            }
            let error = decode(messages).unwrap_err().to_string();
            assert!(error.contains("aggregate success is not established"));
        }
    }
    let mut second_failure = mutation_ack("second target denied", None, "");
    second_failure.metadata.as_mut().unwrap().hostname = "second-node".to_string();
    let error = decode(vec![
        mutation_ack("first target denied", None, ""),
        second_failure,
    ])
    .unwrap_err()
    .to_string();
    assert!(error.contains("first target denied"));
    assert!(error.contains("second target denied"));
    assert!(error.contains("second-node"));

    // An empty envelope cannot establish acceptance, but a present default
    // message can: informational proto3 strings have no nonempty guarantee.
    let TalosError::Grpc(status) = decode(Vec::new()).unwrap_err() else {
        panic!("expected empty envelope error");
    };
    assert_eq!(status.code(), tonic::Code::DataLoss);
    assert!(status.message().contains(action));
    assert!(status.message().contains("configured-node"));

    for payload in ["", " ", "accepted"] {
        assert!(decode(vec![mutation_ack("", None, payload)]).is_ok());
        assert!(decode(vec![mutation_ack("", Some(0), payload)]).is_ok());
        assert!(
            decode(vec![MutationAckFixture {
                metadata: None,
                payload: payload.to_string(),
            }])
            .is_ok()
        );
    }
}

#[tokio::test]
async fn service_restart_response_protocol_rejects_failures_and_empty_envelopes() {
    let client = create_test_client(vec!["configured-node".to_string()]);
    assert_mutation_response_protocol("restart service kubelet", |fixtures| {
        client
            .decode_service_restart(
                crate::proto::machine::ServiceRestartResponse {
                    messages: fixtures
                        .into_iter()
                        .map(|fixture| crate::proto::machine::ServiceRestart {
                            metadata: fixture.metadata,
                            resp: fixture.payload,
                        })
                        .collect(),
                },
                "kubelet",
            )
            .map(|_| ())
    });
}

#[tokio::test]
async fn apply_configuration_response_protocol_rejects_failures_and_empty_envelopes() {
    let client = create_test_client(vec!["configured-node".to_string()]);
    for dry_run in [true, false] {
        let action = if dry_run {
            "validate configuration (dry run)"
        } else {
            "apply configuration"
        };
        assert_mutation_response_protocol(action, |fixtures| {
            client
                .decode_apply_configuration(
                    crate::proto::machine::ApplyConfigurationResponse {
                        messages: fixtures
                            .into_iter()
                            .map(|fixture| crate::proto::machine::ApplyConfiguration {
                                metadata: fixture.metadata,
                                mode_details: fixture.payload,
                                ..Default::default()
                            })
                            .collect(),
                    },
                    dry_run,
                )
                .map(|_| ())
        });
    }
}

#[tokio::test]
async fn reboot_response_protocol_rejects_failures_and_empty_envelopes() {
    let client = create_test_client(vec!["configured-node".to_string()]);
    assert_mutation_response_protocol("reboot", |fixtures| {
        client
            .decode_reboot(crate::proto::machine::RebootResponse {
                messages: fixtures
                    .into_iter()
                    .map(|fixture| crate::proto::machine::Reboot {
                        metadata: fixture.metadata,
                        actor_id: fixture.payload,
                    })
                    .collect(),
            })
            .map(|_| ())
    });
}

#[tokio::test]
async fn shutdown_response_protocol_rejects_failures_and_empty_envelopes() {
    let client = create_test_client(vec!["configured-node".to_string()]);
    assert_mutation_response_protocol("shutdown", |fixtures| {
        client
            .decode_shutdown(crate::proto::machine::ShutdownResponse {
                messages: fixtures
                    .into_iter()
                    .map(|fixture| crate::proto::machine::Shutdown {
                        metadata: fixture.metadata,
                        actor_id: fixture.payload,
                    })
                    .collect(),
            })
            .map(|_| ())
    });
}

#[tokio::test]
async fn mutation_response_decoders_preserve_success_payloads_and_target_fallbacks() {
    let client = create_test_client(vec![
        "configured-node".to_string(),
        "second-node".to_string(),
    ]);
    let services = client
        .decode_service_restart(
            crate::proto::machine::ServiceRestartResponse {
                messages: vec![
                    crate::proto::machine::ServiceRestart {
                        metadata: None,
                        resp: "restart accepted".to_string(),
                    },
                    crate::proto::machine::ServiceRestart {
                        metadata: None,
                        resp: "second restart accepted".to_string(),
                    },
                ],
            },
            "kubelet",
        )
        .unwrap();
    assert_eq!(services[0].node, "configured-node");
    assert_eq!(services[0].response, "restart accepted");
    assert_eq!(services[1].node, "second-node");
    let configs = client
        .decode_apply_configuration(
            crate::proto::machine::ApplyConfigurationResponse {
                messages: vec![crate::proto::machine::ApplyConfiguration {
                    metadata: mutation_ack("", None, "").metadata,
                    mode_details: "configuration applied".to_string(),
                    warnings: vec!["warning".to_string()],
                    ..Default::default()
                }],
            },
            false,
        )
        .unwrap();
    assert_eq!(configs[0].node, "fixture-node");
    assert_eq!(configs[0].mode_result, "configuration applied");
    assert_eq!(configs[0].warnings, vec!["warning"]);
    let reboot = client
        .decode_reboot(crate::proto::machine::RebootResponse {
            messages: vec![crate::proto::machine::Reboot {
                metadata: None,
                actor_id: "reboot-actor".to_string(),
            }],
        })
        .unwrap();
    assert_eq!(reboot.node, "configured-node");
    let shutdown = client
        .decode_shutdown(crate::proto::machine::ShutdownResponse {
            messages: vec![crate::proto::machine::Shutdown {
                metadata: mutation_ack("", None, "").metadata,
                actor_id: "shutdown-actor".to_string(),
            }],
        })
        .unwrap();
    assert_eq!(shutdown.node, "fixture-node");
}
