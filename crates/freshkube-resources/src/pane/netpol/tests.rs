use super::super::tests::mount;
use super::super::{ClusterAccess, Duration, Failure, FailureKind, example};
use super::{Body, Loaded};
use crate::example::netpol::Shape;
use crate::pane::{DetailPane, DetailTarget};
use crate::ui::Tone;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{App, AppContext, Entity, TestAppContext, Window};

const NETPOL: &str = "networkpolicies.networking.k8s.io";

fn target_where(resource: &str, namespace: &str, keep: impl Fn(&str) -> bool) -> DetailTarget {
    let (_, rows) = example::read("prod-fra", resource, Some(namespace), 1_767_312_000).unwrap();
    DetailTarget {
        identity: rows
            .into_iter()
            .find(|row| keep(&row.identity.name))
            .unwrap()
            .identity,
        kind: example::kind(resource).unwrap(),
    }
}

fn policy(namespace: &str, name: &str) -> DetailTarget {
    target_where(NETPOL, namespace, |n| n == name)
}

fn pod(namespace: &str, app: &str) -> DetailTarget {
    target_where("pods", namespace, |n| n.starts_with(&format!("{app}-")))
}

fn open(pane: &Entity<DetailPane>, target: &DetailTarget, shape: Shape, cx: &mut App) {
    pane.update(cx, |pane, cx| {
        pane.close(cx);
        pane.set_netpol_shape(shape);
        pane.open(
            target.clone(),
            ClusterAccess::EXAMPLE,
            "1",
            Duration::ZERO,
            cx,
        )
    });
}

fn directions(pane: &Entity<DetailPane>, cx: &App) -> Vec<(&'static str, Tone, String)> {
    let lines = match &pane.read(cx).netpol.body {
        Some(Body::Policy { directions, .. }) | Some(Body::Applying { directions, .. }) => {
            directions
        }
        _ => panic!("no network policy block"),
    };
    lines
        .iter()
        .map(|d| (d.direction, d.tone, d.verdict.to_owned()))
        .collect()
}

fn verdicts(pane: &Entity<DetailPane>, cx: &App) -> Vec<String> {
    directions(pane, cx).into_iter().map(|d| d.2).collect()
}

#[gpui_kit::test]
fn a_default_deny_policy_denies_both_directions_and_counts_its_pods(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(
            &pane,
            &policy("payments", "default-deny-all"),
            Shape::Mixed,
            cx,
        );
        window.render_frame(cx);
        assert_eq!(
            verdicts(&pane, cx),
            ["Denies everything", "Denies everything"]
        );
        match &pane.read(cx).netpol.body {
            Some(Body::Policy { selector, pods, .. }) => {
                assert!(selector.contains("every pod"), "{selector}");
                assert!(pods.starts_with("Selects "), "{pods}");
                assert!(!pods.contains("at least"));
            }
            _ => panic!("not a policy"),
        }
        assert!(window.find("netpol-ingress").visible());
        assert!(window.find("netpol-pods").visible());
        // The scope is said beside every verdict.
        assert!(window.find("netpol-scope").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_restricting_policy_spells_out_its_peers_and_ports(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(
            &pane,
            &policy("payments", "allow-web-to-api"),
            Shape::Mixed,
            cx,
        );
        window.render_frame(cx);
        assert_eq!(verdicts(&pane, cx), ["Restricts", "Not governed"]);
        match &pane.read(cx).netpol.body {
            Some(Body::Policy { directions, .. }) => {
                assert_eq!(directions[0].rules.len(), 1);
                let rule = directions[0].rules[0].to_string();
                assert!(
                    rule.starts_with("from pods app=frontend in namespaces"),
                    "{rule}"
                );
                assert!(rule.contains("kubernetes.io/metadata.name=web"));
                assert!(rule.ends_with("on TCP port http"), "{rule}");
                assert!(directions[1].rules.is_empty());
            }
            _ => panic!("not a policy"),
        }
        assert!(window.find("netpol-ingress-rule-0").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn open_and_ip_block_policies_read_as_what_they_are(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &policy("web", "open-frontend"), Shape::Mixed, cx);
        window.render_frame(cx);
        assert_eq!(verdicts(&pane, cx), ["Opens to everything", "Not governed"]);
        open(&pane, &policy("web", "egress-private"), Shape::Mixed, cx);
        window.render_frame(cx);
        assert_eq!(verdicts(&pane, cx), ["Not governed", "Restricts"]);
        match &pane.read(cx).netpol.body {
            Some(Body::Policy { directions, .. }) => {
                let rule = directions[1].rules[0].to_string();
                assert_eq!(
                    rule,
                    "to IP range 10.0.0.0/8 except 10.1.0.0/16 on TCP 8000-8100"
                );
            }
            _ => panic!("not a policy"),
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_pod_lists_the_policies_that_apply_and_what_each_direction_comes_to(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod("payments", "api"), Shape::Mixed, cx);
        window.render_frame(cx);
        // A default deny plus allow rules: restricted both ways.
        assert_eq!(verdicts(&pane, cx), ["Restricts", "Restricts"]);
        match &pane.read(cx).netpol.body {
            Some(Body::Applying { policies, .. }) => {
                let names = policies
                    .iter()
                    .map(|p| p.name.to_string())
                    .collect::<Vec<_>>();
                assert_eq!(names, ["default-deny-all", "allow-web-to-api", "allow-dns"]);
            }
            _ => panic!("not a pod's policies"),
        }
        assert!(window.find("netpol-applied-allow-web-to-api").visible());
        assert!(window.find("netpol-scope").visible());

        // Another namespace's policies never apply.
        open(&pane, &pod("web", "frontend"), Shape::Mixed, cx);
        window.render_frame(cx);
        assert_eq!(verdicts(&pane, cx), ["Opens to everything", "Not governed"]);
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_workload_shows_the_policies_of_its_template(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let deployment = target_where("deployments.apps", "payments", |n| n == "api");
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &deployment, Shape::Mixed, cx);
        window.render_frame(cx);
        assert_eq!(verdicts(&pane, cx), ["Restricts", "Restricts"]);
        open(&pane, &deployment, Shape::DefaultDeny, cx);
        window.render_frame(cx);
        assert_eq!(
            verdicts(&pane, cx),
            ["Denies everything", "Denies everything"]
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn no_policy_is_not_governed_and_says_so_plainly(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &pod("payments", "api"), Shape::None, cx);
        window.render_frame(cx);
        assert_eq!(verdicts(&pane, cx), ["Not governed", "Not governed"]);
        assert!(window.find("netpol-none").visible());
        // Not governed is no failure and offers no retry.
        assert!(window.try_find("netpol-retry").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_refused_or_cut_read_is_unknown_with_a_retry_never_no_policy(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        for shape in [Shape::Refused, Shape::Cut] {
            open(&pane, &pod("payments", "api"), shape, cx);
            window.render_frame(cx);
            match &pane.read(cx).netpol.body {
                Some(Body::Message { tone, text }) => {
                    assert_eq!(*tone, Tone::Unknown);
                    assert!(text.contains("unknown"), "{text}");
                }
                _ => panic!("expected an unknown message"),
            }
            assert!(window.find("netpol-message").visible());
            assert!(window.find("netpol-retry").visible());
            assert!(window.try_find("netpol-none").is_none());
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_policys_pod_count_is_unknown_when_refused_and_at_least_when_cut(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        open(&pane, &policy("payments", "allow-dns"), Shape::Refused, cx);
        window.render_frame(cx);
        match &pane.read(cx).netpol.body {
            Some(Body::Policy {
                pods, pods_tone, ..
            }) => {
                assert_eq!(*pods_tone, Tone::Unknown);
                assert!(pods.contains("unknown"), "{pods}");
            }
            _ => panic!("not a policy"),
        }
        // What the policy says still shows.
        assert_eq!(verdicts(&pane, cx), ["Not governed", "Restricts"]);
        assert!(window.find("netpol-retry").visible());

        open(&pane, &policy("payments", "allow-dns"), Shape::Cut, cx);
        window.render_frame(cx);
        match &pane.read(cx).netpol.body {
            Some(Body::Policy { pods, .. }) => {
                assert!(pods.starts_with("Selects at least"), "{pods}")
            }
            _ => panic!("not a policy"),
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn other_kinds_show_no_network_policy_block(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window, cx| {
        let (_, rows) = example::read("prod-fra", "nodes", None, 1_767_312_000).unwrap();
        let node = DetailTarget {
            identity: rows.into_iter().next().unwrap().identity,
            kind: example::kind("nodes").unwrap(),
        };
        open(&pane, &node, Shape::Mixed, cx);
        window.render_frame(cx);
        assert!(window.try_find("netpol-section").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_old_answer_cannot_overwrite_a_newer_one_or_a_hidden_pane(cx: &mut TestAppContext) {
    let (_runtime, pane, _handle, _) = mount(cx);
    let target = pod("payments", "api");
    cx.update(|cx| {
        open(&pane, &target, Shape::Mixed, cx);
        let seq = pane.read(cx).netpol.seq;
        let before = verdicts(&pane, cx);
        let refused = || Failure::new(FailureKind::Forbidden, "late");
        pane.update(cx, |pane, cx| {
            pane.finish_netpol(
                &target.identity,
                seq.wrapping_sub(1),
                Loaded::Applying(Err(refused())),
                cx,
            )
        });
        assert_eq!(verdicts(&pane, cx), before);
        pane.update(cx, |pane, cx| pane.set_active(false, cx));
        let seq = pane.read(cx).netpol.seq;
        pane.update(cx, |pane, cx| {
            pane.finish_netpol(&target.identity, seq, Loaded::Applying(Err(refused())), cx)
        });
        assert_eq!(verdicts(&pane, cx), before);
        assert!(pane.read(cx).netpol.job.is_none());
    });
}

#[gpui_kit::test]
fn the_section_and_its_rows_stay_inside_the_pane(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update_window(handle, |_, window: &mut Window, cx| {
        open(
            &pane,
            &policy("payments", "allow-web-to-api"),
            Shape::Mixed,
            cx,
        );
        window.render_frame(cx);
        let section = window.find("netpol-section").bounds();
        let pane_bounds = window.find("detail-overview").bounds();
        assert!(section.left() >= pane_bounds.left() - gpui_kit::px(1.));
        assert!(section.right() <= pane_bounds.right() + gpui_kit::px(1.));
        for id in ["netpol-ingress", "netpol-ingress-rule-0", "netpol-scope"] {
            let bounds = window.find(id).bounds();
            assert!(bounds.right() <= section.right() + gpui_kit::px(1.), "{id}");
        }
    })
    .unwrap();
}
