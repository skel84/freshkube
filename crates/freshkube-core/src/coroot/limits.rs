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
pub(super) fn map(value: &ServiceMap) -> Result<(), ReadError> {
    count(value.nodes.len(), 120)?;
    count(value.edges.len(), 300)?;
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
pub(super) fn health(value: &AppHealth) -> Result<(), ReadError> {
    text(value.id.as_str(), 1024)?;
    text(&value.namespace, 256)?;
    count(value.reports.len(), 32)?;
    count(value.dependencies.len() + value.clients.len(), 300)?;
    count(value.vitals.len(), 32)?;
    let mut entries = 0;
    let mut series_count = value.vitals.len();
    for report in &value.reports {
        text(&report.name, 256)?;
        entries += report.issues.len() + report.log_patterns.len();
        count(report.charts.len(), 32)?;
        for issue in &report.issues {
            text(&issue.id, 1024)?;
            text(&issue.title, 4096)?;
            text(&issue.message, 8192)?;
        }
        for log in &report.log_patterns {
            text(&log.hash, 256)?;
            text(&log.severity, 64)?;
            text(&log.sample, 4096)?;
        }
        for chart in &report.charts {
            series_count += chart.series.len();
            text(&chart.title, 1024)?;
            count(chart.series.len(), 32)?;
            for series in &chart.series {
                summary(series)?;
            }
        }
    }
    count(entries, 200)?;
    count(series_count, 128)?;
    for series in &value.vitals {
        summary(series)?;
    }
    for dependency in &value.dependencies {
        text(dependency.id.as_str(), 1024)?;
        text(&dependency.connectivity_message, 4096)?;
        count(dependency.protocols.len(), 16)?;
        for protocol in &dependency.protocols {
            text(protocol, 64)?;
        }
    }
    for client in &value.clients {
        text(client.id.as_str(), 1024)?;
    }
    Ok(())
}
fn summary(series: &SeriesSummary) -> Result<(), ReadError> {
    text(&series.name, 1024)?;
    count(series.sparkline.len(), 512)?;
    count(series.labels.len(), 32)?;
    for (key, value) in &series.labels {
        text(key, 256)?;
        text(value, 1024)?;
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
