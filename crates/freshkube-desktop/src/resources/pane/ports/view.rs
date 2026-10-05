//! Drawing the Ports tab. Everything shown was derived when the ports or
//! the forwards changed.

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{PortRow, PortsView};
use crate::forwards::{Row, render_forward};
use crate::palette::palette;
use crate::ui::{self, dp};

impl PortsView {
    fn render_row(&self, row: &PortRow, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        let port = row.port;
        let forward = Button::new(SharedString::from(format!("ports-forward-{port}")))
            .primary()
            .small()
            .icon(IconName::Play)
            .label("Forward")
            .tooltip("Listen on this Mac's loopback and pass connections to this port")
            .disabled(row.blocked.is_some() || self.object.is_none())
            .on_click(cx.listener(move |view, _, _, cx| view.forward_declared(port, cx)));
        v_flex()
            .id(SharedString::from(format!("ports-row-{port}")))
            .test_support()
            .aria_label(row.title.clone())
            .gap_2()
            .py_2()
            .border_b_1()
            .border_color(p.line)
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(row.title.clone()),
                            )
                            .when(!row.detail.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_color(p.muted)
                                        .text_size(dp(11.5))
                                        .child(row.detail.clone()),
                                )
                            }),
                    )
                    .when_some(row.local.clone(), |this, local| {
                        this.child(
                            div()
                                .id(SharedString::from(format!("ports-local-{port}")))
                                .test_support()
                                .aria_label(format!("Local port for {port}"))
                                .w(dp(150.))
                                .flex_none()
                                .child(Input::new(&local).small().disabled(row.blocked.is_some())),
                        )
                    })
                    .child(forward),
            )
            .when_some(row.blocked.clone(), |this, blocked| {
                this.child(div().text_color(p.muted).child(blocked))
            })
            .children(self.render_forwards(&row.forwards, cx))
            .into_any_element()
    }

    fn render_other(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        v_flex()
            .id("ports-other")
            .test_support()
            .aria_label("Other port")
            .gap_2()
            .py_2()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Other port"),
                    )
                    .child(
                        div()
                            .id("ports-other-port")
                            .test_support()
                            .aria_label("Port to forward")
                            .w(dp(90.))
                            .flex_none()
                            .child(Input::new(&self.other_port).small()),
                    )
                    .child(
                        div()
                            .id("ports-other-local")
                            .test_support()
                            .aria_label("Local port")
                            .w(dp(150.))
                            .flex_none()
                            .child(Input::new(&self.other_local).small()),
                    )
                    .child(
                        Button::new("ports-forward-other")
                            .primary()
                            .small()
                            .icon(IconName::Play)
                            .label("Forward")
                            .tooltip("Forward a port the object doesn't declare")
                            .disabled(self.object.is_none())
                            .on_click(cx.listener(|view, _, _, cx| view.forward_other(cx))),
                    ),
            )
            .child(
                div()
                    .text_color(p.muted)
                    .text_size(dp(11.5))
                    .child("Any TCP port the pod serves, declared or not"),
            )
            .children(self.render_forwards(&self.other_forwards, cx))
            .into_any_element()
    }

    fn render_forwards(&self, forwards: &[Row], cx: &mut Context<Self>) -> Vec<AnyElement> {
        forwards
            .iter()
            .map(|row| render_forward("ports-forward-item", &self.list, row, cx))
            .collect()
    }
}

impl Render for PortsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .map(|row| self.render_row(row, cx))
            .collect();
        let empty = match &self.declared {
            Some(declared) if declared.is_empty() => Some("It declares no ports"),
            None => Some("Reading its ports…"),
            _ => None,
        };
        v_flex()
            .id("ports")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .px_3()
            .py_2()
            .text_size(dp(12.5))
            .child(ui::caption("Declared ports", cx))
            .when_some(empty, |this, empty| {
                this.child(
                    div()
                        .id("ports-empty")
                        .test_support()
                        .py_2()
                        .text_color(p.muted)
                        .child(empty),
                )
            })
            .children(rows)
            .child(self.render_other(cx))
            .when_some(self.feedback.clone(), |this, feedback| {
                this.child(
                    div()
                        .id("ports-feedback")
                        .test_support()
                        .role(Role::Status)
                        .text_color(p.warn_ink)
                        .child(feedback),
                )
            })
    }
}
