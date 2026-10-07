//! The selection's details, in the shared Inspector beside the table or
//! under it: derived by `sync` when the data or the selection changes,
//! drawn by `render_detail`. Without a selection there is no Inspector.
use super::*;
use freshkube_ui::inspector::Inspector;

/// What the Inspector shows for the selected row.
pub(super) struct Detail {
    data: Arc<WorkloadData>,
    key: ItemKey,
    /// `None` when the cluster no longer reports the selected item.
    item: Option<Item>,
}

struct Item {
    name: String,
    health: HealthState,
    tone: Tone,
    fields: Vec<(&'static str, Value)>,
}

/// One field's value.
enum Value {
    Text(String),
    /// A workload's issues, one to a line; none reported when empty.
    Issues(Vec<String>),
    /// Where a pod runs, if scheduled, and whether the node is one of the
    /// target's, which Inspect node opens.
    Node {
        name: Option<String>,
        known: bool,
    },
}

impl Detail {
    fn new(data: &Arc<WorkloadData>, key: &ItemKey, source: Option<&ScreenSource>) -> Self {
        Self {
            data: data.clone(),
            key: key.clone(),
            item: item(&data.snapshot.namespaces, key, source),
        }
    }

    fn current(&self, data: &Arc<WorkloadData>, key: &ItemKey) -> bool {
        Arc::ptr_eq(&self.data, data) && &self.key == key
    }
}

fn item(
    namespaces: &[NamespaceSummary],
    key: &ItemKey,
    source: Option<&ScreenSource>,
) -> Option<Item> {
    let text = |text: String| Value::Text(text);
    match key {
        ItemKey::Namespace(name) => {
            let namespace = namespaces.iter().find(|ns| &ns.name == name)?;
            let failing = namespace
                .problem_pods
                .iter()
                .filter(|pod| pod.issue.severity() == HealthState::Failing)
                .count();
            Some(Item {
                name: namespace.name.clone(),
                health: namespace.health,
                tone: health_tone(namespace.health),
                fields: vec![
                    ("Kind", text("Namespace".into())),
                    (
                        "Workloads",
                        text(format!(
                            "{} of {} healthy",
                            namespace.healthy_workloads, namespace.total_workloads
                        )),
                    ),
                    (
                        "Pods needing attention",
                        text(format!(
                            "{} ({failing} failing)",
                            namespace.problem_pods.len()
                        )),
                    ),
                    (
                        "Issues",
                        text(match issues_in(namespace) {
                            0 => "none".to_owned(),
                            count => pluralize(count, "issue", "issues"),
                        }),
                    ),
                ],
            })
        }
        ItemKey::Workload {
            namespace,
            name,
            kind,
        } => {
            let workload = namespaces
                .iter()
                .find(|ns| &ns.name == namespace)?
                .workloads
                .iter()
                .find(|workload| &workload.name == name && workload.kind == *kind)?;
            Some(Item {
                name: workload.name.clone(),
                health: workload.health,
                tone: health_tone(workload.health),
                fields: vec![
                    ("Namespace", text(workload.namespace.clone())),
                    ("Kind", text(workload.kind.label().into())),
                    (
                        "Ready / desired",
                        text(format!("{} / {}", workload.ready, workload.desired)),
                    ),
                    ("Issues", Value::Issues(workload.issues.clone())),
                ],
            })
        }
        ItemKey::Pod { namespace, name } => {
            let pod = namespaces
                .iter()
                .find(|ns| &ns.name == namespace)?
                .problem_pods
                .iter()
                .find(|pod| &pod.name == name)?;
            let known = pod.node.as_ref().is_some_and(|node| {
                source.is_some_and(|source| source.nodes.iter().any(|n| &n.name == node))
            });
            Some(Item {
                name: pod.name.clone(),
                health: pod.issue.severity(),
                tone: pod_tone(&pod.issue),
                fields: vec![
                    ("Namespace", text(pod.namespace.clone())),
                    ("Kind", text("Pod".into())),
                    (
                        "Node",
                        Value::Node {
                            name: pod.node.clone(),
                            known,
                        },
                    ),
                    ("Phase", text(pod.phase.clone())),
                    ("Issue", text(issue_detail(&pod.issue))),
                    ("Restarts", text(pod.restarts.to_string())),
                    (
                        "Created",
                        text(match pod.created_at {
                            Some(created) => format!(
                                "{} ({} ago)",
                                created.format("%Y-%m-%d %H:%M UTC"),
                                age(Some(created))
                            ),
                            None => "unknown".to_owned(),
                        }),
                    ),
                ],
            })
        }
    }
}

impl WorkloadsScreen {
    /// Derives the details again when the selection or the data changed;
    /// `sync` calls it.
    pub(super) fn sync_detail(&mut self, data: Option<&Arc<WorkloadData>>) {
        let (Some(data), Some(key)) = (data, self.selected.as_ref()) else {
            self.detail = None;
            return;
        };
        if self.detail.as_ref().is_some_and(|d| d.current(data, key)) {
            return;
        }
        self.detail = Some(Detail::new(data, key, self.source.as_ref()));
    }

    /// The Inspector for the selected row, or `None` without a selection.
    pub(super) fn render_detail(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let detail = self.detail.as_ref()?;
        let p = palette(cx);
        let inspector = Inspector::new("workload-detail");
        let Some(item) = &detail.item else {
            return Some(
                inspector
                    .child(
                        div()
                            .id("workload-detail-gone")
                            .test_support()
                            .text_size(dp(12.5))
                            .text_color(p.muted)
                            .child("The selected item is no longer reported by the cluster."),
                    )
                    .render(cx)
                    .into_any_element(),
            );
        };
        let title = h_flex()
            .id("workload-detail-title")
            .test_support()
            .aria_label(format!("{} · {}", item.name, health_label(item.health)))
            .gap_2()
            .flex_wrap()
            .min_w_0()
            .child(
                div()
                    .font_family(MONO_FONT)
                    .text_size(dp(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(item.name.clone()),
            )
            .child(ui::tag(item.tone, None, health_label(item.health), cx));
        let fields: Vec<Div> = item
            .fields
            .iter()
            .map(|(label, value)| field(label, self.render_value(value, cx), cx))
            .collect();
        Some(
            inspector
                .heading(title)
                .children(fields)
                .render(cx)
                .into_any_element(),
        )
    }

    fn render_value(&self, value: &Value, cx: &mut Context<Self>) -> AnyElement {
        let p = palette(cx);
        match value {
            Value::Text(text) => mono(text.clone()).into_any_element(),
            Value::Issues(issues) => v_flex()
                .id("workload-issues")
                .test_support()
                .aria_label(if issues.is_empty() {
                    "none reported".to_owned()
                } else {
                    issues.join("; ")
                })
                .children(if issues.is_empty() {
                    vec![div().text_color(p.muted).child("none reported")]
                } else {
                    issues
                        .iter()
                        .map(|issue| div().child(issue.clone()))
                        .collect()
                })
                .into_any_element(),
            Value::Node { name, known } => h_flex()
                .gap_2()
                .child(match name {
                    Some(node) => mono(node.clone()),
                    None => div().text_color(p.muted).child("not scheduled"),
                })
                .when_some(name.clone().filter(|_| *known), |this, node| {
                    this.child(
                        Button::new("select-node")
                            .link()
                            .small()
                            .label("Inspect node")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(ScreenEvent::SelectNode(node.clone()))
                            })),
                    )
                })
                .into_any_element(),
        }
    }
}
