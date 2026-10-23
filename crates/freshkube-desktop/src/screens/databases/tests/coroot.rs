use super::*;

#[gpui_kit::test]
fn every_coroot_state_keeps_the_operators_answer_and_says_so(cx: &mut TestAppContext) {
    let (_runtime, page, window) = mount(cx, 1280.);
    for (shape, words) in [
        ("coroot-none", "names no Coroot cluster"),
        ("coroot-not-found", "Coroot lists no application"),
        (
            "coroot-unreachable",
            "The last answer is dimmed and not used",
        ),
    ] {
        cx.update_window(window.into(), |_, window, cx| {
            page.update(cx, |page, cx| {
                page.shape = shape.into();
                page.loader.reset();
                page.read(cx);
            });
            window.render_frame(cx);
            let rows = &page.read(cx).display.rows;
            // The operator calls both databases ready, and nothing worsens that.
            assert!(
                rows.iter().all(|row| row.cells[4] == "Healthy"),
                "{shape}: {:?}",
                rows.iter().map(|r| r.cells[4].clone()).collect::<Vec<_>>()
            );
            assert!(
                rows.iter()
                    .all(|row| row.chip == Some("Operator") || row.chip == Some("Both"))
            );
            let coroot = rows[0].sections.iter().find(|s| s.id == "coroot").unwrap();
            assert!(
                coroot.fields[0].value.contains(words),
                "{shape}: {}",
                coroot.fields[0].value
            );
            if shape == "coroot-unreachable" {
                // What it said before is shown, dimmed, and not used.
                assert!(coroot.fields.iter().skip(1).all(|f| f.dim));
                assert!(coroot.fields.len() > 1);
            }
        })
        .unwrap();
    }
}

#[test]
fn a_coroot_app_without_a_report_is_not_called_missing() {
    use databases::coroot::{CorootState, Evidence};
    let inv = databases::example::inventory(at());
    let evidence = Evidence {
        states: inv
            .databases
            .items
            .iter()
            .map(|db| (db.identity.uid.clone(), CorootState::NoReport))
            .collect(),
    };
    let display = super::display::Display::new(Some(&inv), &evidence, Mode::Clusters, at());
    let row = display
        .rows
        .iter()
        .find(|row| row.identity.name == "orders-db")
        .unwrap();
    let value = &row
        .sections
        .iter()
        .find(|s| s.id == "coroot")
        .unwrap()
        .fields[0]
        .value;
    assert!(value.contains("has no Postgres report"), "{value}");
    assert!(!value.contains("lists no application"));
}
