//! The one test that changes this process's environment, alone in its file so
//! nothing else reads it meanwhile.
#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use freshkube_agent::investigation::{Investigation, Outcome};
use freshkube_agent::profile::Host;
use serde_json::Value;
use support::{said, scripted_binary, stub, until_ended};

#[tokio::test]
async fn the_child_gets_the_locked_environment() {
    let real = tempfile::tempdir().unwrap();
    let kubeconfig = real.path().join("config");
    std::fs::write(&kubeconfig, "apiVersion: v1\nclusters: []\n").unwrap();
    // SAFETY: this file has one test, and it sets these before anything else
    // in the process reads the environment.
    unsafe {
        std::env::set_var("KUBECONFIG", &kubeconfig);
        std::env::set_var("AWS_SECRET_ACCESS_KEY", "do-not-leak-this-value");
        std::env::set_var("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE");
        std::env::set_var("ANTHROPIC_API_KEY", "do-not-leak-this-key");
    }

    let stub = stub().await;
    let link = scripted_binary(&[]).spawn(&Host::current()).unwrap();
    let (session, mut rx) = freshkube_agent::acp::AcpSession::start(link, stub.endpoint());
    session.prompt("describe your environment").unwrap();
    let mut inv = Investigation::default();
    until_ended(&mut rx, &mut inv).await;
    assert_eq!(inv.outcome(), Outcome::Answered);
    let text = said(&inv);
    for secret in [
        "do-not-leak-this-value",
        "AKIAEXAMPLE",
        "do-not-leak-this-key",
    ] {
        assert!(!text.contains(secret), "{secret} reached the child");
    }
    assert!(
        !text.contains(kubeconfig.to_str().unwrap()),
        "the real kubeconfig's path reached the child"
    );

    let report: Value = serde_json::from_str(&text).unwrap();
    let env = report["env"].as_object().unwrap();
    assert!(
        !env.keys()
            .any(|name| name.starts_with("AWS_") || name.ends_with("_KEY"))
    );
    // KUBECONFIG and TALOSCONFIG are there, and empty files.
    assert_ne!(
        env["KUBECONFIG"].as_str().unwrap(),
        kubeconfig.to_str().unwrap()
    );
    assert!(env.contains_key("TALOSCONFIG"));
    assert_eq!(report["kubeconfig_len"], 0);
    assert_eq!(report["talosconfig_len"], 0);
    // An empty config home, and an empty working directory.
    assert_eq!(report["config_home_entries"], 0);
    assert_eq!(report["cwd_entries"], 0);
    let cwd = std::path::PathBuf::from(report["cwd"].as_str().unwrap());
    assert!(cwd.is_dir());
    assert_ne!(cwd, std::env::current_dir().unwrap());

    // The directories go with the session.
    drop(session);
    let deadline = Instant::now() + Duration::from_secs(6);
    while cwd.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!cwd.exists(), "the working directory outlived the session");
    assert!(
        kubeconfig.exists(),
        "the person's own kubeconfig is untouched"
    );
}
