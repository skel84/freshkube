use super::{TraceLink, TraceModel, verdict_tone};
use crate::ui::Tone;
use freshkube_core::resources::traffic::routes::RouteVerdict;
use freshkube_core::resources::traffic::routes::example::{Shape, read};

fn model(name: &str) -> TraceModel {
    let read = read(Shape::All).unwrap();
    let entry = read
        .routes
        .iter()
        .find(|entry| entry.facts.name == name)
        .unwrap_or_else(|| panic!("no route {name}"));
    TraceModel::new(entry, "test-trace")
}

#[test]
fn unknown_is_never_green_and_not_serving_is_never_unknown() {
    assert_eq!(verdict_tone(RouteVerdict::Serving), Tone::Good);
    assert_eq!(verdict_tone(RouteVerdict::Unknown), Tone::Unknown);
    assert_eq!(verdict_tone(RouteVerdict::NotServing), Tone::Crit);
    assert_eq!(verdict_tone(RouteVerdict::Partly), Tone::Warn);
    for (name, tone) in [
        ("promo", Tone::Unknown),
        ("web", Tone::Good),
        ("checkout", Tone::Crit),
        ("cart-split", Tone::Warn),
    ] {
        assert_eq!(verdict_tone(model(name).verdict), tone, "{name}");
    }
}

#[test]
fn steps_after_the_break_are_not_reached_and_carry_no_owner() {
    let checkout = model("checkout");
    let branch = &checkout.parents[0].branches[0];
    let states = branch.steps.iter().map(|s| s.state).collect::<Vec<_>>();
    assert_eq!(states, ["Broken", "Not reached", "Not reached"]);
    assert_eq!(branch.steps[0].owner, Some("Platform"));
    assert_eq!(branch.steps[1].tone, Tone::Waiting);
    assert!(branch.steps[1].owner.is_none());
    // A passing step names no one.
    assert!(checkout.parents[0].steps.iter().all(|s| s.owner.is_none()));
}

#[test]
fn a_step_quotes_the_route_against_the_gateway_and_keeps_the_controller_apart() {
    let docs = model("docs");
    let listener = &docs.parents[0].steps[2];
    assert_eq!(listener.name, "Listener accepts");
    assert!(listener.asks.contains("docs.example.org"));
    assert!(listener.serves.contains("*.example.com"));
    let checkout = model("checkout");
    let refs = &checkout.parents[0].branches[0].steps[0];
    assert!(refs.controller.as_deref().unwrap().contains("RefNotPermitted"));
    assert!(!refs.reason.contains("RefNotPermitted"));
}

#[test]
fn the_split_is_drawn_as_shares_and_a_broken_branch_is_its_own_row() {
    let split = model("cart-split");
    let shares = split.parents[0]
        .branches
        .iter()
        .map(|b| b.share.as_deref().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(shares, [Some("80%".into()), Some("20%".into())]);
    assert_eq!(split.parents[0].branches[0].state, "Serving");
    assert_eq!(split.parents[0].branches[1].state, "Not serving");
    assert!(split.headline().starts_with("Partly serving · stops at Endpoints ready"));
}

#[test]
fn the_proposed_grant_is_text_scoped_to_the_exact_pair() {
    let checkout = model("checkout");
    let yaml = checkout.proposals().collect::<Vec<_>>();
    assert_eq!(yaml.len(), 1);
    assert!(yaml[0].contains("kind: ReferenceGrant"));
    assert!(yaml[0].contains("namespace: shop"));
    assert!(yaml[0].contains("namespace: payments"));
    assert!(yaml[0].contains("name: ledger"));
    assert!(model("web").proposals().next().is_none());
}

#[test]
fn a_stale_verdict_is_flagged_and_a_service_parent_has_no_steps() {
    let stale = model("stale");
    assert!(stale.stale);
    assert_eq!(stale.verdict, RouteVerdict::Serving);
    let mesh = model("mesh-cart");
    assert_eq!(mesh.verdict, RouteVerdict::Mesh);
    assert!(mesh.parents[0].steps.is_empty() && mesh.parents[0].gateway.is_none());
}

#[test]
fn links_name_the_gateway_and_the_service_at_the_end_of_a_branch() {
    let web = model("web");
    assert_eq!(
        web.parents[0].gateway,
        Some(TraceLink::Gateway {
            namespace: "infra".into(),
            name: "edge".into()
        })
    );
    assert_eq!(
        web.parents[0].branches[0].service,
        Some(TraceLink::Service {
            namespace: "shop".into(),
            name: "cart".into()
        })
    );
    // A rule with no backend has nothing to link to.
    assert!(model("empty").parents[0].branches[0].service.is_none());
}
