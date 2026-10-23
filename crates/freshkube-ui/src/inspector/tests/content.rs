use super::*;

/// An inspector with tabs: a heading, a banner, the tab strip, content
/// that lays itself out, and a footer.
struct Tabbed(TabStrip);

impl Render for Tabbed {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Inspector::new("inspector")
            .heading(div().id("inspector-title").test_support().child("api-7d9"))
            .banner(Some(div().h(px(30.)).child("Showing it as last read.")))
            .tabs(
                &self.0,
                self.0
                    .row("tabs-row", 0)
                    .child(tab("tab-overview", "Overview", true, cx))
                    .child(tab("tab-yaml", "YAML", false, cx)),
            )
            .child(div().id("unused").test_support())
            .content(
                div()
                    .id("content-tall")
                    .test_support()
                    .flex_none()
                    .h(px(2000.))
                    .w_full(),
            )
            .footer(Some("Copied the document"))
            .render(cx)
    }
}

#[gpui_kit::test]
fn tabs_sit_under_the_heading_and_the_content_fills_the_rest(cx: &mut TestAppContext) {
    install(cx);
    let handle = cx.open_window(size(px(600.), px(500.)), |window, cx| {
        let view = cx.new(|_| Tabbed(TabStrip::default()));
        Root::new(view, window, cx)
    });
    let handle: AnyWindowHandle = handle.into();
    cx.run_until_parked();
    for text_size in [13., 20.] {
        cx.update_window(handle, |_, _, cx| crate::text_size::set(text_size, cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let inspector = window.find("inspector").bounds();
            let heading = window.find("inspector-heading").bounds();
            let banner = window.find("inspector-banner").bounds();
            let tabs = window.find("inspector-tabs").bounds();
            let overview = window.find("tab-overview").bounds();
            let yaml = window.find("tab-yaml").bounds();
            let content = window.find("inspector-content").bounds();
            let footer = window.find("inspector-footer").bounds();
            assert!(banner.top() >= heading.bottom(), "{heading:?} {banner:?}");
            assert!(tabs.top() >= banner.bottom(), "{banner:?} {tabs:?}");
            close(
                "tab height",
                overview.size.height,
                dp_px(TAB_HEIGHT, window),
            );
            close("same row", yaml.top(), overview.top());
            close(
                "tabs inset",
                overview.left() - inspector.left(),
                dp_px(PANE_PADDING, window),
            );
            close("strip", tabs.bottom(), overview.bottom() + px(1.));
            close("content under the strip", content.top(), tabs.bottom());
            close("footer under the content", footer.top(), content.bottom());
            close("footer at the bottom", footer.bottom(), inspector.bottom());
            close("fills", inspector.bottom(), window.viewport_size().height);
            // The content lays itself out: the inspector scrolls nothing
            // and draws no body.
            close(
                "tall content starts at the top",
                window.find("content-tall").bounds().top(),
                content.top(),
            );
            assert!(window.try_find("inspector-body").is_none());
            assert!(window.try_find("unused").is_none());
            assert_eq!(window.find("tab-overview").selected(), Some(true));
            assert_eq!(window.find("tab-yaml").selected(), Some(false));
        })
        .unwrap();
    }
}
