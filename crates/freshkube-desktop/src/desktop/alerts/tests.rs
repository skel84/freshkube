//! The session alert read: what it feeds Overview and the rail, that it
//! says Unknown and never "no alerts" for a source it didn't read, and that
//! it stops with its session.
use std::sync::Arc;

use freshkube_core::monitoring::alerts::{Health, State, Unknown, example, example::Shape};
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, test::TestWindowExt};

use crate::desktop::testing::mount;
use crate::desktop::{Area, Page, Pilot};
use crate::presentation::attention::AttentionGroup;
use crate::ui::Tone;
use crate::{DebugOptions, GpuiOptions};

/// The example shell with `FRESHKUBE_ALERTS` set to `shape`.
fn shell(
    cx: &mut TestAppContext,
    shape: Shape,
) -> (tokio::runtime::Runtime, AnyWindowHandle, Entity<Pilot>) {
    let options = GpuiOptions::fixture().with_debug(DebugOptions::parse(|name| {
        (name == "FRESHKUBE_ALERTS").then(|| shape.name().into())
    }));
    let shell = mount(cx, options, 1280., 880.);
    cx.run_until_parked();
    shell
}

/// What the Overview's attention shows of alerts: each row's kind, group and
/// tone.
fn rows(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> Vec<(String, AttentionGroup, Tone)> {
    cx.read(|cx| {
        pilot
            .read(cx)
            .pages
            .node_workspace
            .read(cx)
            .attention
            .rows
            .iter()
            .filter(|row| row.kind == "Alert" || row.kind == "Alerts")
            .map(|row| (row.kind.to_owned(), row.group, row.tone))
            .collect()
    })
}

fn rail(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> Option<Tone> {
    cx.read(|cx| pilot.read(cx).navigation.rail_marks.tone(Area::Monitoring))
}

fn state(cx: &mut TestAppContext, pilot: &Entity<Pilot>) -> Option<Arc<State>> {
    cx.read(|cx| pilot.read(cx).alert_state().cloned())
}

#[gpui_kit::test]
fn critical_and_warning_alerts_reach_the_overview_and_mark_monitoring(cx: &mut TestAppContext) {
    let (_runtime, _handle, pilot) = shell(cx, Shape::Firing);
    let state = state(cx, &pilot).expect("example data read at once");
    assert!(matches!(state.health(), Health::Alerting(_)));
    let rows = rows(cx, &pilot);
    // Two critical and three warning alerts, none of them info or muted.
    assert_eq!(
        rows.iter()
            .filter(|(_, group, _)| *group == AttentionGroup::Failing)
            .count(),
        2,
        "{rows:?}"
    );
    assert_eq!(
        rows.iter()
            .filter(|(_, group, _)| *group == AttentionGroup::Warning)
            .count(),
        3,
        "{rows:?}"
    );
    assert_eq!(
        rail(cx, &pilot),
        Some(Tone::Crit),
        "the worst of them marks the rail"
    );
}

#[gpui_kit::test]
fn an_unread_stale_or_refused_source_is_unknown_never_quiet(cx: &mut TestAppContext) {
    // Nothing active, read now: quiet, and nothing says otherwise.
    let (_runtime, _handle, pilot) = shell(cx, Shape::Quiet);
    assert!(matches!(
        state(cx, &pilot).unwrap().health(),
        Health::Quiet(_)
    ));
    assert!(rows(cx, &pilot).is_empty());
    assert_eq!(rail(cx, &pilot), None);

    // No source found, or only definitions: not a problem, and not quiet.
    for shape in [Shape::NotFound, Shape::RulesOnly] {
        let (_runtime, _handle, pilot) = shell(cx, shape);
        assert!(
            matches!(state(cx, &pilot).unwrap().health(), Health::Unknown(_)),
            "{shape:?}"
        );
        assert!(rows(cx, &pilot).is_empty(), "{shape:?} marks no row");
        assert_eq!(rail(cx, &pilot), None, "{shape:?}");
    }

    // A source that was refused, lacks the label, or was last read before
    // a failure: Unknown, on the rail and as one Unknown row.
    for shape in [Shape::Refused, Shape::NoLabel] {
        let (_runtime, _handle, pilot) = shell(cx, shape);
        assert_eq!(rail(cx, &pilot), Some(Tone::Unknown), "{shape:?}");
        let rows = rows(cx, &pilot);
        assert_eq!(
            rows,
            [("Alerts".to_owned(), AttentionGroup::Unknown, Tone::Unknown)],
            "{shape:?} shows nothing of other clusters' alerts"
        );
    }
    let (_runtime, _handle, pilot) = shell(cx, Shape::Stale);
    assert_eq!(rail(cx, &pilot), Some(Tone::Unknown));
    let rows = rows(cx, &pilot);
    assert!(!rows.is_empty());
    assert!(
        rows.iter().all(|(kind, group, tone)| kind == "Alert"
            && *group == AttentionGroup::Unknown
            && *tone == Tone::Unknown),
        "last-known evidence is never a current problem: {rows:?}"
    );
}

#[gpui_kit::test]
fn the_read_stops_with_its_session_and_nothing_of_the_old_identity_publishes(
    cx: &mut TestAppContext,
) {
    let (_runtime, _handle, pilot) = shell(cx, Shape::Firing);
    assert!(state(cx, &pilot).is_some());
    let identity = cx.read(|cx| {
        pilot
            .read(cx)
            .switch
            .registry
            .active()
            .alerts
            .as_ref()
            .unwrap()
            .identity
            .clone()
    });
    cx.update(|cx| {
        pilot.update(cx, |pilot, _| {
            // The session goes: a context or entry change does this.
            pilot.switch.stop_summary();
        })
    });
    assert!(state(cx, &pilot).is_none(), "nothing of it is kept");
    assert!(
        !cx.read(|cx| pilot.read(cx).switch.registry.alerts_watch.is_running()),
        "and nothing runs"
    );
    // A late answer for the old identity changes nothing.
    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.apply_alerts(
                identity.clone(),
                example::state(Shape::Firing, chrono::Utc::now()),
                cx,
            )
        })
    });
    assert!(state(cx, &pilot).is_none());

    // A new session reads for its own identity, and a stale identity's
    // answer is refused beside it.
    cx.update(|cx| pilot.update(cx, |pilot, cx| pilot.ensure_alerts(cx)));
    assert!(state(cx, &pilot).is_some());
    let mut other = identity;
    other.1 = "another-connection".into();
    cx.update(|cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.apply_alerts(other, State::Unknown(Unknown::NotFound("x".into())), cx)
        })
    });
    assert!(
        matches!(state(cx, &pilot).unwrap().health(), Health::Alerting(_)),
        "another connection's answer never publishes"
    );
}

#[gpui_kit::test]
fn the_column_lists_the_three_pages_first_and_each_opens_in_monitoring(cx: &mut TestAppContext) {
    use crate::monitoring::alerts::Which;
    use crate::monitoring::page::Section;
    let (_runtime, handle, pilot) = shell(cx, Shape::Firing);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.show_area(Area::Monitoring, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    let ids: Vec<String> = cx.read(|cx| {
        pilot
            .read(cx)
            .navigation
            .column_list
            .lines()
            .iter()
            .map(|line| line.id().to_string())
            .take(4)
            .collect()
    });
    assert_eq!(
        ids,
        [
            "nav-monitoring-alerts",
            "nav-monitoring-rules",
            "nav-monitoring-silences",
            "monitoring-builtin"
        ],
        "at the top, the dashboards after"
    );
    for which in Which::ALL {
        cx.update_window(handle, |_, window, cx| {
            window.click(format!("nav-monitoring-{}", which.slug()), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let pilot = pilot.read(cx);
            assert_eq!(pilot.navigation.page, Page::Monitoring);
            assert_eq!(
                pilot.pages.monitoring.read(cx).section(),
                Section::Alerts(which)
            );
            assert!(window.try_find(format!("{}-title", which.slug())).is_some());
            // Only the page that shows reads.
            for other in Which::ALL {
                let shows = pilot
                    .pages
                    .monitoring
                    .read(cx)
                    .alerts_page(other)
                    .read(cx)
                    .is_visible();
                assert_eq!(shows, other == which, "{other:?} while {which:?} shows");
            }
        })
        .unwrap();
    }
    // The Alerts row carries the mark the rail's Monitoring dot does.
    assert!(cx.read(|cx| {
        pilot
            .read(cx)
            .navigation
            .rail_marks
            .row(crate::desktop::shell::RowTarget::Alerts(Which::Alerts))
            .is_some()
    }));
    // A dashboard from the column takes the alert page's place.
    cx.update_window(handle, |_, window, cx| {
        let entry = pilot.read(cx).pages.monitoring.read(cx).catalog().builtins[0]
            .element_id
            .clone();
        window.click(entry, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        let monitoring = pilot.read(cx).pages.monitoring.read(cx);
        assert_eq!(monitoring.section(), Section::Dashboards);
        for which in Which::ALL {
            assert!(!monitoring.alerts_page(which).read(cx).is_visible());
        }
    });
}

#[gpui_kit::test]
fn leaving_monitoring_hides_the_alert_page_and_an_overview_row_opens_it(cx: &mut TestAppContext) {
    use crate::monitoring::alerts::Which;
    use crate::presentation::attention::Destination;
    let (_runtime, handle, pilot) = shell(cx, Shape::Firing);
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| {
            pilot.open_destination(Destination::Alerts, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        let pilot = pilot.read(cx);
        assert_eq!(pilot.navigation.page, Page::Monitoring);
        assert!(
            pilot
                .pages
                .monitoring
                .read(cx)
                .alerts_page(Which::Alerts)
                .read(cx)
                .is_visible()
        );
    });
    cx.update_window(handle, |_, window, cx| {
        pilot.update(cx, |pilot, cx| pilot.navigate(Page::Overview, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        let monitoring = pilot.read(cx).pages.monitoring.read(cx);
        assert!(
            !monitoring.alerts_page(Which::Alerts).read(cx).is_visible(),
            "a hidden page reads nothing"
        );
    });
}

#[gpui_kit::test]
fn a_refresh_keeps_the_last_read_on_show(cx: &mut TestAppContext) {
    let (_runtime, _handle, pilot) = shell(cx, Shape::Firing);
    let before = state(cx, &pilot).unwrap();
    cx.update(|cx| pilot.update(cx, |pilot, cx| pilot.refresh_alerts(cx)));
    let after = state(cx, &pilot).expect("never dropped to nothing");
    assert!(!matches!(*after, State::Reading), "not back to Reading");
    assert_eq!(after.health(), before.health());
    assert_eq!(rows(cx, &pilot).len(), 5, "Overview keeps its rows");
    assert_eq!(rail(cx, &pilot), Some(Tone::Crit));
}
