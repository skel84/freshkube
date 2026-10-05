//! Effective pod requests, reduced before the full object leaves the reflector.
use std::collections::BTreeMap;

use k8s_openapi::{api::core::v1::Pod, apimachinery::pkg::api::resource::Quantity};

use crate::resources::{Amounts, cpu_millis, quantity};

pub(super) fn active(pod: &Pod) -> bool {
    !matches!(
        pod.status.as_ref().and_then(|s| s.phase.as_deref()),
        Some("Succeeded" | "Failed")
    )
}

pub(super) const ZERO: Amounts = Amounts {
    cpu_millis: Some(0.),
    memory_bytes: Some(0.),
};

pub(super) fn add(total: &mut Amounts, value: Amounts) {
    total.cpu_millis = total.cpu_millis.zip(value.cpu_millis).map(|(a, b)| a + b);
    total.memory_bytes = total
        .memory_bytes
        .zip(value.memory_bytes)
        .map(|(a, b)| a + b);
}

/// Kubernetes' PodRequests: steady app/sidecar sum or the init peak, whichever
/// is larger; pod-level requests override each named resource, then overhead
/// is added. Missing requests are zero; malformed quantities remain unknown.
pub(super) fn of(pod: &Pod) -> Amounts {
    let Some(spec) = &pod.spec else {
        return Amounts::default();
    };
    let resource = |key: &str, parse: fn(&str) -> Option<f64>| {
        let amount =
            |values: Option<&BTreeMap<String, Quantity>>| match values.and_then(|v| v.get(key)) {
                Some(value) => parse(&value.0).filter(|v| v.is_finite() && *v >= 0.),
                None => Some(0.),
            };
        let mut steady = Some(0.);
        for container in &spec.containers {
            steady = steady
                .zip(amount(
                    container
                        .resources
                        .as_ref()
                        .and_then(|r| r.requests.as_ref()),
                ))
                .map(|(a, b)| a + b);
        }
        let mut sidecars = Some(0.);
        let mut peak = Some(0.);
        for container in spec.init_containers.iter().flatten() {
            let request = amount(
                container
                    .resources
                    .as_ref()
                    .and_then(|r| r.requests.as_ref()),
            );
            let stage = if container.restart_policy.as_deref() == Some("Always") {
                steady = steady.zip(request).map(|(a, b)| a + b);
                sidecars = sidecars.zip(request).map(|(a, b)| a + b);
                sidecars
            } else {
                sidecars.zip(request).map(|(a, b)| a + b)
            };
            peak = peak.zip(stage).map(|(a, b): (f64, f64)| a.max(b));
        }
        let total = if spec
            .resources
            .as_ref()
            .and_then(|r| r.requests.as_ref())
            .is_some_and(|r| r.contains_key(key))
        {
            amount(spec.resources.as_ref().and_then(|r| r.requests.as_ref()))
        } else {
            steady.zip(peak).map(|(a, b): (f64, f64)| a.max(b))
        };
        total
            .zip(amount(spec.overhead.as_ref()))
            .map(|(a, b)| a + b)
    };
    Amounts {
        cpu_millis: resource("cpu", cpu_millis),
        memory_bytes: resource("memory", quantity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn init_peak_sidecars_overhead_and_pod_level_requests() {
        let mut pod: Pod = serde_json::from_value(json!({"spec": {
            "containers":[{"name":"app","resources":{"requests":{"cpu":"500m","memory":"128Mi"}}}],
            "initContainers":[
                {"name":"sidecar","restartPolicy":"Always","resources":{"requests":{"cpu":"100m","memory":"64Mi"}}},
                {"name":"init","resources":{"requests":{"cpu":"1","memory":"256Mi"}}}
            ], "overhead":{"cpu":"50m","memory":"32Mi"}
        }})).unwrap();
        assert_eq!(
            of(&pod),
            Amounts {
                cpu_millis: Some(1150.),
                memory_bytes: Some(352. * 1024. * 1024.)
            }
        );
        pod.spec.as_mut().unwrap().resources =
            Some(serde_json::from_value(json!({"requests":{"cpu":"2"}})).unwrap());
        assert_eq!(of(&pod).cpu_millis, Some(2050.));
        assert_eq!(of(&pod).memory_bytes, Some(352. * 1024. * 1024.));
    }

    #[test]
    fn missing_requests_are_zero_but_bad_quantities_are_unknown() {
        let mut pod: Pod =
            serde_json::from_value(json!({"spec":{"containers":[{"name":"app"}]}})).unwrap();
        assert_eq!(of(&pod), ZERO);
        pod.spec.as_mut().unwrap().containers[0].resources =
            Some(serde_json::from_value(json!({"requests":{"cpu":"bad","memory":"-1"}})).unwrap());
        assert_eq!(of(&pod), Amounts::default());
    }
}
