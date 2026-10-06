//! The application's map, as Coroot draws it above its reports: the
//! clients on the left, the application and its instances in the middle,
//! its dependencies on the right, each link coloured by its status. It
//! takes plain nodes and links, so a shared graph can draw it later.
use super::super::*;
use freshkube_core::coroot as api;

// The strip's grid, in dp at the default text size.
const NODE_W: f32 = 200.;
const NODE_H: f32 = 40.;
const GAP_Y: f32 = 10.;
const GAP_X: f32 = 120.;
const ARROW: f32 = 7.;
/// The application box's head, above its instances.
const APP_HEAD: f32 = 40.;
const INSTANCE_H: f32 = 22.;
const APP_PAD: f32 = 8.;
/// Instances beyond this show as a count.
const MOST_INSTANCES: usize = 8;
/// Clients or dependencies beyond this show as a count, as Coroot does.
const MOST_NODES: usize = 12;

/// A link's ends in dp, from the caller to the callee, and its status.
type Link = ((f32, f32), (f32, f32), Status);

/// A client or dependency: its box and its link to the application.
pub(super) struct StripNode {
    pub element_id: SharedString,
    pub app: api::AppId,
    pub label: SharedString,
    pub detail: SharedString,
    pub status: Status,
    pub link: Status,
    pub tooltip: SharedString,
    x: f32,
    y: f32,
}

#[derive(Default)]
pub(super) struct Strip {
    clients: Vec<StripNode>,
    dependencies: Vec<StripNode>,
    label: SharedString,
    detail: SharedString,
    status: Option<Status>,
    instances: Vec<SharedString>,
    /// "+3 more", when instances were left out.
    more_instances: Option<SharedString>,
    /// "+3 more" under each side, when nodes were left out.
    more: [Option<SharedString>; 2],
    /// Each link's ends in dp and its status: clients point at the app,
    /// the app at its dependencies.
    links: Rc<Vec<Link>>,
    app_y: f32,
    app_h: f32,
    height: f32,
}

fn node(app: &api::MapApp, x: f32, y: f32) -> StripNode {
    let link = app
        .link
        .as_ref()
        .map_or(Status::Unknown, |l| Status::from(l.status));
    let mut tooltip = format!(
        "{} · {}",
        app_label(&app.id),
        Status::from(app.status).label()
    );
    if let Some(l) = &app.link {
        if !l.reason.is_empty() {
            tooltip.push_str(&format!("\nLink: {}", l.reason));
        }
        if !l.stats.is_empty() {
            tooltip.push_str(&format!("\n{}", l.stats.join(" · ")));
        }
    }
    StripNode {
        element_id: format!("obs-app-map-{}", app.id.as_str()).into(),
        app: app.id.clone(),
        label: app.id.name().to_string().into(),
        detail: app.id.namespace().unwrap_or_default().to_string().into(),
        status: app.status.into(),
        link,
        tooltip: tooltip.into(),
        x,
        y,
    }
}

/// A column's boxes, centred on `middle`.
fn column(apps: &[api::MapApp], x: f32, middle: f32) -> Vec<StripNode> {
    let shown = apps.len().min(MOST_NODES);
    let height = shown as f32 * (NODE_H + GAP_Y) - GAP_Y;
    let top = middle - height / 2.;
    apps.iter()
        .take(shown)
        .enumerate()
        .map(|(ix, app)| node(app, x, top + ix as f32 * (NODE_H + GAP_Y)))
        .collect()
}

fn more(total: usize, shown: usize) -> Option<SharedString> {
    (total > shown).then(|| format!("+{} more", total - shown).into())
}

impl Strip {
    pub(super) fn new(map: &api::AppMap) -> Self {
        let shown = map.instances.len().min(MOST_INSTANCES);
        let app_h = APP_HEAD
            + shown as f32 * INSTANCE_H
            + APP_PAD
            + if map.instances.len() > shown {
                INSTANCE_H
            } else {
                0.
            };
        let side = |n: usize| {
            let n = n.min(MOST_NODES);
            n as f32 * (NODE_H + GAP_Y) - GAP_Y + if n > 0 { INSTANCE_H } else { 0. }
        };
        let height = app_h
            .max(side(map.clients.len()))
            .max(side(map.dependencies.len()));
        let middle = height / 2.;
        let app_x = NODE_W + GAP_X;
        let clients = column(&map.clients, 0., middle);
        let dependencies = column(&map.dependencies, 2. * app_x, middle);
        let links = clients
            .iter()
            .map(|c| ((c.x + NODE_W, c.y + NODE_H / 2.), (app_x, middle), c.link))
            .chain(
                dependencies
                    .iter()
                    .map(|d| ((app_x + NODE_W, middle), (d.x, d.y + NODE_H / 2.), d.link)),
            )
            .collect();
        let app = &map.app;
        Self {
            more: [
                more(map.clients.len(), clients.len()),
                more(map.dependencies.len(), dependencies.len()),
            ],
            clients,
            dependencies,
            label: app.id.name().to_string().into(),
            detail: app.id.namespace().unwrap_or_default().to_string().into(),
            status: Some(app.status.into()),
            instances: map
                .instances
                .iter()
                .take(shown)
                .map(|i| i.id.clone().into())
                .collect(),
            more_instances: more(map.instances.len(), shown),
            links: Rc::new(links),
            app_y: middle - app_h / 2.,
            app_h,
            height,
        }
    }
}

impl ObservabilityPage {
    pub(super) fn render_strip(&self, strip: &Strip, cx: &Context<Self>) -> AnyElement {
        let p = palette(cx);
        let links = strip.links.clone();
        let lines = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let unit = ui::dp_px(1., window);
                let at =
                    |(x, y): (f32, f32)| point(bounds.left() + unit * x, bounds.top() + unit * y);
                for &(a, d, status) in links.iter() {
                    let color = match status {
                        Status::Critical => p.crit,
                        Status::Warning => p.warn,
                        Status::Ok => p.good,
                        _ => p.line_strong,
                    };
                    let end = (d.0 - ARROW, d.1);
                    let pull = ((end.0 - a.0).abs() / 2.).max(32.);
                    let mut path = PathBuilder::stroke(unit * 1.5);
                    if !matches!(status, Status::Ok) {
                        let dash = unit * 5.;
                        path = path.dash_array(&[dash, dash * 0.6]);
                    }
                    path.move_to(at(a));
                    path.cubic_bezier_to(at(end), at((a.0 + pull, a.1)), at((end.0 - pull, end.1)));
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                    let mut head = PathBuilder::fill();
                    head.move_to(at(d));
                    head.line_to(at((end.0, end.1 - ARROW * 0.6)));
                    head.line_to(at((end.0, end.1 + ARROW * 0.6)));
                    head.close();
                    if let Ok(head) = head.build() {
                        window.paint_path(head, color);
                    }
                }
            },
        )
        .size_full();
        let app_x = NODE_W + GAP_X;
        let width = 3. * NODE_W + 2. * GAP_X;
        let side_note = |text: &Option<SharedString>, x: f32| {
            text.clone().map(|text| {
                muted(text, cx)
                    .absolute()
                    .left(dp(x))
                    .bottom_0()
                    .w(dp(NODE_W))
                    .text_center()
            })
        };
        let mut instances = v_flex().px(dp(10.)).gap(dp(0.)).children(
            strip
                .instances
                .iter()
                .map(|name| mono(name.clone()).h(dp(INSTANCE_H)).truncate()),
        );
        if let Some(more) = &strip.more_instances {
            instances = instances.child(muted(more.clone(), cx).h(dp(INSTANCE_H)));
        }
        let app = v_flex()
            .id("obs-app-map-app")
            .test_support()
            .absolute()
            .left(dp(app_x))
            .top(dp(strip.app_y))
            .w(dp(NODE_W))
            .h(dp(strip.app_h))
            .rounded(px(8.))
            .border_1()
            .border_color(p.accent_line)
            .bg(p.surface_2)
            .child(
                line()
                    .h(dp(APP_HEAD))
                    .px(dp(10.))
                    .children(strip.status.map(|s| status(s, cx)))
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .child(
                                mono(strip.label.clone())
                                    .truncate()
                                    .font_weight(ui::HEADING_WEIGHT),
                            )
                            .child(muted(strip.detail.clone(), cx).truncate()),
                    ),
            )
            .child(instances);
        div()
            .id("obs-app-map-scroll")
            .test_support()
            .w_full()
            .overflow_x_scroll()
            .child(
                div()
                    .id("obs-app-map")
                    .test_support()
                    .relative()
                    .flex_none()
                    .w(dp(width))
                    .h(dp(strip.height))
                    .child(lines)
                    .children(strip.clients.iter().map(|n| self.strip_node(n, cx)))
                    .children(strip.dependencies.iter().map(|n| self.strip_node(n, cx)))
                    .child(app)
                    .children(side_note(&strip.more[0], 0.))
                    .children(side_note(&strip.more[1], 2. * app_x)),
            )
            .into_any_element()
    }

    fn strip_node(&self, node: &StripNode, cx: &Context<Self>) -> Button {
        let p = palette(cx);
        let app = node.app.clone();
        Button::new(node.element_id.clone())
            .outline()
            .group("fog-control")
            .absolute()
            .left(dp(node.x))
            .top(dp(node.y))
            .w(dp(NODE_W))
            .h(dp(NODE_H))
            .px(dp(10.))
            .gap(dp(8.))
            .bg(p.surface_2)
            .border_color(p.line_strong)
            .justify_start()
            .child(status(node.status, cx))
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .items_start()
                    .child(mono(node.label.clone()).w_full().truncate())
                    .child(muted(node.detail.clone(), cx).w_full().truncate()),
            )
            .tooltip(node.tooltip.clone())
            .on_click(cx.listener(move |this, _, _, cx| this.open_linked_app(app.clone(), cx)))
    }
}
