use super::*;
use freshkube_core::write::Tier;

/// Opens `target` on its YAML tab, settled.
fn on_yaml(
    cx: &mut TestAppContext,
    pane: &Entity<DetailPane>,
    handle: AnyWindowHandle,
    target: &DetailTarget,
) {
    cx.update_window(handle, |_, window, cx| {
        open(pane, target, Duration::ZERO, cx);
        window.render_frame(cx);
        window.click("detail-tab-yaml", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn press(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn label(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> Option<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .map(|element| element.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

/// Puts `text` in the editor as typing would.
fn type_text(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    pane: &Entity<DetailPane>,
    text: String,
) {
    cx.update_window(handle, |_, window, cx| {
        let state = pane.read(cx).edit_state().unwrap();
        state.update(cx, |state, cx| state.set_value(text, window, cx));
        pane.update(cx, |pane, cx| pane.edited(cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn editor_text(cx: &mut TestAppContext, pane: &Entity<DetailPane>) -> String {
    cx.update(|cx| {
        pane.read(cx)
            .edit_state()
            .unwrap()
            .read(cx)
            .value()
            .to_string()
    })
}

/// The text with one more label under `labels:`, which is a change of a field
/// the edit owns.
fn with_label(text: &str) -> String {
    let mut out = vec![];
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        out.push(line.to_owned());
        if line.trim_end() == "  labels:"
            && let Some(next) = lines.peek()
        {
            let indent = next.len() - next.trim_start().len();
            out.push(format!("{}edited-by-test: \"yes\"", " ".repeat(indent)));
        }
    }
    out.join("\n") + "\n"
}

#[gpui_kit::test]
fn edit_opens_an_editor_marks_changed_lines_and_review_hands_core_s_description_on(
    cx: &mut TestAppContext,
) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = running_pod();
    on_yaml(cx, &pane, handle, &pod);
    assert!(label(cx, handle, "detail-edit-view").is_none());
    press(cx, handle, "detail-edit");

    assert!(label(cx, handle, "detail-edit-view").is_some());
    assert_eq!(
        label(cx, handle, "detail-edit-count").as_deref(),
        Some("No changes yet")
    );
    // One line until opened.
    assert!(
        label(cx, handle, "detail-edit-locked")
            .unwrap()
            .contains("Locked")
    );
    assert!(label(cx, handle, "detail-edit-locked-list").is_none());
    press(cx, handle, "detail-edit-locked");
    let locked = label(cx, handle, "detail-edit-locked-list").unwrap();
    assert!(
        locked.contains("status") && locked.contains("managedFields"),
        "{locked}"
    );
    let text = editor_text(cx, &pane);
    assert!(!text.contains("managedFields:"), "{text}");

    // Review with nothing changed says so and sends nothing on.
    press(cx, handle, "detail-edit-review");
    assert_eq!(
        label(cx, handle, "detail-edit-error").as_deref(),
        Some("Nothing was changed.")
    );
    assert!(emitted.borrow().is_empty());

    type_text(cx, handle, &pane, with_label(&text));
    // The note was about the text before the edit.
    assert!(label(cx, handle, "detail-edit-error").is_none());
    assert_eq!(
        label(cx, handle, "detail-edit-count").as_deref(),
        Some("1 line changed")
    );
    press(cx, handle, "detail-edit-review");
    let events = emitted.borrow();
    let [DetailEvent::EditYaml(action)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(action.tier, Tier::SpecChange);
    assert_eq!(action.changes.len(), 1);
}

#[gpui_kit::test]
fn command_s_reviews_the_edit_and_cancel_puts_the_yaml_back(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = running_pod();
    on_yaml(cx, &pane, handle, &pod);
    press(cx, handle, "detail-edit");
    let text = editor_text(cx, &pane);
    type_text(cx, handle, &pane, with_label(&text));
    cx.update_window(handle, |_, window, cx| window.press("secondary-s", cx))
        .unwrap();
    cx.run_until_parked();
    assert!(matches!(
        emitted.borrow().as_slice(),
        [DetailEvent::EditYaml(_)]
    ));

    press(cx, handle, "detail-edit-cancel");
    assert!(label(cx, handle, "detail-edit-view").is_none());
    assert!(pane.read_with(cx, |pane, _| pane.edit_state().is_none()));
    // The text is gone: the next Edit starts from the object again.
    press(cx, handle, "detail-edit");
    assert_eq!(editor_text(cx, &pane), text);
}

#[gpui_kit::test]
fn an_edit_that_changes_the_identity_is_refused_in_the_editor(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, emitted) = mount(cx);
    let (pod, _) = running_pod();
    on_yaml(cx, &pane, handle, &pod);
    press(cx, handle, "detail-edit");
    let text = editor_text(cx, &pane);
    type_text(
        cx,
        handle,
        &pane,
        text.replacen("kind: Pod", "kind: Service", 1),
    );
    press(cx, handle, "detail-edit-review");
    let error = label(cx, handle, "detail-edit-error").unwrap();
    assert!(
        error.starts_with("kind is the object's identity"),
        "{error}"
    );
    assert!(emitted.borrow().is_empty());
}

#[gpui_kit::test]
fn a_cluster_that_refuses_writes_leaves_edit_off(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    on_yaml(cx, &pane, handle, &pod);
    pane.update(cx, |pane, cx| {
        pane.set_write_block(Some("This cluster is read-only."), cx)
    });
    press(cx, handle, "detail-edit");
    assert!(label(cx, handle, "detail-edit-view").is_none());
    assert!(pane.read_with(cx, |pane, _| pane.edit_state().is_none()));
}

/// Binds the app's offer desk to example data answering `shape`, and has it
/// asked about the open object's Edit, as the screen does with the row's
/// actions.
fn offers_answering(cx: &mut TestAppContext, shape: freshkube_core::access::example::Shape) {
    cx.update(|cx| {
        let desk = freshkube_reads::existing_offer_desk(cx).expect("the pane made the desk");
        let source = crate::screen::ClusterSource {
            id: example::connection(CONTEXT),
            context: CONTEXT.into(),
            access: ClusterAccess::EXAMPLE,
        };
        desk.update(cx, |desk, cx| {
            desk.set_example(shape);
            desk.bind(Some(&source), cx);
        });
    });
}

/// The object is open and the desk is asked about its Edit, then answers.
fn asked_about_edit(cx: &mut TestAppContext, pane: &Entity<DetailPane>) {
    cx.update(|cx| {
        let needs = pane.read(cx).edit_needs();
        let desk = freshkube_reads::existing_offer_desk(cx).unwrap();
        let owner = desk.entity_id();
        desk.update(cx, |desk, cx| desk.want(owner, &needs, cx));
    });
    cx.executor()
        .advance_clock(freshkube_reads::OFFER_PAUSE + Duration::from_millis(1));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn a_cluster_that_refuses_the_edit_leaves_it_off_with_the_reason(cx: &mut TestAppContext) {
    use freshkube_core::access::example::Shape;
    let (_runtime, pane, handle, _) = mount(cx);
    offers_answering(cx, Shape::ActionsRefused);
    let (pod, _) = running_pod();
    let space = pod.identity.namespace.clone();
    assert!(!matches!(space.as_str(), "shop" | "payments"), "{space}");
    on_yaml(cx, &pane, handle, &pod);
    asked_about_edit(cx, &pane);
    let refusal = pane.read_with(cx, |pane, _| pane.access_refusal());
    assert_eq!(refusal, Some(format!("Needs patch on pods in {space}")));
    // That is what Edit's tooltip says, shown, not hidden, and it opens no
    // editor.
    assert_eq!(
        pane.read_with(cx, |pane, _| pane.edit_reason()),
        Some(format!("Needs patch on pods in {space}"))
    );
    assert!(label(cx, handle, "detail-edit").is_some());
    press(cx, handle, "detail-edit");
    assert!(label(cx, handle, "detail-edit-view").is_none());
    assert!(pane.read_with(cx, |pane, _| pane.edit_state().is_none()));
    // The level's reason is the one it says when both apply.
    pane.update(cx, |pane, cx| {
        pane.set_write_block(Some("This cluster is read-only."), cx)
    });
    assert_eq!(
        pane.read_with(cx, |pane, _| pane.edit_reason()),
        Some("This cluster is read-only.".to_owned())
    );
    press(cx, handle, "detail-edit");
    assert!(pane.read_with(cx, |pane, _| pane.edit_state().is_none()));
}

#[gpui_kit::test]
fn an_edit_whose_review_did_not_answer_stays_on(cx: &mut TestAppContext) {
    use freshkube_core::access::{Offer, example::Shape};
    let (_runtime, pane, handle, _) = mount(cx);
    offers_answering(cx, Shape::Failed);
    let (pod, _) = running_pod();
    on_yaml(cx, &pane, handle, &pod);
    asked_about_edit(cx, &pane);
    let offer = pane.read_with(cx, |pane, _| pane.edit_offer.clone());
    assert_eq!(offer, Offer::Unchecked);
    assert_eq!(pane.read_with(cx, |pane, _| pane.edit_reason()), None);
    press(cx, handle, "detail-edit");
    assert!(pane.read_with(cx, |pane, _| pane.edit_state().is_some()));
}

#[gpui_kit::test]
fn another_object_drops_the_edit(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (pod, _) = running_pod();
    on_yaml(cx, &pane, handle, &pod);
    press(cx, handle, "detail-edit");
    assert!(pane.read_with(cx, |pane, _| pane.edit_state().is_some()));
    let (other, _) = crashing_pod();
    cx.update_window(handle, |_, _, cx| open(&pane, &other, Duration::ZERO, cx))
        .unwrap();
    cx.run_until_parked();
    assert!(pane.read_with(cx, |pane, _| pane.edit_state().is_none()));
}

#[gpui_kit::test]
fn a_secret_is_edited_without_its_values(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    let (secret, _) = target("secrets", |_, name| name == "ledger-database");
    on_yaml(cx, &pane, handle, &secret);
    press(cx, handle, "detail-edit");
    let text = editor_text(cx, &pane);
    assert!(
        !text.contains("\ndata:") && !text.contains("‹not read›"),
        "no placeholder to edit: {text}"
    );
    let block = label(cx, handle, "detail-edit-locked-values").unwrap();
    assert!(
        block.contains("only its metadata can be edited")
            && block.contains("data: ‹not read›; stringData: ‹not read›"),
        "{block}"
    );
}

/// At 760x560 with text 20 the locked list is one line, so the editor keeps
/// most of the pane (#983).
#[gpui_kit::test]
fn the_editor_keeps_most_of_a_short_pane_at_text_20(cx: &mut TestAppContext) {
    let (_runtime, pane, handle, _) = mount(cx);
    cx.update(|cx| crate::text_size::install(None, cx));
    cx.update(|cx| crate::text_size::set(20., cx));
    cx.simulate_window_resize(handle, gpui_kit::size(px(760.), px(480.)));
    let (secret, _) = target("secrets", |_, name| name == "ledger-database");
    on_yaml(cx, &pane, handle, &secret);
    press(cx, handle, "detail-edit");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let inspector = window.find("detail-inspector").bounds();
        let editor = window.find("detail-edit-editor").bounds();
        // Closed, the locked fields take no line of their own.
        assert!(window.try_find("detail-edit-locked-list").is_none());
        assert!(
            editor.size.height * 2. >= inspector.size.height,
            "the editor has less than half the pane: {editor:?} of {inspector:?}"
        );
    })
    .unwrap();
}
