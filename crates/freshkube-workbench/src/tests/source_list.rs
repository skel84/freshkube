use super::*;

/// The source list: rows of the table's height at 13 and 20, a click and
/// the keyboard opening rows, the labels passed over, and an API group
/// that fails offering Retry.
#[gpui_kit::test]
fn the_source_list_story_opens_rows_by_click_and_keyboard(cx: &mut TestAppContext) {
    use super::stories::source_list::{Key, SourceListStory};
    let (handle, workbench) = open(cx, super::stories::find("source-list").unwrap());
    let story = cx.update(|cx| story::<SourceListStory>(&workbench, cx));
    cx.update_window(handle, |_, window, cx| {
        for (size, height) in [(13., 26.), (20., 40.)] {
            text_size::set(size, cx);
            window.render_frame(cx);
            for id in ["source-list-pods", "source-list-workloads", "story-graph"] {
                assert_eq!(
                    window.find(id).bounds().size.height,
                    px(height),
                    "{id} at {size}"
                );
            }
        }
        text_size::set(13., cx);
        window.render_frame(cx);
        // A long label is cut at the column's edge: its text is wider than
        // the room the row gives it, and the tooltip holds all of it.
        let label = "ValidatingAdmissionPolicyBindings";
        let style = window.text_style();
        let text = window.text_system().shape_line(
            label.into(),
            freshkube_ui::ui::dp_px(13., window),
            &[style.to_run(label.len())],
            None,
        );
        let room = window.find("source-list-bindings-label").bounds();
        assert!(
            room.size.width < text.width,
            "{:?} fits {:?}",
            room.size.width,
            text.width
        );
        let column = window.find("source-list-column").bounds();
        assert!(room.right() <= column.right());
        let story_list = story.read(cx).list();
        let row = story_list
            .lines()
            .iter()
            .filter_map(|line| line.row())
            .find(|row| row.label().as_ref() == label)
            .unwrap();
        assert!(row.tip().starts_with(label));
        assert_eq!(window.find("source-list-pods").selected(), Some(true));
        assert_eq!(window.find("story-source-list").selected(), Some(true));

        window.click("source-list-deployments", cx);
        window.render_frame(cx);
        assert_eq!(story.read(cx).current(), Key::Kind("deployments"));
        assert_eq!(
            window.find("source-list-deployments").selected(),
            Some(true)
        );
        assert_ne!(window.find("source-list-pods").selected(), Some(true));

        // The keyboard starts on the row shown and passes the labels.
        let focus = story.read(cx).list().focus_handle().clone();
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("down", cx);
        window.press("down", cx);
        assert_eq!(story.read(cx).list().cursor(), Some(&Key::Group));
        window.press("enter", cx);
        window.render_frame(cx);
        assert_eq!(window.find("source-list-group").expanded(), Some(false));
        assert!(window.try_find("source-list-widgets").is_none());
        window.press("enter", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(story.read(cx).current(), Key::Kind("widgets"));
        window.press("end", cx);
        window.press("enter", cx);
        assert_eq!(
            story.read(cx).current(),
            Key::Namespace(Some("observability"))
        );
        window.press("home", cx);
        window.press("enter", cx);
        assert_eq!(story.read(cx).current(), Key::Kind("health"));

        // A failed group says why and retries.
        window.click("source-list-failed", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("source-list-group-status").label(),
            Some("Timed out")
        );
        window.click("source-list-group-status-retry", cx);
        window.render_frame(cx);
        assert!(window.find("source-list-widgets").visible());
    })
    .unwrap();
}
