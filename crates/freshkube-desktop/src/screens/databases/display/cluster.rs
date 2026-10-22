//! The Clusters view: one row per database, grouped by whether it needs a look.
use super::format::{age, cron, health_word, seconds, span, tone_of};
use super::*;
use freshkube_core::HealthIndicator;
use freshkube_core::databases::coroot::{CorootState, DatabaseReport};
use freshkube_core::databases::{
    self, Archiving, CertificateState, Database, Engine, Freshness, LAG_WARNING_SECONDS, Role,
    Source, Switchover,
};

pub(super) const NEEDS_A_LOOK: usize = 0;
const HEALTHY: usize = 1;

pub(super) fn group_line(group: usize) -> GroupLine {
    if group == NEEDS_A_LOOK {
        GroupLine {
            slug: "needs-a-look",
            label: "Needs a look",
            tone: ui::Tone::Warn,
            count: 0,
        }
    } else {
        GroupLine {
            slug: "healthy",
            label: "Healthy",
            tone: ui::Tone::Good,
            count: 0,
        }
    }
}

pub(super) fn rows(inv: &Inventory, evidence: &Evidence, now: chrono::DateTime<Utc>) -> Vec<Row> {
    inv.databases
        .items
        .iter()
        .map(|db| row(db, inv, evidence.state(&db.identity.uid), now))
        .collect()
}

fn source_word(source: Source) -> &'static str {
    source.label()
}

fn row(
    db: &Database,
    inv: &Inventory,
    coroot: Option<&CorootState>,
    now: chrono::DateTime<Utc>,
) -> Row {
    let not_read = CorootState::NotRead;
    let coroot = coroot.unwrap_or(&not_read);
    let report = coroot.report();
    let verdict = databases::verdict(db, inv, coroot, now);
    // A cluster list kept after a failed refresh is not evidence of health.
    let mut state = verdict.state;
    if inv.databases.stale || inv.retained.contains(&db.engine) {
        state.health = HealthIndicator::Unknown;
    }
    let found = databases::coverage(db, inv, inv.backups.observed_at);
    let archiving = databases::archiving(db, report);
    let window = databases::restore_window(db, inv, &found, archiving, now);
    let (key, id) = identity_key(&db.identity);

    let lag = lag_cell(db, report);
    let backup = match (found.freshness, found.completed.and_then(|b| b.completed)) {
        (Freshness::None, _) => ("None".to_owned(), None),
        (Freshness::Unknown, _) => ("Unknown".to_owned(), None),
        (_, Some(at)) => (age(now, at), Some(time(Some(at)))),
        (_, None) => ("Unknown".to_owned(), None),
    };
    let chip = match state.health {
        HealthIndicator::Unknown => None,
        _ => Some(source_word(state.source)),
    };
    let engine = db.engine_label();
    let sections = vec![
        instances(db, report, now),
        backups(db, inv, &found, archiving, window),
        proxy(db),
        certificates(db, now),
        from_coroot(db, coroot),
        conditions(db),
        sources(db, inv, coroot),
    ];
    let group = if matches!(
        state.health,
        HealthIndicator::Healthy | HealthIndicator::Info
    ) {
        HEALTHY
    } else {
        NEEDS_A_LOOK
    };
    let mut search = vec![
        db.identity.name.clone(),
        db.identity.namespace.clone(),
        engine.clone(),
        db.engine.label().to_owned(),
        health_word(state.health).to_owned(),
        verdict.headline.clone(),
    ];
    search.extend(db.current_primary.known().cloned());
    search.extend(db.target_primary.known().cloned());
    search.extend(db.instances.iter().map(|i| i.name.clone()));
    let observe = report.map(|report| Observe {
        app: report.app.clone(),
        tab: match db.engine {
            Engine::CloudNativePg => freshkube_nav::ObservabilityTab::Postgres,
            Engine::MariaDb => freshkube_nav::ObservabilityTab::Mysql,
        },
    });
    let why = verdict
        .problems()
        .map(|finding| {
            let by = if finding.source == databases::Source::Coroot {
                " · Coroot"
            } else {
                ""
            };
            SharedString::from(format!("{}{by}", finding.why))
        })
        .collect();
    let mut next: Vec<SharedString> = Vec::new();
    for finding in verdict.problems() {
        if let Some(step) = &finding.next {
            let step = SharedString::from(step.clone());
            if !next.contains(&step) {
                next.push(step);
            }
        }
    }
    Row {
        key,
        id,
        label: format!("{} · {engine}", db.identity.name).into(),
        identity: db.identity.clone(),
        cells: vec![
            db.identity.name.clone().into(),
            engine.clone().into(),
            lag.0.into(),
            backup.0.into(),
            health_word(state.health).into(),
        ],
        tips: vec![
            None,
            Some(db.engine.label().into()),
            lag.1.map(Into::into),
            backup.1.map(Into::into),
            Some(verdict.headline.clone().into()),
        ],
        tone: tone_of(state.health),
        chip,
        group: Some(group),
        search: search.join(" ").to_lowercase(),
        engine: engine.into(),
        verdict: Verdict {
            tone: tone_of(state.health),
            headline: verdict.headline.into(),
            source: chip,
            why,
            next,
        },
        sections,
        observe,
    }
    .finish()
}

/// The worst lag among the replicas, with whose it is and who measured it.
fn lag_cell(db: &Database, report: Option<&DatabaseReport>) -> (String, Option<String>) {
    let replicas: Vec<_> = db
        .instances
        .iter()
        .filter(|i| i.role == Fact::Known(Role::Replica))
        .collect();
    if replicas.is_empty() {
        return if db.instances.is_empty() {
            ("Unknown".into(), None)
        } else {
            ("—".into(), Some("No replicas".into()))
        };
    }
    let worst = replicas
        .iter()
        .filter_map(|i| {
            databases::lag_of(i, report)
                .known()
                .map(|lag| (i.name.as_str(), *lag))
        })
        .max_by(|a, b| a.1.seconds.total_cmp(&b.1.seconds));
    match worst {
        Some((name, lag)) => (
            seconds(lag.seconds),
            Some(format!(
                "{name} · from {}{}",
                lag.source.label(),
                if lag.seconds > LAG_WARNING_SECONDS {
                    " · above 30 s"
                } else {
                    ""
                }
            )),
        ),
        None => (
            "Unknown".into(),
            Some("Neither the operator nor Coroot reports it; Prometheus is not read".into()),
        ),
    }
}

fn instances(
    db: &Database,
    report: Option<&DatabaseReport>,
    now: chrono::DateTime<Utc>,
) -> Section {
    let mut fields = vec![
        Field::mono("Namespace", db.identity.namespace.clone()),
        Field::fact("Primary", &db.current_primary),
    ];
    if db.target_primary != db.current_primary {
        fields.push(Field::fact("Requested primary", &db.target_primary));
    }
    match databases::switchover(db, now) {
        Switchover::Stuck(age) => fields.push(
            Field::prose(
                "Switchover",
                format!("Stuck: requested for {} min", age.num_minutes()),
            )
            .tone(ui::Tone::Crit),
        ),
        Switchover::InProgress(age) => fields.push(Field::prose(
            "Switchover",
            format!("In progress for {} min", age.num_minutes()),
        )),
        Switchover::Unknown => fields.push(Field::prose(
            "Switchover",
            "Unknown: the requested primary or when it was asked is not reported",
        )),
        Switchover::Settled | Switchover::NotApplicable => {}
    }
    fields.push(Field::prose(
        "Instances ready",
        match (&db.ready_instances, &db.desired_instances) {
            (Fact::Known(ready), Fact::Known(desired)) => format!("{ready} of {desired}"),
            (Fact::Known(ready), Fact::Unknown) => format!("{ready} (wanted not reported)"),
            (Fact::Unknown, Fact::Known(desired)) => {
                format!("Not reported by the operator ({desired} wanted)")
            }
            (Fact::Unknown, Fact::Unknown) => "Not reported".to_owned(),
        },
    ));
    for instance in &db.instances {
        let mut parts = vec![match instance.role {
            Fact::Known(Role::Primary) => "Primary".to_owned(),
            Fact::Known(Role::Replica) => "Replica".to_owned(),
            Fact::Unknown => "Role unknown".to_owned(),
        }];
        parts.push(match instance.ready {
            Fact::Known(true) => "ready".into(),
            Fact::Known(false) => "not ready".into(),
            Fact::Unknown => "readiness not reported".into(),
        });
        if instance.role != Fact::Known(Role::Primary) {
            parts.push(match databases::lag_of(instance, report) {
                Fact::Known(lag) => {
                    format!("lag {} ({})", seconds(lag.seconds), lag.source.label())
                }
                Fact::Unknown => "lag Unknown".into(),
            });
        }
        if let Fact::Known(replication) = &instance.replication {
            parts.push(replication.clone());
        }
        if let Fact::Known(node) = &instance.node {
            parts.push(format!("node {node}"));
        }
        let tone = match databases::lag_of(instance, report) {
            Fact::Known(lag) if lag.seconds > LAG_WARNING_SECONDS => Some(ui::Tone::Warn),
            _ => None,
        };
        let mut field = Field::prose(&instance.name, parts.join(" · "));
        field.tone = tone;
        fields.push(field);
    }
    if db.instances.iter().any(|i| i.node == Fact::Unknown) {
        fields.push(Field::prose(
            "Node",
            "Not reported by the operator's status; a node needs a Pods read",
        ));
    }
    Section::new("instances", "Instances", fields)
}

fn backups(
    db: &Database,
    inv: &Inventory,
    found: &databases::BackupCoverage,
    archiving: Archiving,
    window: Option<databases::RestoreWindow>,
) -> Section {
    let mut fields = vec![
        Field::prose(
            "Last good backup",
            found
                .completed
                .map(|b| format!("{} · {}", b.identity.name, time(b.completed)))
                .unwrap_or_else(|| "Unknown / none confirmed".into()),
        ),
        Field::prose("Freshness", found.freshness.label()),
        Field::prose(
            "Newest attempt",
            found
                .newest
                .map(|b| format!("{} · {}", b.identity.name, b.state.label()))
                .unwrap_or_else(|| {
                    if inv.backups.complete() {
                        "None confirmed for this incarnation".into()
                    } else {
                        "Unknown".into()
                    }
                }),
        ),
    ];
    fields.push(match window {
        Some(window) => Field::prose(
            "Restore window",
            if window.to_now {
                format!("{} to now (WAL archived)", time(Some(window.from)))
            } else if window.from == window.until {
                // One completed backup, and nothing after it is confirmed.
                format!(
                    "Last backup {}; WAL not confirmed",
                    time(Some(window.until))
                )
            } else {
                format!(
                    "{} to {} (last backup; WAL not confirmed)",
                    time(Some(window.from)),
                    time(Some(window.until))
                )
            },
        ),
        None => Field::prose("Restore window", "Unknown"),
    });
    let archiving_field = Field::prose("WAL archiving", archiving.label());
    fields.push(match archiving {
        Archiving::Ok => archiving_field.tone(ui::Tone::Good),
        Archiving::Failing => archiving_field.tone(ui::Tone::Crit),
        Archiving::Disagree => archiving_field.tone(ui::Tone::Warn),
        Archiving::Unknown => archiving_field,
    });
    if found.no_schedule {
        fields.push(Field::prose("Schedules", "No schedule"));
    } else if !inv.schedules.complete() {
        fields.push(Field::prose(
            "Schedules",
            "Unknown: the schedule read is incomplete or stale",
        ));
    } else {
        for schedule in &found.schedules {
            let (words, raw) = cron(db.engine, &schedule.schedule);
            let state = match schedule.suspended {
                Fact::Known(true) => "Suspended",
                Fact::Known(false) => "Active",
                Fact::Unknown => "Suspension unknown",
            };
            fields.push(Field::prose(
                &schedule.identity.name,
                format!("{state} · {words}"),
            ));
            if let Some(raw) = raw {
                fields.push(Field::mono("Expression", raw));
            }
        }
    }
    fields.push(Field::prose("Restore prerequisites", found.restore.label()));
    fields.push(Field::prose(
        "Backup configuration",
        match db.backup_configuration {
            Fact::Known(true) => "Reported configured; external prerequisites unverified",
            Fact::Known(false) => "No backup or WAL archiver configuration reported",
            Fact::Unknown => "Unknown",
        },
    ));
    Section::new("backups", "Backups", fields)
}

fn proxy(db: &Database) -> Section {
    let value = match &db.proxy {
        Fact::Unknown => "Unknown: the proxy's status was not read".to_owned(),
        Fact::Known(None) => "None configured".to_owned(),
        Fact::Known(Some(proxy)) => {
            let mut text = format!(
                "{} of {} pods running · {}",
                proxy.running,
                proxy.desired,
                proxy.kind.label()
            );
            match proxy.ready {
                Fact::Known(true) => text.push_str(" · ready"),
                Fact::Known(false) => text.push_str(" · not ready"),
                Fact::Unknown => text.push_str(" · readiness not reported"),
            }
            text
        }
    };
    let mut field = Field::prose("Proxy", value);
    if let Fact::Known(Some(proxy)) = &db.proxy
        && (proxy.running < proxy.desired || proxy.ready == Fact::Known(false))
    {
        field = field.tone(ui::Tone::Warn);
    }
    Section::new("proxy", "Proxy", vec![field])
}

fn certificates(db: &Database, now: chrono::DateTime<Utc>) -> Section {
    let fields = match &db.certificates {
        Fact::Unknown => vec![Field::prose(
            "Certificates",
            match db.engine {
                Engine::CloudNativePg => "Unknown: the Cluster's status reports none",
                Engine::MariaDb => "Unknown: the MariaDB operator's status reports none",
            },
        )],
        Fact::Known(list) if list.is_empty() => {
            vec![Field::prose("Certificates", "None reported")]
        }
        Fact::Known(list) => list
            .iter()
            .map(|cert| {
                let (value, tone) = match databases::certificate(cert, now) {
                    CertificateState::Valid => (format!("Expires {}", time(cert.expires)), None),
                    CertificateState::Expiring(left) => (
                        format!("Expires in {} · {}", span(left), time(cert.expires)),
                        Some(ui::Tone::Warn),
                    ),
                    CertificateState::Expired => (
                        format!("Expired {}", time(cert.expires)),
                        Some(ui::Tone::Crit),
                    ),
                    CertificateState::Unknown => ("Expiry not reported".into(), None),
                };
                let mut field = Field::prose(&cert.name, value);
                field.tone = tone;
                field
            })
            .collect(),
    };
    Section::new("certificates", "Certificates", fields)
}

fn from_coroot(db: &Database, coroot: &CorootState) -> Section {
    let fields = match coroot {
        CorootState::NoCoroot(why) => vec![Field::prose(
            "Coroot",
            format!("{}; the operator's answer stands", why.label()),
        )],
        CorootState::AppNotFound => vec![Field::prose(
            "Coroot",
            format!(
                "Coroot lists no application for {}; the operator's answer stands",
                db.identity.name
            ),
        )],
        CorootState::NoReport => vec![Field::prose(
            "Coroot",
            format!(
                "Coroot lists {} but has no {} report for it; the operator's answer stands",
                db.identity.name,
                databases::coroot::report_name(db.engine)
            ),
        )],
        CorootState::NotRead => vec![Field::prose(
            "Coroot",
            "Not read: only the first 10 databases are looked up at once",
        )],
        CorootState::Unreachable { why, last } => {
            let mut fields = vec![
                Field::prose(
                    "Coroot",
                    format!("{why} The last answer is dimmed and not used."),
                )
                .tone(ui::Tone::Warn),
            ];
            if let Some(last) = last {
                fields.extend(report_fields(last).into_iter().map(Field::dimmed));
            }
            fields
        }
        CorootState::Read(report) => {
            let mut fields = report_fields(report);
            fields.push(Field::time("Read", Some(report.read_at)));
            fields
        }
    };
    Section::new("coroot", "From Coroot", fields)
}

/// Coroot's report: what fired first, in its words, then the folded rest.
fn report_fields(report: &DatabaseReport) -> Vec<Field> {
    use freshkube_core::coroot::Status;
    let mut fields: Vec<Field> = report
        .fired
        .iter()
        .map(|fired| {
            Field::prose(
                &fired.title,
                if fired.message.is_empty() {
                    "Fired".to_owned()
                } else {
                    fired.message.clone()
                },
            )
            .tone(if fired.status == Status::Critical {
                ui::Tone::Crit
            } else {
                ui::Tone::Warn
            })
        })
        .collect();
    if !report.ok.is_empty() {
        fields.push(Field::prose(
            &format!("{} ok", report.ok.len()),
            report.ok.join(", "),
        ));
    }
    if !report.unused.is_empty() {
        fields.push(Field::prose(
            &format!("{} unused", report.unused.len()),
            format!("{} (no data to judge by)", report.unused.join(", ")),
        ));
    }
    fields.push(Field::prose(
        "SLO",
        if report.slo.is_empty() {
            "No SLO check fired".to_owned()
        } else {
            report
                .slo
                .iter()
                .map(|slo| {
                    if slo.message.is_empty() {
                        slo.title.clone()
                    } else {
                        format!("{}: {}", slo.title, slo.message)
                    }
                })
                .collect::<Vec<_>>()
                .join(" · ")
        },
    ));
    fields
}

fn conditions(db: &Database) -> Section {
    let fields = db
        .conditions
        .iter()
        .map(|condition| {
            Field::prose(
                &condition.kind,
                format!(
                    "{} · {} · {} · transition {} · generation {}",
                    condition.status,
                    condition.reason,
                    if condition.current {
                        "current"
                    } else {
                        "freshness unknown / outdated"
                    },
                    time(condition.transition_time),
                    condition
                        .observed_generation
                        .map(|generation| generation.to_string())
                        .unwrap_or_else(|| "Not reported".into())
                ),
            )
        })
        .collect::<Vec<_>>();
    let fields = if fields.is_empty() {
        vec![Field::prose("Conditions", "None reported")]
    } else {
        fields
    };
    Section::new("conditions", "Conditions", fields)
}

fn sources(db: &Database, inv: &Inventory, coroot: &CorootState) -> Section {
    let mut fields = vec![
        Field::prose("Operator", db.engine.label()),
        Field::mono(
            "Object",
            format!(
                "{}/{} {}",
                db.identity.group, db.identity.version, db.identity.kind
            ),
        ),
        Field::time("Read", Some(inv.databases.observed_at)),
    ];
    fields.push(match coroot {
        CorootState::Read(report) => Field::time("Coroot read", Some(report.read_at)),
        CorootState::Unreachable { last, .. } => Field::prose(
            "Coroot read",
            match last {
                Some(last) => format!("Unreachable; last answer {}", time(Some(last.read_at))),
                None => "Unreachable".into(),
            },
        ),
        CorootState::NoCoroot(_)
        | CorootState::AppNotFound
        | CorootState::NoReport
        | CorootState::NotRead => Field::prose("Coroot read", "None"),
    });
    fields.push(Field::prose(
        "Lag",
        "Coroot, else the operator's own status; Prometheus is not read",
    ));
    Section::new("sources", "Sources", fields)
}
