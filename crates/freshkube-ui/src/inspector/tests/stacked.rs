use super::*;

/// At 760 by 560 with text size 20 a stacked split is at its least
/// heights: the table still shows its header, the group row that leads
/// and one whole data row above its footer, and the inspector its heading
/// and first field, while the frame scrolls for the rest.
#[gpui_kit::test]
fn a_short_stacked_split_keeps_a_whole_row_and_the_first_field(cx: &mut TestAppContext) {
    install(cx);
    cx.update(|cx| {
        set_store(Rc::new(MemorySizes::default()), cx);
        crate::text_size::set(20., cx);
    });
    let handle = cx.open_window(size(px(760.), px(560.)), |window, cx| {
        let view = cx.new(|cx| ShortPage::new(InspectorSplit::new(PAGE, cx)));
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        settle(window, cx);
        let bounds = |id: &'static str| window.find(id).bounds();
        let split = bounds("split");
        let pane = bounds("table-pane");
        let (rows, row, footer) = (bounds("rows"), bounds("data-row"), bounds("footer"));
        let (inspector, field) = (bounds("inspector"), bounds("inspector-field"));
        assert!(
            split.bottom() > window.viewport_size().height,
            "the frame scrolls: the split ends at {:?}",
            split.bottom()
        );
        close(
            "the table at its least",
            pane.size.height,
            dp_px(LIST_MIN_HEIGHT, window),
        );
        assert!(
            row.bottom() <= rows.bottom() + px(0.5),
            "the first data row is whole: it ends at {:?}, the rows at {:?}",
            row.bottom(),
            rows.bottom()
        );
        assert!(row.bottom() <= footer.top() + px(0.5));
        assert!(
            field.bottom() <= inspector.bottom(),
            "the first field shows: it ends at {:?}, the inspector at {:?}",
            field.bottom(),
            inspector.bottom()
        );
    })
    .unwrap();
}

/// At 760 by 560 with text size 20 the wheel over a stacked inspector's body
/// scrolls the body, and the frame stays until the body reaches its end.
#[gpui_kit::test]
fn the_wheel_over_a_short_stacked_body_scrolls_the_body(cx: &mut TestAppContext) {
    install(cx);
    cx.update(|cx| {
        set_store(Rc::new(MemorySizes::default()), cx);
        crate::text_size::set(20., cx);
    });
    let handle = cx.open_window(size(px(760.), px(560.)), |window, cx| {
        let view = cx.new(|cx| ShortPage::new(InspectorSplit::new(PAGE, cx)));
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        settle(window, cx);
        let toolbar = window.find("toolbar").bounds().top();
        let tall = window.find("inspector-tall").bounds().top();
        window.scroll(
            "inspector-body",
            ScrollDelta::Pixels(point(px(0.), px(-40.))),
            cx,
        );
        settle(window, cx);
        close(
            "the frame stays",
            window.find("toolbar").bounds().top(),
            toolbar,
        );
        close(
            "the body moved",
            window.find("inspector-tall").bounds().top(),
            tall - px(40.),
        );
        // At its end the body lets the wheel through, and the frame scrolls.
        let wheel = |window: &mut Window, delta: Pixels, cx: &mut App| {
            window.scroll(
                "inspector-body",
                ScrollDelta::Pixels(point(px(0.), delta)),
                cx,
            );
            settle(window, cx);
        };
        let left = window.find("inspector-tall").bounds().bottom()
            - window.find("inspector-body").bounds().bottom();
        wheel(window, -left, cx);
        close(
            "the frame stays",
            window.find("toolbar").bounds().top(),
            toolbar,
        );
        let end = window.find("inspector-tall").bounds().bottom();
        let frame = window.find("toolbar").bounds().top();
        wheel(window, px(-100.), cx);
        assert!(
            window.find("toolbar").bounds().top() < frame,
            "the frame scrolls once the body is at its end"
        );
        close(
            "the body stays at its end",
            window.find("inspector-tall").bounds().bottom(),
            end + (window.find("toolbar").bounds().top() - frame),
        );
    })
    .unwrap();
}

struct TallBanner(InspectorSplit, Rc<Cell<usize>>);

impl Render for TallBanner {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.1.clone();
        div().size_full().flex().child(split(
            "banner-split",
            &self.0,
            false,
            div().size_full().into_any_element(),
            Some(
                Inspector::new("banner-inspector")
                    .heading("Pruned PipelineRun")
                    .banner(Some(
                        div().h(dp(500.)).child("Not found; retained evidence"),
                    ))
                    .child("Retained fields")
                    .footer(Some(
                        gpui_kit::component::button::Button::new("banner-forward")
                            .label("Forward")
                            .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                    ))
                    .render(cx)
                    .into_any_element(),
            ),
            window,
        ))
    }
}

#[gpui_kit::test]
fn stacked_inspector_with_tall_banner_keeps_footer_reachable_at_text_twenty(
    cx: &mut TestAppContext,
) {
    install(cx);
    cx.update(|cx| crate::text_size::set(20., cx));
    let clicked = Rc::new(Cell::new(0));
    let handle = cx.open_window(size(px(760.), px(560.)), |window, cx| {
        let view = cx.new(|cx| {
            TallBanner(
                InspectorSplit::new("banner-regression", cx),
                clicked.clone(),
            )
        });
        Root::new(view, window, cx)
    });
    cx.update_window(handle.into(), |_, window, cx| {
        settle(window, cx);
        let inspector = window.find("banner-inspector").bounds();
        let action = window.find("banner-forward").bounds();
        assert!(
            action.bottom() <= inspector.bottom(),
            "footer {action:?}, inspector {inspector:?}"
        );
        window.click("banner-forward", cx);
        assert_eq!(clicked.get(), 1);
    })
    .unwrap();
}
