use super::*;

#[gpui_kit::test]
fn a_superseded_panel_answer_cannot_end_the_current_read(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.ask_again(cx);
            let board = page.board.as_ref().unwrap();
            let id = board.id;
            let index = board
                .slots
                .iter()
                .position(|slot| slot.request.is_some())
                .unwrap();
            let request = board.slots[index].request.clone().unwrap();
            let window = board.window.unwrap();
            let generation = page.generation;
            page.ask_again(cx);
            page.land(
                &request,
                move |page| {
                    page.board
                        .as_mut()
                        .filter(|board| board.id == id)
                        .map(|board| &mut board.slots[index].read)
                },
                Err(QueryError::unsupported("an older panel answer")),
                cx,
                move |page, result, cx| page.answered(index, generation, window, result, cx),
            );
            let slot = &page.board.as_ref().unwrap().slots[index];
            assert!(slot.read.is_loading(), "the old answer ended the new read");
            assert!(slot.request.is_some());
            assert!(!slot.failed);
        })
    });
    cx.run_until_parked();
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
}

#[gpui_kit::test]
fn a_superseded_variables_answer_cannot_replace_the_new_resolution(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.resolve_variables(cx);
            let board = page.board.as_ref().unwrap();
            let id = board.id;
            let request = board.resolving.clone().unwrap();
            page.resolve_variables(cx);
            page.land(
                &request,
                move |page| {
                    page.board
                        .as_mut()
                        .filter(|board| board.id == id)
                        .map(|board| &mut board.variables_read)
                },
                Err(QueryError::unsupported("an older variables answer")),
                cx,
                move |page, result, cx| page.variables_resolved(id, result, cx),
            );
            let board = page.board.as_ref().unwrap();
            assert!(board.variables_read.is_loading());
            assert!(board.resolving.is_some());
            assert!(board.variable_error.is_none());
        })
    });
    cx.run_until_parked();
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
    assert!(cx.read(|cx| {
        page.read(cx)
            .board
            .as_ref()
            .unwrap()
            .variable_error
            .is_none()
    }));
}

#[gpui_kit::test]
fn a_panel_answer_from_before_hiding_cannot_end_the_resumed_read(cx: &mut TestAppContext) {
    let (_runtime, handle, page) = mount(cx, Some(example_source()));
    show(cx, handle, &page);
    cx.update(|cx| {
        page.update(cx, |page, cx| {
            page.ask_again(cx);
            let board = page.board.as_ref().unwrap();
            let id = board.id;
            let index = board
                .slots
                .iter()
                .position(|slot| slot.request.is_some())
                .unwrap();
            let request = board.slots[index].request.clone().unwrap();
            let window = board.window.unwrap();
            let generation = page.generation;
            page.set_visible(false, cx);
            page.set_visible(true, cx);
            assert_eq!(page.generation, generation);
            page.land(
                &request,
                move |page| {
                    page.board
                        .as_mut()
                        .filter(|board| board.id == id)
                        .map(|board| &mut board.slots[index].read)
                },
                Err(QueryError::unsupported("an answer from before hiding")),
                cx,
                move |page, result, cx| page.answered(index, generation, window, result, cx),
            );
            let slot = &page.board.as_ref().unwrap().slots[index];
            assert!(slot.read.is_loading());
            assert!(slot.request.is_some());
            assert!(!slot.failed);
        })
    });
    cx.run_until_parked();
    assert_eq!(ready(cx, &page).len(), slot_count(cx, &page));
}
