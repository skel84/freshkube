//! The selected node's or alert's details, in the shared Inspector beside
//! the roster or under it: derived by `sync_detail` when the data or the
//! selection changes, drawn by `render_detail`. Without a selection there
//! is no Inspector.
use super::*;
use freshkube_ui::inspector::Inspector;

/// What the details were derived from: the loader's revision, the Node
/// observation merged into the data, and the selection.
type DetailKey = (u64, Option<(SessionIdentity, u64)>, Item);

/// What the Inspector shows for the selection.
pub(super) struct Detail {
    key: DetailKey,
    /// The heading's accessibility label.
    aria: SharedString,
    shown: Shown,
}

enum Shown {
    Node(NodeDetail),
    Alert(AlertDetail),
    /// The selection is no longer in the roster or the alerts: a node's
    /// name, kept as the heading, and why.
    Gone(Option<String>, &'static str),
}

struct NodeDetail {
    name: String,
    role: String,
    reported: bool,
    /// Whether the target knows the node, so Open node can select it.
    target: bool,
    fields: Vec<(&'static str, Value)>,
}

struct AlertDetail {
    tone: Tone,
    icon: Option<IconName>,
    label: &'static str,
    message: String,
    origin: &'static str,
    evidence: Vec<(String, String)>,
    /// The alert's nodes the target knows, each with Open node.
    targets: Vec<String>,
}

/// One field's value.
enum Value {
    Mono(String),
    Text(String),
    /// Not reported, and why.
    Unknown(String),
    /// A value with a tag before or after it.
    Tagged {
        text: String,
        tag: Option<(Tone, &'static str)>,
        tag_first: bool,
    },
}

impl Value {
    fn read(result: &Result<String, String>) -> Self {
        match result {
            Ok(value) => Self::Mono(value.clone()),
            Err(reason) => Self::Unknown(reason.clone()),
        }
    }

    fn roster(value: Option<bool>, reason: Option<String>, label: &str) -> Self {
        match value {
            Some(true) => Self::Mono("listed".into()),
            Some(false) => Self::Text(format!("Not listed in {label}")),
            None => {
                Self::Unknown(reason.unwrap_or_else(|| format!("{label} wasn't read or is empty")))
            }
        }
    }
}

fn node_detail(row: &NodeRow, view: &LifecycleView, target: bool) -> NodeDetail {
    let config = match (&row.config, row.drift) {
        (Ok(hash), drift) => Value::Tagged {
            text: hash.clone(),
            tag: Some(match drift {
                Drift::Differs => (Tone::Warn, "Differs from other nodes"),
                Drift::InSync => (Tone::Good, "Matches other nodes"),
                _ => (Tone::Outline, "Only reading"),
            }),
            tag_first: false,
        },
        (Err(reason), _) => Value::Unknown(reason.clone()),
    };
    let time = match &row.time {
        Ok(time) => Value::Tagged {
            text: format!("offset {:.3} s · {}", time.offset_seconds, time.server),
            tag: Some(if time.synced {
                (Tone::Good, "Synchronized")
            } else {
                (Tone::Warn, "Not synchronized")
            }),
            tag_first: true,
        },
        Err(reason) => Value::Unknown(reason.clone()),
    };
    let kubelet = match &row.kubelet {
        Ok(version) => Value::Tagged {
            text: version.clone(),
            tag: row
                .kubelet_behind
                .then_some((Tone::Warn, "Behind the newest kubelet")),
            tag_first: false,
        },
        Err(reason) => Value::Unknown(reason.clone()),
    };
    NodeDetail {
        name: row.name.clone(),
        role: row.role_label(),
        reported: row.reported(),
        target,
        fields: vec![
            (
                "Address",
                Value::Mono(row.address.clone().unwrap_or_else(|| "not known".into())),
            ),
            ("Role", Value::Text(row.role_label())),
            ("Talos version", Value::read(&row.talos)),
            ("Platform", Value::read(&row.platform)),
            ("Kubelet version", kubelet),
            ("Machine config", config),
            ("Time sync", time),
            (
                "Talos discovery",
                Value::roster(
                    row.in_discovery,
                    unavailable_reason(&view.snapshot.talos_discovery),
                    "Talos discovery",
                ),
            ),
            (
                "Kubernetes",
                Value::roster(
                    row.in_kubernetes,
                    unavailable_reason(&view.snapshot.kubernetes_roster),
                    "Kubernetes",
                ),
            ),
        ],
    }
}

impl LifecycleScreen {
    /// Derives the details again when the data, the Node observation or
    /// the selection changed; render calls it, so it costs a comparison
    /// on every other frame.
    pub(super) fn sync_detail(&mut self) {
        let (Some(view), Some(selected)) = (self.loader.data(), self.selected.as_ref()) else {
            self.detail = None;
            return;
        };
        let key = (
            self.loader.revision(),
            view.node_observation.clone(),
            selected.clone(),
        );
        if self.detail.as_ref().is_some_and(|detail| detail.key == key) {
            return;
        }
        let shown = match selected {
            Item::Node(name) => match view.display.rows.iter().find(|row| &row.name == name) {
                Some(row) => Shown::Node(node_detail(row, view, self.can_target(name))),
                None => Shown::Gone(Some(name.clone()), "No longer in the roster."),
            },
            Item::Alert(key) => match view.display.alerts.iter().find(|alert| &alert.key == key) {
                Some(alert) => {
                    let (tone, icon, label) = health_tone(&alert.health);
                    Shown::Alert(AlertDetail {
                        tone,
                        icon,
                        label,
                        message: alert.message.clone(),
                        origin: alert.origin,
                        evidence: alert.evidence.clone(),
                        targets: (alert.nodes.iter())
                            .filter(|node| self.can_target(node))
                            .cloned()
                            .collect(),
                    })
                }
                None => Shown::Gone(None, "The selected alert is no longer raised."),
            },
        };
        let aria = match &shown {
            Shown::Node(NodeDetail { name, .. }) | Shown::Gone(Some(name), _) => {
                format!("Details of node {name}").into()
            }
            Shown::Alert(alert) => format!("Details of alert: {}", alert.message).into(),
            Shown::Gone(None, _) => SharedString::default(),
        };
        self.detail = Some(Detail { key, aria, shown });
    }

    /// The Inspector for the selection, or `None` without one.
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let detail = self.detail.as_ref()?;
        let inspector = Inspector::new("lifecycle-detail");
        let inspector = match &detail.shown {
            Shown::Node(node) => self.render_node(inspector, node, detail.aria.clone(), cx),
            Shown::Alert(alert) => self.render_alert(inspector, alert, detail.aria.clone(), cx),
            Shown::Gone(name, why) => {
                let inspector = match name {
                    Some(name) => inspector.heading(
                        div()
                            .id("lifecycle-detail-title")
                            .test_support()
                            .aria_label(detail.aria.clone())
                            .font_family(MONO_FONT)
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(name.clone()),
                    ),
                    None => inspector,
                };
                inspector.child(
                    div()
                        .id("lifecycle-detail-gone")
                        .test_support()
                        .text_size(dp(12.5))
                        .text_color(palette(cx).muted)
                        .child(*why),
                )
            }
        };
        Some(inspector.render(cx).into_any_element())
    }

    fn render_node(
        &self,
        inspector: Inspector,
        node: &NodeDetail,
        aria: SharedString,
        cx: &mut Context<Self>,
    ) -> Inspector {
        let p = palette(cx);
        let title = h_flex()
            .id("lifecycle-detail-title")
            .test_support()
            .aria_label(aria)
            .gap_2()
            .flex_wrap()
            .min_w_0()
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(dp(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(node.name.clone()),
            )
            .child(if node.reported {
                ui::tag(Tone::Outline, None, node.role.clone(), cx)
            } else {
                ui::tag(Tone::Unknown, None, "Not reported", cx)
            })
            .child(div().flex_1())
            .when(node.target, |this| {
                this.child(self.target_button(&node.name, cx))
            });
        let silent = (!node.reported).then(|| {
            div()
                .text_size(dp(12.))
                .text_color(p.muted)
                .child("No Talos answer was received from this node. That doesn't mean it is down; nothing positively reported a failure.")
        });
        let fields: Vec<Div> = (node.fields.iter())
            .map(|(label, value)| field(label, render_value(value, cx), cx))
            .collect();
        inspector
            .heading(title)
            .children(silent)
            .children(fields)
            .child(
                div()
                    .text_size(dp(11.5))
                    .text_color(p.muted)
                    .child("Config is the machineconfig resource version. A difference is a drift indicator, not a diff; nodes may legitimately differ."),
            )
    }

    fn render_alert(
        &self,
        inspector: Inspector,
        alert: &AlertDetail,
        aria: SharedString,
        cx: &mut Context<Self>,
    ) -> Inspector {
        let p = palette(cx);
        let title = v_flex()
            .id("lifecycle-detail-title")
            .test_support()
            .aria_label(aria)
            .gap_2()
            .min_w_0()
            .child(h_flex().child(ui::tag(alert.tone, alert.icon, alert.label, cx)))
            .child(
                div()
                    .text_size(dp(13.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(alert.message.clone()),
            );
        let buttons: Vec<Button> = (alert.targets.iter())
            .map(|node| self.target_button(node, cx))
            .collect();
        let evidence = (!alert.evidence.is_empty()).then(|| {
            v_flex()
                .gap_2p5()
                .child(ui::caption("Evidence", cx))
                .children(alert.evidence.iter().map(|(label, value)| {
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(
                            div()
                                .w(dp(150.))
                                .flex_none()
                                .min_w_0()
                                .font_family(MONO_FONT)
                                .text_size(dp(12.))
                                .truncate()
                                .child(label.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(dp(12.))
                                .child(value.clone()),
                        )
                }))
        });
        inspector
            .heading(title)
            .child(
                div()
                    .text_size(dp(12.))
                    .text_color(p.muted)
                    .child(alert.origin),
            )
            .children(evidence)
            .children((!buttons.is_empty()).then(|| h_flex().gap_2().flex_wrap().children(buttons)))
    }
}

fn render_value(value: &Value, cx: &App) -> AnyElement {
    let p = palette(cx);
    match value {
        Value::Mono(text) => mono(text.clone()).into_any_element(),
        Value::Text(text) => div().child(text.clone()).into_any_element(),
        Value::Unknown(reason) => v_flex()
            .gap_0p5()
            .child(div().text_color(p.unk_ink).child("Not reported"))
            .child(
                div()
                    .text_size(dp(11.5))
                    .text_color(p.muted)
                    .child(reason.clone()),
            )
            .into_any_element(),
        Value::Tagged {
            text,
            tag,
            tag_first,
        } => {
            let tag = tag.map(|(tone, label)| ui::tag(tone, None, label, cx));
            // Within the row's width, so a long value wraps in a narrow
            // Inspector rather than running past its edge.
            let text = mono(text.clone()).max_w_full();
            let row = h_flex().gap_2().flex_wrap();
            if *tag_first {
                row.children(tag).child(text)
            } else {
                row.child(text).children(tag)
            }
            .into_any_element()
        }
    }
}
