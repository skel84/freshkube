use super::*;

impl NetworkScreen {
    fn summary(&self, data: &NetworkData, cx: &App) -> AnyElement {
        let p = palette(cx);
        let snapshot = &data.snapshot;
        let totals: NetworkTotals = snapshot.totals;
        let measured = snapshot
            .interfaces
            .iter()
            .any(|interface| interface.rate.is_some());
        let throughput = if measured {
            format!(
                "RX {}  TX {}",
                rate_text(Some(totals.rx_bytes_per_sec)),
                rate_text(Some(totals.tx_bytes_per_sec))
            )
        } else {
            "measuring…".into()
        };
        let connections = snapshot.connections.as_ref().map_or_else(
            || "unknown".to_owned(),
            |connections| {
                let counts = &connections.counts;
                format!(
                    "{} · {} established · {} listening",
                    counts.total(),
                    counts.established,
                    counts.listen
                )
            },
        );
        let item = |label: &'static str, value: String, color: Option<Hsla>| {
            h_flex()
                .gap_1p5()
                .child(div().text_color(p.muted).child(label))
                .child(mono(value).when_some(color, |this, color| this.text_color(color)))
        };
        h_flex()
            .id("network-summary")
            .test_support()
            .aria_label(format!(
                "Throughput {throughput}; errors {}; dropped {}; connections {connections}",
                totals.errors, totals.dropped
            ))
            .gap_x_5()
            .gap_y_1()
            .flex_wrap()
            .text_size(dp(12.5))
            .child(item("Throughput", throughput, None))
            .child(item(
                "Errors",
                totals.errors.to_string(),
                (totals.errors > 0).then_some(p.crit_ink),
            ))
            .child(item(
                "Dropped",
                totals.dropped.to_string(),
                (totals.dropped > 0).then_some(p.warn_ink),
            ))
            .child(item("Connections", connections, None))
            .into_any_element()
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

    fn tabs(&self, data: &NetworkData, cx: &mut Context<Self>) -> Div {
        let view = self.view;
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
        h_flex().gap_2p5().flex_wrap().child(
            ButtonGroup::new("network-view")
                .outline()
                .small()
                .child(
                    Button::new("network-view-interfaces")
                        .icon(IconName::EthernetPort)
                        .label(format!("Interfaces {}", snapshot.interfaces.len()))
                        .selected(view == View::Interfaces),
                )
                .child(
                    Button::new("network-view-connections")
                        .icon(IconName::ArrowUpDown)
                        .label(format!("Connections {connections}"))
                        .selected(view == View::Connections),
                )
                .child(
                    Button::new("network-view-listeners")
                        .icon(IconName::RadioTower)
                        .label(format!("Listeners {listeners}"))
                        .selected(view == View::Listeners),
                )
                .child(
                    Button::new("network-view-kubespan")
                        .icon(IconName::Waypoints)
                        .label(format!("KubeSpan {peers}"))
                        .selected(view == View::KubeSpan),
                )
                .on_click(cx.listener(|screen, selected: &Vec<usize>, window, cx| {
                    let view = View::from_index(selected.first().copied().unwrap_or(0));
                    screen.switch(view, window, cx);
                })),
        )
    }

    fn connection_toolbar(&self, data: &NetworkData, cx: &mut Context<Self>) -> Div {
        let filter = self.state_filter;
        let counts = data.snapshot.connections.as_ref().map(|c| c.counts.clone());
        let count = |value: Option<usize>| value.map_or("?".to_owned(), |v| v.to_string());
        let counts_for = |f: StateFilter| {
            count(counts.as_ref().map(|c| match f {
                StateFilter::All => c.total(),
                StateFilter::Established => c.established,
                StateFilter::Listen => c.listen,
                StateFilter::TimeWait => c.time_wait,
                StateFilter::CloseWait => c.close_wait,
                StateFilter::SynSent => c.syn_sent,
                StateFilter::Other => c.other,
            }))
        };
        let labelled = |id: &'static str, text: &str, f: StateFilter| {
            Button::new(id)
                .label(format!("{text} {}", counts_for(f)))
                .selected(filter == f)
        };
        h_flex()
            .gap_2p5()
            .flex_wrap()
            .child(
                div().flex_1().min_w(dp(180.)).max_w(dp(320.)).child(
                    Input::new(&self.query)
                        .id("network-filter")
                        .aria_label("Filter connections by address, service, process or state")
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).size(dp(14.))),
                ),
            )
            .when_some(self.iface_filter.clone(), |this, iface| {
                this.child(
                    Button::new("network-clear-interface")
                        .small()
                        .primary()
                        .icon(IconName::X)
                        .label(format!("Interface {iface}"))
                        .on_click(cx.listener(|screen, _, _, cx| {
                            screen.iface_filter = None;
                            cx.notify();
                        })),
                )
            })
            .when(self.view == View::Connections, |this| {
                this.child(
                    ButtonGroup::new("network-state")
                        .outline()
                        .small()
                        .child(labelled("state-all", "All", StateFilter::All))
                        .child(labelled(
                            "state-established",
                            "Est.",
                            StateFilter::Established,
                        ))
                        .child(labelled("state-listen", "Listen", StateFilter::Listen))
                        .child(labelled(
                            "state-time-wait",
                            "TIME_WAIT",
                            StateFilter::TimeWait,
                        ))
                        .child(labelled(
                            "state-close-wait",
                            "CLOSE_WAIT",
                            StateFilter::CloseWait,
                        ))
                        .child(labelled("state-syn-sent", "SYN_SENT", StateFilter::SynSent))
                        .child(labelled("state-other", "Other", StateFilter::Other))
                        .on_click(cx.listener(|screen, selected: &Vec<usize>, _, cx| {
                            let index = selected.first().copied().unwrap_or(0);
                            screen.set_state_filter(StateFilter::from_index(index), cx);
                        })),
                )
            })
    }

    /// The showing view's table, inside a wrapper that holds the page's keys.
    /// The wrapper is drawn in every state, so a view without rows keeps
    /// Tab, the arrows and the filter keys.
    fn list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("network-table")
            .test_support()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
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
            .flex()
            .flex_col()
            .flex_1()
            .min_h(dp(list_min(window)))
            .child(
                DataTable::new()
                    .carded()
                    .render(self, window, cx)
                    .flex_1()
                    .min_h_0(),
            )
    }

    /// List and details, side by side when wide. Short windows scroll the
    /// page rather than squeezing the list.
    fn split(
        &self,
        details_id: &'static str,
        list: impl IntoElement,
        details: Div,
        wide: bool,
        window: &Window,
    ) -> Div {
        let least = list_min(window);
        if wide {
            h_flex()
                .flex_1()
                .min_h(dp(least))
                .items_stretch()
                .gap(dp(14.))
                .child(v_flex().flex_1().min_w_0().min_h_0().child(list))
                .child(
                    div()
                        .id(details_id)
                        .test_support()
                        .aria_label("Details of the selected row")
                        .w(dp(340.))
                        .flex_none()
                        .overflow_y_scroll()
                        .restrict_scroll_to_axis()
                        .child(details),
                )
        } else {
            h_flex()
                .flex_1()
                .min_h(dp(least + 14. + DETAILS_HEIGHT))
                .child(
                    v_flex().size_full().gap(dp(14.)).child(list).child(
                        div()
                            .id(details_id)
                            .test_support()
                            .aria_label("Details of the selected row")
                            .h(dp(DETAILS_HEIGHT))
                            .flex_none()
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .child(details),
                    ),
                )
        }
    }

    // ---- Interfaces ----

    fn interfaces_tab(
        &mut self,
        data: &NetworkData,
        wide: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let list = self.list(window, cx);
        let details = self.interface_details(data, cx);
        self.split("interface-details", list, details, wide, window)
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
        data: &NetworkData,
        wide: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let details_id = if self.view == View::Listeners {
            "listener-details"
        } else {
            "connection-details"
        };
        let toolbar = self.connection_toolbar(data, cx);
        let list = self.list(window, cx);
        let details = self.connection_details(cx);
        v_flex()
            .flex_1()
            .gap(dp(14.))
            .child(toolbar)
            .child(self.split(details_id, list, details, wide, window))
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

    fn kubespan_tab(&mut self, wide: bool, window: &mut Window, cx: &mut Context<Self>) -> Div {
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
            return v_flex().flex_1().child(status(
                "kubespan-status",
                IconName::CircleDashed,
                title.into(),
                detail,
                cx,
            ));
        };
        let peers = match state {
            KubeSpanState::Unavailable(message) => {
                return v_flex().flex_1().child(status(
                    "kubespan-status",
                    IconName::CircleDashed,
                    "KubeSpan status is unknown".into(),
                    format!("It couldn't be read, so nothing is shown as failed. {message}."),
                    cx,
                ));
            }
            KubeSpanState::Disabled => {
                return v_flex().flex_1().child(status(
                    "kubespan-status",
                    IconName::Waypoints,
                    "KubeSpan isn't enabled on this node".into(),
                    "KubeSpan builds encrypted WireGuard tunnels between cluster nodes. Enable it with machine.network.kubespan.enabled: true in the machine configuration.".into(),
                    cx,
                ));
            }
            KubeSpanState::Enabled(peers) if peers.is_empty() => {
                return v_flex().flex_1().child(status(
                    "kubespan-status",
                    IconName::Waypoints,
                    "KubeSpan is enabled, with no peers yet".into(),
                    "Waiting for other nodes to establish KubeSpan connections.".into(),
                    cx,
                ));
            }
            KubeSpanState::Enabled(peers) => peers,
        };
        let peers = peers.clone();
        let up = peers.iter().filter(|peer| peer.state == "up").count();
        let list = self.list(window, cx);
        let selected = self
            .selected_peer_key()
            .and_then(|id| peers.iter().find(|peer| peer.id == id.as_ref()));
        let details = self.peer_details(selected, cx);
        v_flex()
            .flex_1()
            .gap(dp(14.))
            .child(
                div()
                    .id("kubespan-summary")
                    .test_support()
                    .aria_label(format!("{up} of {} peers up", peers.len()))
                    .text_size(dp(12.5))
                    .text_color(if up == peers.len() {
                        p.good_ink
                    } else {
                        p.warn_ink
                    })
                    .child(format!("{up}/{} peers up", peers.len())),
            )
            .child(self.split("peer-details", list, details, wide, window))
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

/// The list's least height: shorter in a short window, which scrolls the
/// page to the details instead.
fn list_min(window: &Window) -> f32 {
    if freshkube_ui::page::is_short(window) {
        freshkube_ui::page::SHORT_LIST_HEIGHT
    } else {
        LIST_MIN_HEIGHT
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

impl Render for NetworkScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::desktop::probe::hit("network");
        if let Some(page) = gated_page_mode(
            "network-page",
            "Network",
            Scope::Node,
            self.source.as_ref(),
            &self.loader,
            "network statistics",
            self.embedded,
            cx,
        ) {
            return page;
        }
        let (Some(source), Some(data)) = (self.source.clone(), self.loader.data().cloned()) else {
            return div().into_any_element();
        };
        let wide = content_width(window) >= SIDE_DETAILS;
        self.sync_rows(cx);
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
        let tab = match self.view {
            View::Interfaces => self.interfaces_tab(&data, wide, window, cx),
            View::Connections | View::Listeners => self.connections_tab(&data, wide, window, cx),
            View::KubeSpan => self.kubespan_tab(wide, window, cx),
        };
        let capture = (self.view == View::Interfaces).then(|| self.capture_panel(&source, cx));
        v_flex()
            .id("network-page")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .px(dp(crate::desktop::PAGE_PADDING))
            .pt(dp(22.))
            .pb(dp(18.))
            .gap(dp(14.))
            .child(crate::screens::header_mode(
                "Network",
                &source,
                Scope::Node,
                &self.loader,
                self.embedded,
                cx,
            ))
            .children(failure_banner(&self.loader, cx))
            .children(partial_notice(missing, cx))
            .child(self.summary(&data, cx))
            .children(self.notices(&data, cx))
            .child(self.tabs(&data, cx))
            .child(tab)
            .children(capture)
            .into_any_element()
    }
}
