use super::*;

#[gpui_kit::test]
fn a_click_on_the_info_icon_copies_the_promql_as_sent(cx: &mut TestAppContext) {
    let (dashboard, specs) = cluster();
    let cpu = index_of(&specs, "CPU usage by node");
    let (handle, panels) = mount(cx, specs);
    answer(cx, &dashboard, &panels);
    frame(cx, handle);
    cx.update_window(handle, |_, window, cx| {
        window.click(format!("monitoring-panel-{cpu}-query"), cx)
    })
    .unwrap();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert!(copied.contains("node_cpu_seconds_total"), "{copied}");
    assert!(!copied.contains("$node"), "{copied}");
}
