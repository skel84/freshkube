use super::*;

impl NetworkScreen {
    fn render_header(
        &self,
        data: Option<&NetworkData>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = PageHeader::new(PREFIX, "Network");
        let header = match data {
            Some(data) => {
                let sockets = matches!(self.view, View::Connections | View::Listeners);
                let header = if sockets {
                    header.filter(
                        div().child(
                            Input::new(&self.query)
                                .id("network-filter")
                                .aria_label(
                                    "Filter connections by address, service, process or state",
                                )
                                .small()
                                .h(dp(ui::CONTROL_HEIGHT))
                                .cleanable(true)
                                .prefix(Icon::new(IconName::Search).size(dp(14.))),
                        ),
                    )
                } else {
                    header
                };
                // The rightmost folds first: the states, then the interface,
                // then the views.
                let views = self.view_labels(data);
                let header =
                    header.foldable(self.render_views(&views, cx), self.views_fold(&views, cx));
                let header = match self.iface_filter.clone().filter(|_| sockets) {
                    Some(iface) => header.foldable(
                        self.render_interface(&iface, cx),
                        self.interface_fold(&iface, cx),
                    ),
                    None => header,
                };
                if self.view == View::Connections {
                    let states = state_labels(data);
                    header.foldable(
                        self.render_states(&states, cx),
                        self.states_fold(&states, cx),
                    )
                } else {
                    header
                }
            }
            None => header,
        };
        let refresh = refresh_control(
            header.id("refresh"),
            "Refresh network",
            self.source.as_ref(),
            &self.loader,
            cx,
        );
        let parts = self
            .derived
            .summary
            .as_ref()
            .map(|(_, parts)| parts.clone())
            .unwrap_or_default();
        header
            .control(refresh)
            .meta(meta(
                self.source.as_ref(),
                Scope::Node,
                &self.loader,
                self.embedded,
                parts,
            ))
            .render(window, cx)
    }

    /// Each view's label with its count, by [`View::index`].
    fn view_labels(&self, data: &NetworkData) -> [SharedString; 4] {
        let snapshot = &data.snapshot;
        let connections = snapshot
            .connections
            .as_ref()
            .map_or("?".to_owned(), |c| c.connections.len().to_string());
        let listeners = snapshot
            .connections
            .as_ref()
            .map_or("?".to_owned(), |c| c.listeners.len().to_string());
        let peers = match self.kubespan.data() {
            Some(KubeSpanState::Enabled(peers)) => peers.len().to_string(),
            Some(KubeSpanState::Disabled) => "off".into(),
            Some(KubeSpanState::Unavailable(_)) => "?".into(),
            // Not asked for yet, or on its way: unknown, never "off".
            None if self.kubespan.is_loading() => "…".into(),
            None => "?".into(),
        };
        [
            format!("Interfaces {}", snapshot.interfaces.len()).into(),
            format!("Connections {connections}").into(),
            format!("Listeners {listeners}").into(),
            format!("KubeSpan {peers}").into(),
        ]
    }

    fn render_views(&self, labels: &[SharedString; 4], cx: &mut Context<Self>) -> ButtonGroup {
        let current = self.view;
        ButtonGroup::new("network-view")
            .outline()
            .small()
            .children(View::ALL.iter().map(|&view| {
                Button::new(view.id())
                    .h(dp(ui::CONTROL_HEIGHT))
                    .icon(view.icon())
                    .label(labels[view.index()].clone())
                    .selected(view == current)
            }))
            .on_click(cx.listener(|screen, selected: &Vec<usize>, window, cx| {
                let view = View::from_index(selected.first().copied().unwrap_or(0));
                screen.switch(view, window, cx);
            }))
    }

    /// The views folded: `View · Interfaces 4` over a checked item for each.
    fn views_fold(&self, labels: &[SharedString; 4], cx: &mut Context<Self>) -> page::Fold {
        let current = self.view;
        let items = View::ALL.map(|view| {
            page::checked_item(
                labels[view.index()].clone(),
                view == current,
                page::handler(cx, move |screen: &mut Self, window, cx| {
                    screen.switch(view, window, cx)
                }),
            )
        });
        page::Fold::from(page::submenu_value(
            "View",
            labels[current.index()].clone(),
            all_of(items.into()),
        ))
    }

    fn render_interface(&self, iface: &str, cx: &mut Context<Self>) -> Button {
        Button::new("network-clear-interface")
            .small()
            .primary()
            .h(dp(ui::CONTROL_HEIGHT))
            .icon(IconName::X)
            .label(format!("Interface {iface}"))
            .on_click(cx.listener(|screen, _, _, cx| screen.clear_interface(cx)))
    }

    /// The interface's clear button folded: an item that shows every
    /// connection again.
    fn interface_fold(&self, iface: &str, cx: &mut Context<Self>) -> page::Fold {
        page::Fold::from(page::item(
            "All interfaces",
            page::handler(cx, |screen: &mut Self, _, cx| screen.clear_interface(cx)),
        ))
        .changed(Some(format!("Interface {iface}").into()))
    }

    fn render_states(&self, labels: &[SharedString; 7], cx: &mut Context<Self>) -> ButtonGroup {
        let current = self.state_filter;
        ButtonGroup::new("network-state")
            .outline()
            .small()
            .children(StateFilter::ALL.iter().enumerate().map(|(ix, &filter)| {
                Button::new(filter.id())
                    .h(dp(ui::CONTROL_HEIGHT))
                    .label(labels[ix].clone())
                    .selected(filter == current)
            }))
            .on_click(cx.listener(|screen, selected: &Vec<usize>, _, cx| {
                let index = selected.first().copied().unwrap_or(0);
                screen.set_state_filter(StateFilter::from_index(index), cx);
            }))
    }

    /// The states folded: `State · All 42` over a checked item for each.
    fn states_fold(&self, labels: &[SharedString; 7], cx: &mut Context<Self>) -> page::Fold {
        let current = self.state_filter;
        let items: Vec<page::MenuItems> = StateFilter::ALL
            .iter()
            .zip(labels)
            .map(|(&filter, label)| {
                page::checked_item(
                    label.clone(),
                    filter == current,
                    page::handler(cx, move |screen: &mut Self, _, cx| {
                        screen.set_state_filter(filter, cx)
                    }),
                )
            })
            .collect();
        let ix = StateFilter::ALL
            .iter()
            .position(|&filter| filter == current)
            .unwrap_or(0);
        page::Fold::from(page::submenu_value(
            "State",
            labels[ix].clone(),
            all_of(items),
        ))
        .changed((current != StateFilter::All).then(|| format!("State {}", current.name()).into()))
    }

    /// The TUI's warning line plus the key ports that are listening. Optional
    /// data that is missing adds nothing: unknown is not a warning.
    fn notices(&self, data: &NetworkData, cx: &App) -> Option<AnyElement> {
        let snapshot = &data.snapshot;
        let mut tags: Vec<(Tone, String)> = Vec::new();
        if snapshot.totals.errors > 0 {
            tags.push((
                Tone::Crit,
                format!("{} interface errors", snapshot.totals.errors),
            ));
        }
        if snapshot.totals.dropped > 0 {
            tags.push((Tone::Warn, format!("{} dropped", snapshot.totals.dropped)));
        }
        if let Some(connections) = &snapshot.connections {
            let counts = &connections.counts;
            if counts.time_wait > TIME_WAIT_WARNING {
                tags.push((Tone::Warn, format!("High TIME_WAIT ({})", counts.time_wait)));
            }
            if counts.close_wait > 0 {
                tags.push((Tone::Warn, format!("CLOSE_WAIT ({})", counts.close_wait)));
            }
            if counts.syn_sent > 0 {
                tags.push((Tone::Warn, format!("SYN_SENT ({})", counts.syn_sent)));
            }
            for (name, port) in [
                ("API", 6443),
                ("Etcd", 2379),
                ("Kubelet", 10250),
                ("Scheduler", 10259),
                ("Controller", 10257),
            ] {
                let listening = connections.listeners.iter().any(|l| l.port == port);
                if listening {
                    tags.push((Tone::Good, format!("{name} :{port} listening")));
                }
            }
        }
        if tags.is_empty() {
            return None;
        }
        let label = tags
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        Some(
            h_flex()
                .id("network-notices")
                .test_support()
                .role(Role::Status)
                .aria_label(label)
                .gap_2()
                .flex_wrap()
                .children(
                    tags.into_iter()
                        .map(|(tone, text)| ui::tag(tone, None, text, cx)),
                )
                .into_any_element(),
        )
    }

    /// The showing view's table, edge to edge.
    fn render_table(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("network-table")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(DataTable::new().render(self, window, cx).flex_1().min_h_0())
            .into_any_element()
    }

    /// The table with the selection's details beside it on a wide page and
    /// below it on a narrow one.
    fn split(
        &self,
        details_id: &'static str,
        table: AnyElement,
        details: Div,
        beside: bool,
    ) -> AnyElement {
        let details = div()
            .id(details_id)
            .test_support()
            .aria_label("Details of the selected row")
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .when_else(
                beside,
                |this| this.pr(dp(page::PANE_PADDING)).py(dp(page::PANE_PADDING_Y)),
                |this| this.px(dp(page::PANE_PADDING)).pb(dp(page::PANE_PADDING_Y)),
            )
            .child(details)
            .into_any_element();
        crate::screens::split_fill("network-split", beside, DETAILS_HEIGHT, table, details)
    }

    // ---- Interfaces ----

    fn interfaces_tab(
        &mut self,
        data: &NetworkData,
        beside: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let table = self.render_table(window, cx);
        let details = self.interface_details(data, cx);
        self.split("interface-details", table, details, beside)
    }

    fn interface_details(&self, data: &NetworkData, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let snapshot = &data.snapshot;
        let Some(interface) = self.selected_interface_name().and_then(|name| {
            snapshot
                .interfaces
                .iter()
                .find(|interface| interface.stats.name == name.as_ref())
        }) else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child("Select an interface to see its counters.");
        };
        let stats = &interface.stats;
        let name = stats.name.clone();
        let has_errors = stats.has_errors();
        let rate = |value: Option<String>| value.unwrap_or_else(|| "measuring…".into());
        let side = |label: &'static str,
                    bytes: u64,
                    rate_text: String,
                    packets: u64,
                    errors: u64,
                    dropped: u64| {
            let counter = |value: u64, color: Hsla| {
                div()
                    .text_color(if value > 0 { color } else { p.muted })
                    .child(value.to_string())
            };
            v_flex()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                .child(field("Total", mono(format_bytes(bytes)), cx))
                .child(field("Rate", mono(rate_text), cx))
                .child(field("Packets", mono(group_digits(packets)), cx))
                .child(field("Errors", counter(errors, p.crit_ink), cx))
                .child(field("Dropped", counter(dropped, p.warn_ink), cx))
        };
        let conn_counts = snapshot.connections.as_ref().map(|c| c.counts.clone());
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(name.clone()),
                    )
                    .child(ui::tag(
                        Tone::Outline,
                        None,
                        interface_kind(&name).to_owned(),
                        cx,
                    ))
                    .when(has_errors, |this| {
                        this.child(ui::tag(
                            if stats.total_errors() > 0 {
                                Tone::Crit
                            } else {
                                Tone::Warn
                            },
                            Some(IconName::CircleAlert),
                            "Errors or drops",
                            cx,
                        ))
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("open-interface-connections")
                            .outline()
                            .xsmall()
                            .icon(IconName::ArrowUpDown)
                            .label("Connections")
                            .on_click(cx.listener(|view, _, _, cx| view.open_connections(cx))),
                    ),
            )
            .child(side(
                "Receive",
                stats.rx_bytes,
                rate(interface.receive_rate_display()),
                stats.rx_packets,
                stats.rx_errors,
                stats.rx_dropped,
            ))
            .child(side(
                "Transmit",
                stats.tx_bytes,
                rate(interface.transmit_rate_display()),
                stats.tx_packets,
                stats.tx_errors,
                stats.tx_dropped,
            ))
            .when_some(conn_counts, |this, counts| {
                this.child(field(
                    "Node connections",
                    mono(format!(
                        "{} established · {} listening · {} TIME_WAIT · {} CLOSE_WAIT",
                        counts.established, counts.listen, counts.time_wait, counts.close_wait
                    )),
                    cx,
                ))
            })
            .when(has_errors, |this| {
                this.child(
                    div()
                        .text_size(dp(12.))
                        .text_color(p.warn_ink)
                        .child("Counters are totals since boot. If they keep rising, check the cable, driver or hardware."),
                )
            })
    }

    // ---- Connections and listeners ----

    fn connections_tab(
        &mut self,
        beside: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let details_id = if self.view == View::Listeners {
            "listener-details"
        } else {
            "connection-details"
        };
        let table = self.render_table(window, cx);
        let details = self.connection_details(cx);
        self.split(details_id, table, details, beside)
    }

    fn connection_details(&self, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let Some(conn) = self.selected_connection() else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child("Select a connection to see its details.");
        };
        let info = &conn.connection;
        let key = conn_key(info);
        let copied = self.copied.as_deref() == Some(key.as_str());
        let many = self
            .connections()
            .is_some_and(|c| c.counts.time_wait > TIME_WAIT_WARNING);
        let tone = match info.state {
            ConnectionState::Established => Tone::Good,
            ConnectionState::Listen => Tone::Accent,
            ConnectionState::CloseWait => Tone::Crit,
            ConnectionState::SynSent | ConnectionState::SynRecv => Tone::Warn,
            ConnectionState::TimeWait if many => Tone::Warn,
            _ => Tone::Outline,
        };
        let direction = match conn.direction {
            ConnectionDirection::Inbound => "Inbound (the local port is a known service)",
            ConnectionDirection::Outbound => "Outbound (the remote port is a known service)",
            ConnectionDirection::Unknown => "Unknown",
        };
        let queue = |value: u64| {
            div()
                .text_color(if value > 0 { p.warn_ink } else { p.muted })
                .child(format!("{value} bytes"))
        };
        let with_service = |address: String, service: Option<&'static str>| {
            let text = match service {
                Some(service) => format!("{address} · {service}"),
                None => address,
            };
            mono(text)
        };
        let service_id = service_of(conn).and_then(|(name, _)| talos_service_id(name));
        let service_info = service_id.and_then(|id| self.service(id));
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(local_text(info)),
                    )
                    .child(ui::tag(tone, None, state_label(info.state), cx))
                    .child(div().flex_1())
                    .child(
                        Button::new("copy-connection")
                            .outline()
                            .xsmall()
                            .icon(if copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            })
                            .label(if copied { "Copied" } else { "Copy line" })
                            .on_click(cx.listener(|view, _, _, cx| view.copy_connection(cx))),
                    ),
            )
            .child(field("Protocol", mono(info.protocol.clone()), cx))
            .child(field(
                "Local",
                with_service(local_text(info), conn.local_service),
                cx,
            ))
            .child(field(
                "Remote",
                with_service(remote_text(info), conn.remote_service),
                cx,
            ))
            .child(field("Direction", div().child(direction), cx))
            .child(field("Process", mono(process_text(info)), cx))
            .child(field(
                "Network namespace",
                mono(info.netns.clone().unwrap_or_else(|| "host".into())),
                cx,
            ))
            .child(field(
                "Queues",
                h_flex()
                    .gap_3()
                    .child(h_flex().gap_1().child("RX").child(queue(info.rx_queue)))
                    .child(h_flex().gap_1().child("TX").child(queue(info.tx_queue))),
                cx,
            ))
            .when_some(service_of(conn), |this, (name, _)| {
                let health = service_info
                    .and_then(|service| service.health.as_ref())
                    .map(health_text)
                    .unwrap_or("health unknown");
                let state = service_info.map_or("state unknown", |service| service.state.as_str());
                this.child(field(
                    "Service",
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(mono(name))
                        .child(
                            div()
                                .text_color(p.muted)
                                .child(format!("{state} · {health}")),
                        )
                        .when_some(service_info.map(|s| s.id.clone()), |this, id| {
                            this.child(
                                Button::new("open-service-logs")
                                    .link()
                                    .small()
                                    .label("Logs")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(ScreenEvent::OpenLogs(id.clone()))
                                    })),
                            )
                        }),
                    cx,
                ))
            })
    }

    // ---- KubeSpan ----

    /// KubeSpan's status or its peers' count, for the inset under the
    /// toolbar, and its peers' table when it has any.
    fn kubespan_tab(
        &mut self,
        beside: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, Option<AnyElement>) {
        let p = palette(cx);
        let status = |id: &'static str, icon: IconName, title: String, detail: String, cx: &App| {
            panel(cx)
                .id(id)
                .test_support()
                .role(Role::Status)
                .aria_label(format!("{title}. {detail}"))
                .p_4()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(Icon::new(icon).size(dp(16.)).text_color(p.muted))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title)),
                )
                .child(div().text_size(dp(12.5)).text_color(p.muted).child(detail))
                .into_any_element()
        };
        let Some(state) = self.kubespan.data() else {
            // Nothing is known yet: say so, and never claim KubeSpan is off.
            let (title, detail) = match self.kubespan.error() {
                Some(error) if !self.kubespan.is_loading() => (
                    "KubeSpan status is unknown",
                    format!("It couldn't be read, so nothing is shown as failed. {error}."),
                ),
                _ => (
                    "Loading KubeSpan status",
                    "Asking the node for its KubeSpan configuration and peers.".into(),
                ),
            };
            return (
                status(
                    "kubespan-status",
                    IconName::CircleDashed,
                    title.into(),
                    detail,
                    cx,
                ),
                None,
            );
        };
        let peers = match state {
            KubeSpanState::Unavailable(message) => {
                return (
                    status(
                        "kubespan-status",
                        IconName::CircleDashed,
                        "KubeSpan status is unknown".into(),
                        format!("It couldn't be read, so nothing is shown as failed. {message}."),
                        cx,
                    ),
                    None,
                );
            }
            KubeSpanState::Disabled => {
                return (
                    status(
                        "kubespan-status",
                        IconName::Waypoints,
                        "KubeSpan isn't enabled on this node".into(),
                        "KubeSpan builds encrypted WireGuard tunnels between cluster nodes. Enable it with machine.network.kubespan.enabled: true in the machine configuration.".into(),
                        cx,
                    ),
                    None,
                );
            }
            KubeSpanState::Enabled(peers) if peers.is_empty() => {
                return (
                    status(
                        "kubespan-status",
                        IconName::Waypoints,
                        "KubeSpan is enabled, with no peers yet".into(),
                        "Waiting for other nodes to establish KubeSpan connections.".into(),
                        cx,
                    ),
                    None,
                );
            }
            KubeSpanState::Enabled(peers) => peers,
        };
        let peers = peers.clone();
        let up = peers.iter().filter(|peer| peer.state == "up").count();
        let table = self.render_table(window, cx);
        let selected = self
            .selected_peer_key()
            .and_then(|id| peers.iter().find(|peer| peer.id == id.as_ref()));
        let details = self.peer_details(selected, cx);
        let summary = div()
            .id("kubespan-summary")
            .test_support()
            .aria_label(format!("{up} of {} peers up", peers.len()))
            .text_size(dp(12.5))
            .text_color(if up == peers.len() {
                p.good_ink
            } else {
                p.warn_ink
            })
            .child(format!("{up}/{} peers up", peers.len()))
            .into_any_element();
        (
            summary,
            Some(self.split("peer-details", table, details, beside)),
        )
    }

    fn peer_details(&self, peer: Option<&KubeSpanPeerStatus>, cx: &mut Context<Self>) -> Div {
        let p = palette(cx);
        let Some(peer) = peer else {
            return panel(cx)
                .p_4()
                .text_color(p.muted)
                .text_size(dp(12.5))
                .child("Select a peer to see its details.");
        };
        panel(cx)
            .p_4()
            .gap_2p5()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .font_family(MONO_FONT)
                            .text_size(dp(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(peer.label.clone()),
                    )
                    .child(ui::tag(
                        peer_tone(&peer.state),
                        None,
                        peer.state.clone(),
                        cx,
                    )),
            )
            .child(field("ID", mono(peer.id.clone()), cx))
            .child(field(
                "Endpoint",
                mono(peer.endpoint.clone().unwrap_or_else(|| "--".into())),
                cx,
            ))
            .when_some(peer.rtt_ms, |this, rtt| {
                this.child(field("Round trip", mono(format!("{rtt:.1} ms")), cx))
            })
            .child(field(
                "Last handshake",
                mono(handshake_text(peer.last_handshake.as_deref())),
                cx,
            ))
            .child(field(
                "Transfer",
                mono(format!(
                    "RX {} · TX {}",
                    format_bytes(peer.rx_bytes),
                    format_bytes(peer.tx_bytes)
                )),
                cx,
            ))
    }
}

fn health_text(health: &ServiceHealth) -> &'static str {
    if health.unknown {
        "health unknown"
    } else if health.healthy {
        "healthy"
    } else {
        "unhealthy"
    }
}

/// The menu items of several controls, one after another.
fn all_of(items: Vec<page::MenuItems>) -> page::MenuItems {
    Rc::new(move |menu, window, cx| items.iter().fold(menu, |menu, item| item(menu, window, cx)))
}

/// Each state's label with its count, by [`StateFilter::ALL`].
fn state_labels(data: &NetworkData) -> [SharedString; 7] {
    let counts = data.snapshot.connections.as_ref().map(|c| &c.counts);
    StateFilter::ALL.map(|filter| {
        let count = counts.map_or("?".to_owned(), |c| {
            match filter {
                StateFilter::All => c.total(),
                StateFilter::Established => c.established,
                StateFilter::Listen => c.listen,
                StateFilter::TimeWait => c.time_wait,
                StateFilter::CloseWait => c.close_wait,
                StateFilter::SynSent => c.syn_sent,
                StateFilter::Other => c.other,
            }
            .to_string()
        });
        format!("{} {count}", filter.name()).into()
    })
}

impl Render for NetworkScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("network");
        self.sync_rows(cx);
        let data = self.loader.data().cloned();
        let header = self.render_header(data.as_deref(), window, cx);
        let state = gate(
            self.source.as_ref(),
            &self.loader,
            Scope::Node,
            "network statistics",
            cx,
        );
        // The table runs edge to edge under the toolbar; the banners, the
        // notices and KubeSpan's status sit in an inset between them, and
        // packet capture in one after it. A short page scrolls its frame,
        // so the list keeps some rows.
        let page = page::page("network-page")
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .child(page::toolbar(cx).child(header));
        let page = match (state, data, self.source.clone()) {
            (Some(state), _, _) => page.child(
                page::inset()
                    .id("network-state")
                    .test_support()
                    .child(state),
            ),
            (None, Some(data), Some(source)) => self.render_body(page, &data, &source, window, cx),
            _ => page,
        };
        // The keys live on a wrapper drawn in every state, so Tab, the
        // arrows and the filter keys work while a view shows no rows.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .on_action(cx.listener(|view, _: &NextRow, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &PreviousRow, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &FirstRow, _, cx| view.step(isize::MIN, cx)))
            .on_action(cx.listener(|view, _: &LastRow, _, cx| view.step(isize::MAX, cx)))
            .on_action(cx.listener(|view, _: &NextPage, _, cx| view.step(PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &PreviousPage, _, cx| view.step(-PAGE_ROWS, cx)))
            .on_action(cx.listener(|view, _: &NextView, window, cx| {
                view.switch(view.view.shifted(1), window, cx)
            }))
            .on_action(cx.listener(|view, _: &PreviousView, window, cx| {
                view.switch(view.view.shifted(-1), window, cx)
            }))
            .on_action(cx.listener(|view, _: &OpenConnections, _, cx| view.open_connections(cx)))
            .on_action(cx.listener(|view, _: &SortPrimary, _, cx| {
                let sort = match view.view {
                    View::Interfaces => Sort::Traffic,
                    _ => Sort::State,
                };
                view.set_sort(sort, cx);
            }))
            .on_action(cx.listener(|view, _: &SortSecondary, _, cx| {
                let sort = match view.view {
                    View::Interfaces => Sort::Errors,
                    _ => Sort::Port,
                };
                view.set_sort(sort, cx);
            }))
            .on_action(cx.listener(|view, _: &FocusFilter, window, cx| {
                if matches!(view.view, View::Connections | View::Listeners) {
                    let focus = view.query.read(cx).focus_handle(cx);
                    window.focus(&focus, cx);
                }
            }))
            .on_action(
                cx.listener(|view, _: &ClearFilter, window, cx| view.clear_filter(window, cx)),
            )
            .on_action(cx.listener(|view, _: &CopyConnection, _, cx| view.copy_connection(cx)))
            .child(page)
    }
}

impl NetworkScreen {
    /// The page under the toolbar once a sample is in: the insets, the
    /// showing view's table and details, and capture on Interfaces.
    fn render_body(
        &mut self,
        page: Observed<Stateful<Div>>,
        data: &NetworkData,
        source: &ScreenSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Observed<Stateful<Div>> {
        let mut missing: Vec<String> = data
            .snapshot
            .unavailable
            .iter()
            .map(|InspectionUnavailable { source, message }| {
                format!("{}: {message}", source.label())
            })
            .collect();
        if let Some(KubeSpanState::Unavailable(message)) = self.kubespan.data() {
            missing.push(format!("{}: {message}", InspectionSource::KubeSpan.label()));
        }
        let mut insets: Vec<AnyElement> = failure_banner(&self.loader, cx)
            .map(IntoElement::into_any_element)
            .into_iter()
            .chain(partial_notice(missing, cx))
            .chain(self.notices(data, cx))
            .collect();
        let beside = crate::screens::beside(window, self.embedded);
        let split = match self.view {
            View::Interfaces => Some(self.interfaces_tab(data, beside, window, cx)),
            View::Connections | View::Listeners => Some(self.connections_tab(beside, window, cx)),
            View::KubeSpan => {
                let (status, split) = self.kubespan_tab(beside, window, cx);
                insets.push(status);
                split
            }
        };
        let capture = (self.view == View::Interfaces).then(|| self.capture_panel(source, cx));
        page.when(!insets.is_empty(), |page| {
            page.child(
                page::inset()
                    .flex()
                    .flex_col()
                    .gap(dp(page::PANE_PADDING_Y))
                    .children(insets),
            )
        })
        .children(split)
        .children(capture.map(|capture| page::inset().child(capture)))
    }
}
