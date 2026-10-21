//! Form presentation. The page owns field state, validation and saving;
//! these helpers draw DESIGN.md's groups, rows and platform-ordered actions.

use crate::palette::palette;
use crate::platform::{self, Side};
use crate::ui::{CONTROL_HEIGHT, dp};
use gpui_kit::component::{
    Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputContentType, InputState},
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ClickEvent, ElementId, Entity, FontWeight, IntoElement, RenderOnce, Role,
    SharedString, TestSupportExt, Window, div,
};

pub const MAX_WIDTH: f32 = 640.;
pub const LABEL_WIDTH: f32 = 160.;
pub const NARROW: f32 = 560.;

fn part(id: &SharedString, suffix: &str) -> SharedString {
    format!("{id}-{suffix}").into()
}

/// A group of related fields, with its saving information and actions.
pub fn group(id: impl Into<SharedString>, heading: impl Into<SharedString>) -> Group {
    Group {
        id: id.into(),
        heading: heading.into(),
        dirty: false,
        narrow: false,
        rows: Vec::new(),
        saved: None,
        actions: None,
    }
}

/// The accent dot shared by a changed group and its navigation row.
pub fn dirty_marker(id: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .id(id.into())
        .test_support()
        .aria_label("Unsaved changes")
        .text_color(palette(cx).accent)
        .child("•")
}

pub struct Group {
    id: SharedString,
    heading: SharedString,
    dirty: bool,
    narrow: bool,
    rows: Vec<Row>,
    saved: Option<SavedIn>,
    actions: Option<Actions>,
}

impl Group {
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }

    pub fn narrow(mut self, narrow: bool) -> Self {
        self.narrow = narrow;
        self
    }

    pub fn row(mut self, row: Row) -> Self {
        self.rows.push(row);
        self
    }

    pub fn saved_in(mut self, saved: SavedIn) -> Self {
        self.saved = Some(saved);
        self
    }

    pub fn actions(mut self, actions: Actions) -> Self {
        self.actions = Some(actions);
        self
    }

    pub fn render(self, cx: &App) -> impl IntoElement {
        let p = palette(cx);
        v_flex()
            .id(self.id.clone())
            .test_support()
            .role(Role::Group)
            .aria_label(self.heading.clone())
            .w_full()
            .max_w(dp(MAX_WIDTH))
            .child(
                h_flex()
                    .id(part(&self.id, "heading"))
                    .test_support()
                    .h(dp(26.))
                    .gap(dp(6.))
                    .border_b_1()
                    .border_color(p.line)
                    .text_size(dp(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .child(self.heading)
                    .when(self.dirty, |this| {
                        this.child(dirty_marker(part(&self.id, "dirty"), cx))
                    }),
            )
            .child(
                v_flex()
                    .pt(dp(10.))
                    .gap(dp(10.))
                    .children(self.rows.into_iter().map(|row| row.render(self.narrow, cx))),
            )
            .when(self.saved.is_some() || self.actions.is_some(), |this| {
                this.child(
                    v_flex()
                        .mt(dp(12.))
                        .gap(dp(8.))
                        .when_some(self.saved, |this, saved| {
                            this.child(saved_in(part(&self.id, "saved-in"), &saved, cx))
                        })
                        .children(self.actions),
                )
            })
    }
}

/// A field's label and its control. Rows that call [`Row::error`] reserve
/// a message line; errors replace hints or the empty line.
pub fn row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    control: impl IntoElement,
) -> Row {
    Row {
        id: id.into(),
        label: label.into(),
        control: control.into_any_element(),
        hint: None,
        error: None,
        checked: false,
        on_click: None,
    }
}

type ClickHandler = std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub struct Row {
    id: SharedString,
    label: SharedString,
    control: AnyElement,
    hint: Option<AnyElement>,
    error: Option<SharedString>,
    checked: bool,
    on_click: Option<ClickHandler>,
}

impl Row {
    pub fn hint(mut self, text: impl Into<SharedString>) -> Self {
        self.hint = Some(text.into().into_any_element());
        self
    }

    /// A hint with its own wrapping, such as a paragraph containing a URL.
    pub fn hint_element(mut self, hint: impl IntoElement) -> Self {
        self.hint = Some(hint.into_any_element());
        self
    }

    /// Mark this row as checked, reserving its message line even with `None`.
    pub fn error(mut self, error: Option<impl Into<SharedString>>) -> Self {
        self.checked = true;
        self.error = error.map(Into::into);
        self
    }

    /// Click anywhere on the row, its label included. A click on the control
    /// reaches the control's own handler too.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(std::rc::Rc::new(handler));
        self
    }

    fn render(self, narrow: bool, cx: &App) -> impl IntoElement {
        let p = palette(cx);
        let error = self.error.is_some();
        let on_click = self.on_click.clone();
        div()
            .id(self.id.clone())
            .test_support()
            .when_some(on_click, |this, handler| {
                this.cursor_pointer()
                    .on_click(move |event, window, cx| handler(event, window, cx))
            })
            .flex()
            .when(narrow, |this| this.flex_col())
            .gap(dp(if narrow { 4. } else { 12. }))
            .child(
                h_flex()
                    .id(part(&self.id, "label"))
                    .test_support()
                    .h(dp(CONTROL_HEIGHT))
                    .when(!narrow, |this| this.w(dp(LABEL_WIDTH)).flex_none())
                    .text_size(dp(13.))
                    .text_color(p.ink_2)
                    .child(self.label),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(dp(4.))
                    .child(self.control)
                    .children(
                        self.error
                            .map(IntoElement::into_any_element)
                            .or(self.hint)
                            .map(|text| {
                                div()
                                    .id(part(&self.id, if error { "error" } else { "hint" }))
                                    .test_support()
                                    .when(error, |this| this.role(Role::Alert))
                                    .text_size(dp(12.))
                                    .text_color(if error { p.crit_ink } else { p.muted })
                                    .child(text)
                                    .into_any_element()
                            })
                            // A blank line follows the same font metrics as a message.
                            .or_else(|| {
                                self.checked
                                    .then(|| div().text_size(dp(12.)).child(" ").into_any_element())
                            }),
                    ),
            )
    }
}

/// Give an invalid input a critical border, including while focused.
pub fn invalid(input: Input, cx: &App) -> Input {
    input.border_color(palette(cx).crit).focus_bordered(false)
}

/// Where a group's values are saved; it holds no paths or secret values.
pub struct SavedIn(SharedString);

impl SavedIn {
    pub fn file(name: impl Into<SharedString>) -> Self {
        Self(format!("Saved in {}", name.into()).into())
    }

    pub fn and_credential(self, what: impl Into<SharedString>) -> Self {
        Self(
            format!(
                "{}; {} in {}",
                self.0,
                what.into(),
                platform::credential_store()
            )
            .into(),
        )
    }

    pub fn session() -> Self {
        Self("Not saved; resets at launch".into())
    }
}

pub fn saved_in(id: impl Into<ElementId>, saved: &SavedIn, cx: &App) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .text_size(dp(12.))
        .text_color(palette(cx).muted)
        .child(saved.0.clone())
}

/// A secret is either kept in a store or being replaced in a
/// page-owned input. A kept secret's value is never passed to this helper.
pub enum Secret {
    Kept,
    Typing(Entity<InputState>),
}

#[derive(IntoElement)]
pub struct SecretField {
    id: SharedString,
    secret: Secret,
    store_name: SharedString,
    replace: Option<Button>,
    forget: Option<Button>,
    disabled: bool,
}

pub fn secret(id: impl Into<SharedString>, secret: Secret, _: &App) -> SecretField {
    SecretField {
        id: id.into(),
        secret,
        store_name: platform::credential_store().into(),
        replace: None,
        forget: None,
        disabled: false,
    }
}

impl SecretField {
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Names the secret's store, overriding the platform credential store.
    pub fn store_name(mut self, name: impl Into<SharedString>) -> Self {
        self.store_name = name.into();
        self
    }

    pub fn on_replace(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.replace = Some(Button::new(part(&self.id, "replace")).on_click(handler));
        self
    }

    pub fn on_forget(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.forget = Some(Button::new(part(&self.id, "forget")).on_click(handler));
        self
    }
}

impl RenderOnce for SecretField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        match self.secret {
            Secret::Typing(state) => {
                if !state.read(cx).presentation().is_masked() {
                    state.update(cx, |state, cx| state.set_masked(true, window, cx));
                }
                Input::new(&state)
                    .small()
                    .id(part(&self.id, "input"))
                    .content_type(InputContentType::Password)
                    .disabled(self.disabled)
                    .map(|input| gpui_kit::Styled::h(input, dp(CONTROL_HEIGHT)))
                    .into_any_element()
            }
            Secret::Kept => {
                let label = SharedString::from(format!("Kept in {}", self.store_name));
                h_flex()
                    .gap(dp(8.))
                    .h(dp(CONTROL_HEIGHT))
                    .child(
                        div()
                            .id(part(&self.id, "kept"))
                            .test_support()
                            .aria_label(label.clone())
                            .text_size(dp(12.))
                            .text_color(palette(cx).muted)
                            .child(label),
                    )
                    .child(
                        self.replace
                            .map(|button| button.disabled(self.disabled))
                            .unwrap_or_else(|| {
                                Button::new(part(&self.id, "replace")).disabled(true)
                            })
                            .xsmall()
                            .outline()
                            .label("Replace…"),
                    )
                    .child(
                        self.forget
                            .map(|button| button.disabled(self.disabled))
                            .unwrap_or_else(|| Button::new(part(&self.id, "forget")).disabled(true))
                            .xsmall()
                            .outline()
                            .label("Forget"),
                    )
                    .into_any_element()
            }
        }
    }
}

/// An editable path, or a chosen file whose contents must never be edited.
#[derive(IntoElement)]
pub struct Path {
    id: SharedString,
    input: Input,
    browse: Button,
    chosen_only: bool,
}

pub fn path(
    id: impl Into<SharedString>,
    input: Input,
    on_browse: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Path {
    let id = id.into();
    let browse = Button::new(part(&id, "browse")).on_click(on_browse);
    Path {
        id,
        input,
        browse,
        chosen_only: false,
    }
}

impl Path {
    pub fn chosen_only(mut self) -> Self {
        self.chosen_only = true;
        self
    }

    /// Browse… takes no click, as the field beside it takes no typing.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.browse = self.browse.disabled(disabled);
        self
    }
}

impl RenderOnce for Path {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        h_flex()
            .gap(dp(8.))
            .child(
                div().flex_1().min_w_0().child(
                    self.input
                        .id(part(&self.id, "input"))
                        .small()
                        .map(|input| gpui_kit::Styled::h(input, dp(CONTROL_HEIGHT)))
                        .readonly(self.chosen_only),
                ),
            )
            .child(
                self.browse
                    .small()
                    .outline()
                    .h(dp(CONTROL_HEIGHT))
                    .label("Browse…"),
            )
    }
}

/// One action, retaining the caller's domain id and click handler.
pub struct Action {
    button: Button,
    danger: bool,
}

impl Action {
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            button: Button::new(id).label(label).on_click(on_click),
            danger: false,
        }
    }

    pub fn cancel(id: impl Into<ElementId>) -> Self {
        Self::new(id, "Cancel", |_, window, cx| window.close_dialog(cx))
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.button = self.button.disabled(disabled);
        self
    }

    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    fn render(self, default: bool) -> Button {
        self.button.small().h(dp(CONTROL_HEIGHT)).map(|button| {
            if default && self.danger {
                button.danger()
            } else if default {
                button.primary()
            } else {
                button.outline()
            }
        })
    }
}

/// A trailing-aligned pair in the platform's default-button order.
pub fn actions(default: Action, other: Action) -> Actions {
    Actions {
        default,
        other,
        extra: None,
    }
}

#[derive(IntoElement)]
pub struct Actions {
    default: Action,
    other: Action,
    extra: Option<Action>,
}

impl Actions {
    /// Adds an alternative before the pair on trailing-default platforms.
    pub fn extra(mut self, extra: Action) -> Self {
        self.extra = Some(extra);
        self
    }

    fn render_on(self, side: Side) -> impl IntoElement {
        let default = self.default.render(true);
        let other = self.other.render(false);
        let extra = self.extra.map(|action| action.render(false));
        let pair = match side {
            Side::Leading => [Some(default), extra, Some(other)],
            Side::Trailing => [extra, Some(other), Some(default)],
        }
        .into_iter()
        .flatten();
        h_flex().w_full().justify_end().gap(dp(8.)).children(pair)
    }
}

impl RenderOnce for Actions {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.render_on(platform::default_button())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Platform;
    use gpui_kit::component::{Root, Theme, ThemeMode};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AnyWindowHandle, Context, Pixels, Render, TestAppContext, px, size};

    struct Form {
        input: Entity<InputState>,
        narrow: bool,
        error: bool,
        hint: bool,
        checked: bool,
        side: Option<Side>,
        chosen: bool,
        typing: bool,
        store_name: Option<SharedString>,
        clicks: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl Render for Form {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let input = gpui_kit::Styled::h(
                Input::new(&self.input).small().id("field-input"),
                dp(CONTROL_HEIGHT),
            );
            let input = if self.error {
                invalid(input, cx)
            } else {
                input
            };
            let field = row("field", "Context", input);
            let field = if self.checked {
                field.error(self.error.then_some("Enter a context."))
            } else {
                field
            };
            let field = if self.hint {
                field.hint("Choose a context.")
            } else {
                field
            };
            let pair = actions(
                Action::new("save", "Save", |_, _, _| {}),
                Action::new("revert", "Revert", |_, _, _| {}),
            );
            let pair = match self.side {
                Some(side) => pair.render_on(side).into_any_element(),
                None => pair.into_any_element(),
            };
            v_flex()
                .w(if self.narrow {
                    dp(NARROW - 1.)
                } else {
                    dp(MAX_WIDTH)
                })
                .child(
                    group("group", "Connection")
                        .dirty(true)
                        .narrow(self.narrow)
                        .row(field)
                        .row({
                            let clicks = self.clicks.clone();
                            row("next-field", "Namespace", div().h(dp(CONTROL_HEIGHT)))
                                .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
                        })
                        .saved_in(SavedIn::file("workspace.json"))
                        .render(cx),
                )
                .child(pair)
                .child(
                    path("file", Input::new(&self.input), |_, _, _| {}).map(|path| {
                        if self.chosen {
                            path.chosen_only()
                        } else {
                            path
                        }
                    }),
                )
                .child(
                    secret(
                        "credential",
                        if self.typing {
                            Secret::Typing(self.input.clone())
                        } else {
                            Secret::Kept
                        },
                        cx,
                    )
                    .map(|field| match &self.store_name {
                        Some(name) => field.store_name(name.clone()),
                        None => field,
                    }),
                )
        }
    }

    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Form>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::install(cx);
            crate::text_size::install(None, cx);
            cx.set_reduce_motion(true);
        });
        let mut view = None;
        let handle = cx.open_window(size(px(800.), px(600.)), |window, cx| {
            let form = cx.new(|cx| Form {
                input: cx.new(|cx| InputState::new(window, cx).default_value("cluster-a")),
                narrow: false,
                error: false,
                hint: true,
                checked: true,
                side: None,
                chosen: false,
                typing: false,
                store_name: None,
                clicks: Default::default(),
            });
            view = Some(form.clone());
            Root::new(form, window, cx)
        });
        cx.run_until_parked();
        (handle.into(), view.unwrap())
    }

    fn close(actual: Pixels, expected: f32) {
        assert!(
            (actual - px(expected)).abs() < px(1.),
            "{actual:?} != {expected}"
        );
    }

    #[gpui_kit::test]
    fn a_click_on_the_label_or_the_control_reaches_the_row(cx: &mut TestAppContext) {
        let (handle, form) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("next-field-label", cx);
            assert_eq!(form.read(cx).clicks.get(), 1);
            window.click("next-field", cx);
            assert_eq!(form.read(cx).clicks.get(), 2);
            // A row without a handler has nothing to click.
            window.click("field-label", cx);
            assert_eq!(form.read(cx).clicks.get(), 2);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn labels_align_and_stack_below_the_narrow_breakpoint(cx: &mut TestAppContext) {
        let (handle, form) = open(cx);
        for text in [13., 20.] {
            for narrow in [false, true] {
                cx.update_window(handle, |_, window, cx| {
                    crate::text_size::set(text, cx);
                    form.update(cx, |form, cx| {
                        form.narrow = narrow;
                        cx.notify();
                    });
                    window.render_frame(cx);
                    let label = window.find("field-label").bounds();
                    let input = window.find("field-input").bounds();
                    let scale = text / 13.;
                    if narrow {
                        close(input.left() - label.left(), 0.);
                        close(input.top() - label.bottom(), 4. * scale);
                    } else {
                        close(label.size.width, LABEL_WIDTH * scale);
                        close(input.left() - label.right(), 12. * scale);
                        close(input.center().y - label.center().y, 0.);
                    }
                    close(input.size.height, CONTROL_HEIGHT * scale);
                    assert_eq!(window.find("group").role(), Some(Role::Group));
                    assert_eq!(window.find("group").label(), Some("Connection"));
                    assert_eq!(window.find("group-dirty").label(), Some("Unsaved changes"));
                })
                .unwrap();
            }
        }
    }

    #[gpui_kit::test]
    fn an_error_replaces_the_hint_and_paints_a_critical_border(cx: &mut TestAppContext) {
        let (handle, form) = open(cx);
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            cx.update_window(handle, |_, window, cx| {
                Theme::change(mode, Some(window), cx);
                form.update(cx, |form, cx| {
                    form.error = false;
                    cx.notify();
                });
                window.render_frame(cx);
                let hint = window.find("field-hint").bounds();
                form.update(cx, |form, cx| {
                    form.error = true;
                    cx.notify();
                });
                window.render_frame(cx);
                assert!(window.try_find("field-hint").is_none());
                let error = window.find("field-error");
                assert_eq!(error.role(), Some(Role::Alert));
                assert_eq!(error.bounds().origin, hint.origin);
                for focused in [false, true] {
                    if focused {
                        window.click("field-input", cx);
                    }
                    window.render_frame(cx);
                    let input = window
                        .find("field-input")
                        .bounds()
                        .scale(window.scale_factor());
                    let crit = palette(cx).crit;
                    assert!(
                        window.painted_quads().iter().any(|quad| {
                            quad.border_color == crit
                                && quad.border_widths.top.0 > 0.
                                && quad.bounds == input
                        }),
                        "missing critical input border, focused {focused}"
                    );
                }
            })
            .unwrap();
        }
    }

    #[gpui_kit::test]
    fn an_error_without_a_hint_keeps_the_next_row_in_place(cx: &mut TestAppContext) {
        let (handle, form) = open(cx);
        for text in [13., 20.] {
            for narrow in [false, true] {
                cx.update_window(handle, |_, window, cx| {
                    crate::text_size::set(text, cx);
                    form.update(cx, |form, cx| {
                        form.hint = false;
                        form.error = false;
                        form.narrow = narrow;
                        cx.notify();
                    });
                    window.render_frame(cx);
                    assert!(window.try_find("field-hint").is_none());
                    let next = window.find("next-field").bounds().top();
                    form.update(cx, |form, cx| {
                        form.error = true;
                        cx.notify();
                    });
                    window.render_frame(cx);
                    assert_eq!(window.find("field-error").role(), Some(Role::Alert));
                    assert_eq!(window.find("next-field").bounds().top(), next);
                })
                .unwrap();
            }
        }
    }

    #[gpui_kit::test]
    fn unchecked_rows_without_hints_keep_their_original_height(cx: &mut TestAppContext) {
        let (handle, form) = open(cx);
        for text in [13., 20.] {
            for narrow in [false, true] {
                cx.update_window(handle, |_, window, cx| {
                    crate::text_size::set(text, cx);
                    form.update(cx, |form, cx| {
                        form.checked = false;
                        form.hint = false;
                        form.narrow = narrow;
                        cx.notify();
                    });
                    window.render_frame(cx);
                    let field = window.find("field").bounds();
                    let next = window.find("next-field").bounds();
                    let scale = text / 13.;
                    let height = if narrow {
                        2. * CONTROL_HEIGHT + 4.
                    } else {
                        CONTROL_HEIGHT
                    };
                    close(field.size.height, height * scale);
                    close(next.size.height, height * scale);
                    close(next.top() - field.bottom(), 10. * scale);
                    assert!(window.try_find("field-hint").is_none());
                    assert!(window.try_find("field-error").is_none());
                })
                .unwrap();
            }
        }
    }

    #[gpui_kit::test]
    fn action_order_follows_every_platform_and_the_current_default(cx: &mut TestAppContext) {
        let (handle, form) = open(cx);
        for platform in Platform::ALL {
            cx.update_window(handle, |_, window, cx| {
                form.update(cx, |form, cx| {
                    form.side = Some(platform.default_button());
                    cx.notify();
                });
                window.render_frame(cx);
                let save = window.find("save").bounds();
                let revert = window.find("revert").bounds();
                assert_eq!(
                    save.left() < revert.left(),
                    platform.default_button() == Side::Leading
                );
                close(save.size.height, CONTROL_HEIGHT);
                close(revert.size.height, CONTROL_HEIGHT);
                let gap = if save.left() < revert.left() {
                    revert.left() - save.right()
                } else {
                    save.left() - revert.right()
                };
                close(gap, 8.);
            })
            .unwrap();
        }
        cx.update_window(handle, |_, window, cx| {
            form.update(cx, |form, cx| {
                form.side = None;
                cx.notify();
            });
            window.render_frame(cx);
            assert_eq!(
                window.find("save").bounds().left() < window.find("revert").bounds().left(),
                platform::default_button() == Side::Leading
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_chosen_path_rejects_typing_and_a_secret_is_masked(cx: &mut TestAppContext) {
        let (handle, form) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            form.update(cx, |form, cx| {
                form.chosen = true;
                cx.notify();
            });
            window.render_frame(cx);
            window.click("file-input", cx);
            window.input("changed", cx);
            assert_eq!(form.read(cx).input.read(cx).value(), "cluster-a");
            assert!(window.find("credential-kept").visible());
            assert!(window.try_find("credential-input").is_none());
            form.update(cx, |form, cx| {
                form.typing = true;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(form.read(cx).input.read(cx).presentation().is_masked());
            assert_eq!(
                window.find("credential-input").role(),
                Some(Role::PasswordInput)
            );
            assert!(window.find("credential-input").value().is_none());
            assert!(window.try_find("credential-kept").is_none());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_kept_secret_defaults_to_the_platform_store(cx: &mut TestAppContext) {
        let (handle, _) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("credential-kept").label(),
                Some(format!("Kept in {}", platform::credential_store()).as_str())
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn a_kept_secret_can_name_its_store(cx: &mut TestAppContext) {
        let (handle, form) = open(cx);
        cx.update_window(handle, |_, window, cx| {
            form.update(cx, |form, cx| {
                form.store_name = Some("keys.json".into());
                cx.notify();
            });
            window.render_frame(cx);
            assert_eq!(
                window.find("credential-kept").label(),
                Some("Kept in keys.json")
            );
            assert!(window.try_find("credential-input").is_none());
        })
        .unwrap();
    }

    #[test]
    fn saved_in_names_the_file_store_and_session() {
        assert_eq!(
            &*SavedIn::file("monitoring.json")
                .and_credential("the token")
                .0,
            format!(
                "Saved in monitoring.json; the token in {}",
                platform::credential_store()
            )
        );
        assert_eq!(&*SavedIn::session().0, "Not saved; resets at launch");
    }
}
