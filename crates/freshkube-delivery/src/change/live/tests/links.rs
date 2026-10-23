use super::*;

fn config_map(namespace: &str, name: &str, key: &str, value: &str) -> Value {
    json!({
        "metadata": {"name": name, "namespace": namespace},
        "data": {key: value, "other": "kept out"}
    })
}

/// The run as Pipelines as Code annotates a pull request's, and its task
/// as Tekton records the pod it ran in.
fn annotated(world: &mut World, provider: &str, sha_url: &str) {
    annotated_with(
        world,
        provider,
        sha_url,
        "https://console.example.test/runs/storefront-push-x",
    );
}

/// [`annotated`], with the run's console page at `log_url`.
fn annotated_with(world: &mut World, provider: &str, sha_url: &str, log_url: &str) {
    let mut run = pipeline_run(SHA, true, Some(NEW));
    let annotations = run["metadata"]["annotations"]
        .as_object_mut()
        .expect("annotations");
    for (key, value) in [
        ("repo-url", "https://git.example/acme/storefront"),
        ("sha-url", sha_url),
        ("git-provider", provider),
        ("pull-request", "7"),
        ("log-url", log_url),
    ] {
        annotations.insert(format!("pipelinesascode.tekton.dev/{key}"), json!(value));
    }
    let mut task = task_run();
    task["metadata"]["name"] = json!("storefront-push-x-build");
    task["metadata"]["labels"]["tekton.dev/pipelineTask"] = json!("build");
    task["status"]["podName"] = json!("storefront-push-x-build-pod");
    task["status"]["steps"] = json!([{"container": "step-build"}, {"container": "step-push"}]);
    task["status"]["conditions"] = json!([{"type": "Succeeded", "status": "True"}]);
    world.tekton = std::mem::take(&mut world.tekton)
        .with("pipelineruns", vec![run])
        .with("taskruns", vec![task]);
}

#[tokio::test]
async fn links_out_are_made_from_what_the_cluster_records() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with(
            "configmaps",
            vec![
                // User info, a query and a fragment never leave the cluster.
                config_map(
                    "argocd",
                    "argocd-cm",
                    "url",
                    "https://admin:pw@argocd.example.test/?token=abc#x",
                ),
                config_map(
                    "kargo",
                    "kargo-api",
                    "ADMIN_ACCOUNT_TOKEN_ISSUER",
                    "https://kargo.example.test",
                ),
            ],
        );
        annotated(
            world,
            "github",
            "https://git.example/acme/storefront/commit/abc",
        );
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);

    let freight = "https://kargo.example.test/project/storefront/freight/f-new";
    assert_eq!(
        change.page.as_ref().map(|a| a.to_string()),
        Ok(freight.into())
    );
    let kargo = link_out(&links, "Open in Kargo");
    assert_eq!((kargo.2.as_deref(), &kargo.3), (Some(freight), &None));

    let argocd = link_out(&links, "Open in Argo CD");
    assert_eq!(
        argocd.2.as_deref(),
        Some("https://argocd.example.test/applications/argocd/storefront-dev")
    );
    assert_eq!(
        link_out(&links, "Open the pull request").2.as_deref(),
        Some("https://git.example/acme/storefront/pull/7")
    );
    assert_eq!(
        link_out(&links, "Open the commit").2.as_deref(),
        Some("https://git.example/acme/storefront/commit/abc")
    );
    assert_eq!(
        link_out(&links, "Open the run").2.as_deref(),
        Some("https://console.example.test/runs/storefront-push-x")
    );
    let logs = link_out(&links, "Logs of build");
    assert_eq!(
        (logs.2.as_deref(), &logs.3),
        (
            Some("cluster-key/acme-builds/storefront-push-x-build-pod Some(\"step-build\")"),
            &None
        )
    );

    // Only the two ConfigMaps were read beside the change: never a Secret.
    let requests = reader.requests.borrow();
    assert!(
        !requests.iter().any(|r| r.contains("secrets")),
        "{requests:#?}"
    );
    let maps: Vec<&String> = requests
        .iter()
        .filter(|r| r.contains("configmaps"))
        .collect();
    assert_eq!(
        maps,
        [
            "GET /configmaps Some(\"argocd\") argocd-cm",
            "GET /configmaps Some(\"kargo\") kargo-api"
        ]
    );
}

#[tokio::test]
async fn links_without_a_recorded_address_are_greyed_out_with_why() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).refusing("configmaps");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);

    let why = change.page.as_ref().expect_err("no Kargo page");
    assert!(
        why.starts_with("Kargo's address isn't known: ConfigMap kargo-api in kargo is refused"),
        "{why}"
    );
    let argocd = link_out(&links, "Open in Argo CD");
    assert_eq!(argocd.2, None);
    assert!(
        argocd.3.as_deref().is_some_and(|why| why.starts_with(
            "Argo CD's address isn't known: ConfigMap argocd-cm in argocd is refused"
        )),
        "{argocd:?}"
    );
    // The healthy run carries no Pipelines as Code addresses.
    assert_eq!(
        link_out(&links, "Open the commit").3.as_deref(),
        Some("The run has no Pipelines as Code sha-url")
    );
}

#[tokio::test]
async fn only_http_and_https_addresses_open() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).with(
            "configmaps",
            vec![
                config_map("argocd", "argocd-cm", "url", "javascript:alert(1)"),
                config_map(
                    "kargo",
                    "kargo-api",
                    "ADMIN_ACCOUNT_TOKEN_ISSUER",
                    "kargo.example.test",
                ),
            ],
        );
        annotated(world, "bitbucket-datacenter", "file:///etc/passwd");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);

    for (label, why) in [
        (
            "Open in Argo CD",
            "Argo CD's address isn't known: url of ConfigMap argocd-cm in argocd: it isn't an http or https address",
        ),
        (
            "Open in Kargo",
            "Kargo's address isn't known: ADMIN_ACCOUNT_TOKEN_ISSUER of ConfigMap kargo-api in kargo: it isn't an address",
        ),
        (
            "Open the commit",
            "The run's sha-url: it isn't an http or https address",
        ),
        (
            "Open the pull request",
            "Pipelines as Code's provider bitbucket-datacenter has no known pull request page",
        ),
    ] {
        let link = link_out(&links, label);
        assert_eq!(
            (link.2.as_deref(), link.3.as_deref()),
            (None, Some(why)),
            "{label}"
        );
    }
    assert!(change.page.is_err());
}

/// One run's links, read from a cluster whose run Pipelines as Code
/// annotated with `provider`, `sha_url` and `log_url`.
async fn run_links(
    provider: &str,
    sha_url: &str,
    log_url: &str,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let reader = one_cluster(|world| annotated_with(world, provider, sha_url, log_url));
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    links_out(&change)
}

#[tokio::test]
async fn each_provider_has_its_own_pull_request_page() {
    for (provider, page) in [
        ("github", "https://git.example/acme/storefront/pull/7"),
        ("gitea", "https://git.example/acme/storefront/pulls/7"),
        ("forgejo", "https://git.example/acme/storefront/pulls/7"),
        (
            "gitlab",
            "https://git.example/acme/storefront/-/merge_requests/7",
        ),
        (
            "bitbucket-cloud",
            "https://git.example/acme/storefront/pull-requests/7",
        ),
    ] {
        let links = run_links(
            provider,
            "https://git.example/c",
            "https://console.example.test/r",
        )
        .await;
        let pull = link_out(&links, "Open the pull request");
        assert_eq!(
            (pull.2.as_deref(), &pull.3),
            (Some(page), &None),
            "{provider}"
        );
    }
}

/// Pipelines as Code names a fallback console when none is configured;
/// it serves no page, so the run's link is greyed out.
async fn assert_no_console(log_url: &str) {
    let links = run_links("github", "https://git.example/c", log_url).await;
    let run = link_out(&links, "Open the run");
    assert_eq!(
        (run.2.as_deref(), run.3.as_deref()),
        (None, Some("Pipelines as Code has no dashboard configured")),
        "{log_url}"
    );
}

#[tokio::test]
async fn pipelines_as_codes_fallback_console_is_no_page() {
    assert_no_console(
        "https://dashboard.url.is.not.configured/#/namespaces/acme-builds/pipelineruns/x",
    )
    .await;
}

/// Some releases write the fallback without `url`.
#[tokio::test]
async fn pipelines_as_codes_shorter_fallback_console_is_no_page() {
    assert_no_console("https://dashboard.is.not.configured/acme-builds/x").await;
}

/// The Tekton Dashboard's run page is a route in the fragment: it is kept,
/// without its query, where every other link drops its fragment.
#[tokio::test]
async fn the_dashboards_run_route_is_kept() {
    let links = run_links(
        "github",
        "https://git.example/c#L1",
        "https://dashboard.example.test/#/namespaces/acme-builds/pipelineruns/x?pipelineTask=build",
    )
    .await;
    assert_eq!(
        link_out(&links, "Open the run").2.as_deref(),
        Some("https://dashboard.example.test/#/namespaces/acme-builds/pipelineruns/x")
    );
    assert_eq!(
        link_out(&links, "Open the commit").2.as_deref(),
        Some("https://git.example/c")
    );
}

/// A commit page on another host than the repository isn't taken for it.
#[tokio::test]
async fn a_commit_on_another_host_is_greyed_out() {
    let links = run_links(
        "github",
        "https://elsewhere.example.test/acme/storefront/commit/abc",
        "https://console.example.test/r",
    )
    .await;
    let commit = link_out(&links, "Open the commit");
    assert_eq!(
        (commit.2.as_deref(), commit.3.as_deref()),
        (
            None,
            Some("The run's sha-url names another host than its repo-url")
        )
    );
}

/// A ConfigMap GET that hangs greys its links out after its own deadline;
/// the change is still read.
#[tokio::test(start_paused = true)]
async fn a_hanging_address_read_greys_its_links_out_not_the_change() {
    let reader = one_cluster(|world| {
        world.argocd = std::mem::take(&mut world.argocd).hanging("configmaps");
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("the change is read");
    let why = change.page.as_ref().expect_err("no Kargo page");
    assert_eq!(
        why,
        "Kargo's address isn't known: ConfigMap kargo-api in kargo didn't answer within 5 s"
    );
    let argocd = link_out(&links_out(&change), "Open in Argo CD").clone();
    assert_eq!(
        argocd.3.as_deref(),
        Some(
            "Argo CD's address isn't known: ConfigMap argocd-cm in argocd didn't answer within 5 s"
        )
    );
    assert!(!change.groups.is_empty());
}

/// Without a failed task, the logs are the task's that ended last, by its
/// completion time rather than the order the TaskRuns were listed in.
#[tokio::test]
async fn the_logs_are_the_task_that_ended_last() {
    let reader = one_cluster(|world| {
        annotated(world, "github", "https://git.example/c");
        let mut tasks = Vec::new();
        for (task, ended) in [
            ("push", "2026-01-01T10:05:00Z"),
            ("build", "2026-01-01T10:09:00Z"),
            ("lint", "2026-01-01T10:01:00Z"),
        ] {
            let mut run = task_run();
            run["metadata"]["name"] = json!(format!("storefront-push-x-{task}"));
            run["metadata"]["labels"]["tekton.dev/pipelineTask"] = json!(task);
            run["status"]["podName"] = json!(format!("storefront-push-x-{task}-pod"));
            run["status"]["steps"] = json!([{"container": format!("step-{task}")}]);
            run["status"]["conditions"] = json!([{"type": "Succeeded", "status": "True"}]);
            run["status"]["completionTime"] = json!(ended);
            tasks.push(run);
        }
        world.tekton = std::mem::take(&mut world.tekton).with("taskruns", tasks);
    });
    let change = read(&reader, &place("f-new"), observed_at())
        .await
        .expect("read");
    let links = links_out(&change);
    assert_eq!(
        link_out(&links, "Logs of build").2.as_deref(),
        Some("cluster-key/acme-builds/storefront-push-x-build-pod Some(\"step-build\")")
    );
}
