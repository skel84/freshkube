use super::*;
use std::collections::HashSet;

pub(super) fn projects(values: &[ProjectInfo]) -> Result<(), ReadError> {
    count(values.len(), 100)?;
    let mut ids = HashSet::new();
    for value in values {
        if value.id.is_empty() || !ids.insert(&value.id) {
            return Err(ReadError::InvalidResponse);
        }
        text(&value.id, 256)?;
        text(&value.name, 1024)?;
    }
    Ok(())
}
pub(super) fn applications(values: &[Application]) -> Result<(), ReadError> {
    count(values.len(), 2_000)?;
    let mut ids = HashSet::new();
    for app in values {
        if app.id.as_str().is_empty() || !ids.insert(&app.id) {
            return Err(ReadError::InvalidResponse);
        }
        text(app.id.as_str(), 1024)?;
        text(&app.category, 256)?;
        text(&app.app_type, 256)?;
        count(app.signals.len(), 12)?;
        for signal in app.signals.values() {
            text(&signal.value, 4096)?;
        }
    }
    Ok(())
}
/// As many applications as the Applications list takes; the map view pages
/// them and draws only the connections in view.
pub(super) fn map(value: &ServiceMap) -> Result<(), ReadError> {
    count(value.nodes.len(), 2_000)?;
    count(value.edges.len(), 20_000)?;
    let mut nodes = HashSet::new();
    for node in &value.nodes {
        text(node.id.as_str(), 1024)?;
        if node.id.as_str().is_empty() || !nodes.insert(&node.id) {
            return Err(ReadError::InvalidResponse);
        }
    }
    let mut links = HashSet::new();
    for edge in &value.edges {
        if !nodes.contains(&edge.from)
            || !nodes.contains(&edge.to)
            || !links.insert((&edge.from, &edge.to))
        {
            return Err(ReadError::InvalidResponse);
        }
        text(&edge.issue, 4096)?;
    }
    Ok(())
}

/// Coroot's page for one application. Charts are bounded per page rather
/// than per report, since a report's charts are drawn side by side.
pub(super) fn app_view(value: &AppView) -> Result<(), ReadError> {
    let map = &value.map;
    map_app(&map.app)?;
    count(map.instances.len(), 2_000)?;
    for instance in &map.instances {
        text(&instance.id, 1024)?;
        labels(&instance.labels)?;
    }
    count(map.clients.len() + map.dependencies.len(), 1_000)?;
    for app in map.clients.iter().chain(&map.dependencies) {
        map_app(app)?;
    }
    count(value.reports.len(), 32)?;
    let mut charts = 0;
    let mut series = 0;
    let mut cells = 0;
    for report in &value.reports {
        text(&report.name, 256)?;
        text(&report.instrumentation, 256)?;
        count(report.checks.len(), 64)?;
        for c in &report.checks {
            check(c)?;
        }
        count(report.widgets.len(), 128)?;
        for widget in &report.widgets {
            match &widget.kind {
                WidgetKind::Chart(c) => {
                    charts += 1;
                    series += chart(c)?;
                }
                WidgetKind::ChartGroup {
                    title,
                    charts: group,
                } => {
                    text(title, 1024)?;
                    charts += group.len();
                    for c in group {
                        series += chart(c)?;
                    }
                }
                WidgetKind::Table(table) => {
                    count(table.header.len(), 32)?;
                    for h in &table.header {
                        text(h, 1024)?;
                    }
                    count(table.rows.len(), 2_000)?;
                    for row in &table.rows {
                        count(row.len(), 32)?;
                        cells += row.len();
                        for c in row {
                            cell(c)?;
                        }
                    }
                }
                WidgetKind::Heatmap(h) => {
                    text(&h.title, 1024)?;
                    count(h.rows.len(), 64)?;
                    series += h.rows.len();
                    for row in &h.rows {
                        points(row)?;
                    }
                }
                WidgetKind::Logs(Some(c)) => check(c)?,
                WidgetKind::Header(h) => text(h, 1024)?,
                _ => {}
            }
        }
    }
    count(charts, 512)?;
    count(series, 4_096)?;
    count(cells, 50_000)?;
    Ok(())
}
/// An application's deployments: coroot-rs bounds how many and their
/// findings; the text is bounded here, as the page's is.
pub(super) fn revisions(values: &[coroot_rs::DeploymentRevision]) -> Result<(), ReadError> {
    for revision in values {
        text(&revision.id, 1024)?;
        text(&revision.version, 4096)?;
        text(revision.note.as_deref().unwrap_or_default(), 4096)?;
        for finding in &revision.findings {
            text(&finding.report, 256)?;
            text(&finding.message, 4096)?;
        }
    }
    Ok(())
}
fn map_app(app: &MapApp) -> Result<(), ReadError> {
    text(app.id.as_str(), 1024)?;
    text(&app.cluster, 1024)?;
    text(&app.category, 256)?;
    text(&app.icon, 256)?;
    labels(&app.labels)?;
    if let Some(link) = &app.link {
        text(&link.reason, 4096)?;
        count(link.stats.len(), 16)?;
        for stat in &link.stats {
            text(stat, 256)?;
        }
    }
    Ok(())
}
fn labels(labels: &std::collections::BTreeMap<String, String>) -> Result<(), ReadError> {
    count(labels.len(), 32)?;
    for (key, value) in labels {
        text(key, 256)?;
        text(value, 1024)?;
    }
    Ok(())
}
fn check(c: &Check) -> Result<(), ReadError> {
    text(&c.id, 256)?;
    text(&c.title, 1024)?;
    text(&c.message, 8192)?;
    text(&c.unit, 64)?;
    text(&c.condition, 4096)
}
/// The chart's own bounds; returns how many series it has.
fn chart(c: &AppChart) -> Result<usize, ReadError> {
    text(&c.title, 1024)?;
    count(c.series.len(), 128)?;
    count(c.annotations.len(), 256)?;
    for a in &c.annotations {
        text(&a.name, 1024)?;
        text(&a.icon, 64)?;
    }
    for s in c.series.iter().chain(&c.threshold) {
        points(s)?;
    }
    Ok(c.series.len() + usize::from(c.threshold.is_some()))
}
fn points(s: &Series) -> Result<(), ReadError> {
    text(&s.name, 1024)?;
    text(&s.title, 1024)?;
    text(&s.color, 64)?;
    count(s.points.len(), 4_096)
}
fn cell(c: &Cell) -> Result<(), ReadError> {
    text(&c.value, 4096)?;
    text(&c.short_value, 1024)?;
    text(&c.unit, 64)?;
    count(c.values.len(), 32)?;
    for v in &c.values {
        text(v, 4096)?;
    }
    count(c.tags.len(), 32)?;
    for t in &c.tags {
        text(t, 256)?;
    }
    if let Some((name, color)) = &c.icon {
        text(name, 256)?;
        text(color, 64)?;
    }
    if let Some(link) = &c.link {
        text(&link.title, 1024)?;
        text(&link.view, 256)?;
        text(&link.id, 1024)?;
    }
    if let Some((_, color)) = &c.progress {
        text(color, 64)?;
    }
    if let Some((rx, tx)) = &c.bandwidth {
        text(rx, 64)?;
        text(tx, 64)?;
    }
    count(c.chart.len(), 4_096)?;
    count(c.deployments.len(), 32)?;
    for d in &c.deployments {
        text(&d.report, 256)?;
        text(&d.message, 4096)?;
    }
    Ok(())
}

pub(super) fn tracing(value: &Tracing) -> Result<(), ReadError> {
    text(&value.message, 4096)?;
    count(value.sources.len(), 8)?;
    for source in &value.sources {
        text(&source.kind, 64)?;
        text(&source.name, 256)?;
    }
    if let Some(heatmap) = &value.heatmap {
        count(heatmap.rows.len(), 32)?;
        for row in &heatmap.rows {
            text(&row.name, 64)?;
            text(&row.value, 64)?;
            count(row.points.len(), 4_096)?;
        }
    }
    // One trace can have many more spans than Coroot's list of 100.
    count(value.spans.len(), 5_000)?;
    for span in &value.spans {
        text(&span.service, 1024)?;
        text(&span.trace_id, 256)?;
        text(&span.id, 256)?;
        text(&span.parent_id, 256)?;
        text(&span.name, 4096)?;
        text(&span.status.message, 16_384)?;
        text(&span.details.text, 65_536)?;
        count(span.attributes.len(), 256)?;
        for (key, value) in &span.attributes {
            text(key, 1024)?;
            text(value, 65_536)?;
        }
        count(span.events.len(), 128)?;
        for event in &span.events {
            text(&event.name, 1024)?;
            count(event.attributes.len(), 64)?;
            for (key, value) in &event.attributes {
                text(key, 1024)?;
                text(value, 65_536)?;
            }
        }
    }
    Ok(())
}

/// A read asks for at most 1,000 messages; each is bounded on its own, and
/// all of them together, so a page of stack traces stays a page.
pub(super) fn logs(value: &LogsView) -> Result<(), ReadError> {
    text(&value.message, 4096)?;
    count(value.origins.len(), 4)?;
    if let Some(c) = &value.chart {
        chart(c)?;
    }
    count(value.lines.len(), 1_000)?;
    let mut bytes = 0;
    for line in &value.lines {
        text(&line.severity, 64)?;
        text(&line.message, 65_536)?;
        text(&line.trace_id, 256)?;
        count(line.attributes.len(), 128)?;
        bytes += line.message.len();
        for (key, value) in &line.attributes {
            text(key, 1024)?;
            text(value, 16_384)?;
            bytes += key.len() + value.len();
        }
    }
    count(bytes, 8 << 20)?;
    count(value.patterns.len(), 1_000)?;
    for pattern in &value.patterns {
        text(&pattern.severity, 64)?;
        text(&pattern.sample, 16_384)?;
        if let Some(c) = &pattern.chart {
            chart(c)?;
        }
    }
    Ok(())
}

pub(super) fn profiling(value: &Profiling) -> Result<(), ReadError> {
    text(&value.message, 4096)?;
    count(value.kinds.len(), 64)?;
    for kind in &value.kinds {
        text(&kind.id, 128)?;
        text(&kind.name, 256)?;
    }
    count(value.instances.len(), 2_000)?;
    for instance in &value.instances {
        text(instance, 512)?;
    }
    Ok(())
}

pub(super) fn incidents(values: &[Incident]) -> Result<(), ReadError> {
    count(values.len(), 100)?;
    let mut keys = HashSet::new();
    for value in values {
        if value.key.is_empty() || value.app.as_str().is_empty() || !keys.insert(&value.key) {
            return Err(ReadError::InvalidResponse);
        }
        incident(value)?;
    }
    Ok(())
}

fn incident(value: &Incident) -> Result<(), ReadError> {
    text(&value.key, 256)?;
    text(value.app.as_str(), 1024)?;
    text(&value.cluster, 1024)?;
    text(&value.description, 4096)?;
    if let Some(rca) = &value.rca {
        text(&rca.status, 256)?;
        text(&rca.summary, 4096)?;
        text(&rca.root_cause, 16384)?;
        text(&rca.immediate_fixes, 16384)?;
        text(&rca.detailed_analysis, 32768)?;
        text(&rca.error, 4096)?;
        count(rca.propagation.len(), 100)?;
        let mut entries = 0;
        let mut bytes = rca.summary.len()
            + rca.root_cause.len()
            + rca.immediate_fixes.len()
            + rca.detailed_analysis.len()
            + rca.error.len();
        let mut apps = HashSet::new();
        for app in &rca.propagation {
            if app.app_id.as_str().is_empty() || !apps.insert(&app.app_id) {
                return Err(ReadError::InvalidResponse);
            }
            text(app.app_id.as_str(), 1024)?;
            entries += app.issues.len();
            for issue in &app.issues {
                text(issue, 4096)?;
                bytes += issue.len();
            }
        }
        count(entries, 200)?;
        count(bytes, 65536)?;
    }
    if let Some(slo) = &value.slo {
        count(
            slo.availability_burn_rates.len() + slo.latency_burn_rates.len(),
            32,
        )?;
    }
    Ok(())
}

pub(super) fn incident_view(value: &IncidentView) -> Result<(), ReadError> {
    incident(value.incident())?;
    for objective in [value.availability(), value.latency()]
        .into_iter()
        .flatten()
    {
        text(objective.objective(), 4096)?;
        text(objective.compliance(), 256)?;
    }
    Ok(())
}
fn count(value: usize, max: usize) -> Result<(), ReadError> {
    if value > max {
        Err(ReadError::Limit)
    } else {
        Ok(())
    }
}
fn text(value: &str, max: usize) -> Result<(), ReadError> {
    count(value.len(), max)
}
