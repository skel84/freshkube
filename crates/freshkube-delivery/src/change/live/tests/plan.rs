use super::*;

#[test]
fn the_plan_keeps_the_setups_log_switch_and_optional_url() {
    let freight = crate::kargo::parse_freight(&freight("f-new", NEW, SHA)).unwrap();
    let mut at = place("f-new");
    let off = plan_of(&at, &freight);
    assert!(!off.transparency_log);
    assert_eq!(off.transparency_url, None);
    at.transparency_log = true;
    for url in [Some("https://rekor.example.test/log"), None] {
        at.transparency_url = url.map(str::to_owned);
        let plan = plan_of(&at, &freight);
        assert!(plan.transparency_log);
        assert_eq!(plan.transparency_url.as_deref(), url);
    }
}
