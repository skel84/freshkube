//! Processes example data for a Talos node.
use std::time::Instant;

use talos_rs::{CpuStat, ProcessInfo, ProcessState};

use super::ExampleNode;
use crate::inspection::{
    InspectionSource, InspectionTarget, InspectionUnavailable, LoadAverageSnapshot,
    ProcessCpuSnapshot, ProcessInspectionSnapshot, ProcessMemorySnapshot, ProcessSampleState,
    ProcessSnapshotEntry, ProcessStateCounts, ProcessSystemSnapshot,
};

/// pid, ppid, state, command, arguments, CPU seconds, MiB resident, threads, CPU%.
type ExampleProcess = (
    i32,
    i32,
    ProcessState,
    &'static str,
    &'static str,
    f64,
    u64,
    i32,
    f32,
);

/// Example processes for `--fixture`: a plausible Talos node whose CPU
/// figures move a little on each refresh.
pub fn processes(
    node: &ExampleNode,
    target: InspectionTarget,
    tick: u64,
) -> ProcessInspectionSnapshot {
    let control_plane = node.control_plane;
    let degraded = node.name.contains("wk-fra1-02");
    let mut spec: Vec<ExampleProcess> = vec![
        (
            1,
            0,
            ProcessState::Sleeping,
            "init",
            "/sbin/init",
            312.4,
            38,
            9,
            0.2,
        ),
        (
            412,
            1,
            ProcessState::Sleeping,
            "udevd",
            "/sbin/udevd --resolve-names=never",
            4.1,
            9,
            1,
            0.0,
        ),
        (
            590,
            1,
            ProcessState::Sleeping,
            "machined",
            "/sbin/init machined",
            841.0,
            84,
            14,
            0.6,
        ),
        (
            604,
            1,
            ProcessState::Sleeping,
            "apid",
            "/apid",
            196.2,
            46,
            12,
            0.3,
        ),
        (
            611,
            1,
            ProcessState::Sleeping,
            "trustd",
            "/trustd",
            22.8,
            21,
            9,
            0.0,
        ),
        (
            633,
            1,
            ProcessState::Sleeping,
            "containerd",
            "/bin/containerd --address /system/run/containerd/containerd.sock --state /system/run/containerd --root /system/var/lib/containerd",
            410.9,
            61,
            16,
            0.4,
        ),
        (
            702,
            1,
            ProcessState::Sleeping,
            "containerd",
            "/bin/containerd --address /run/containerd/containerd.sock --config /etc/cri/containerd.toml",
            1873.5,
            118,
            22,
            1.8,
        ),
        (
            781,
            702,
            ProcessState::Sleeping,
            "kubelet",
            "/usr/local/bin/kubelet --config=/etc/kubernetes/kubelet.yaml --kubeconfig=/etc/kubernetes/kubeconfig-kubelet --bootstrap-kubeconfig=/etc/kubernetes/bootstrap-kubeconfig --cert-dir=/var/lib/kubelet/pki --node-ip=10.20.0.11",
            5310.2,
            142,
            24,
            3.4,
        ),
        (
            1022,
            702,
            ProcessState::Sleeping,
            "containerd-shim",
            "/bin/containerd-shim-runc-v2 -namespace k8s.io -id 4be1c2 -address /run/containerd/containerd.sock",
            12.3,
            14,
            11,
            0.0,
        ),
        (
            1041,
            1022,
            ProcessState::Sleeping,
            "pause",
            "/pause",
            0.1,
            1,
            1,
            0.0,
        ),
        (
            1063,
            1022,
            ProcessState::Sleeping,
            "flanneld",
            "/opt/bin/flanneld --ip-masq --kube-subnet-mgr",
            288.0,
            36,
            13,
            0.2,
        ),
        (
            1102,
            702,
            ProcessState::Sleeping,
            "containerd-shim",
            "/bin/containerd-shim-runc-v2 -namespace k8s.io -id 9ac0f1 -address /run/containerd/containerd.sock",
            9.8,
            13,
            11,
            0.0,
        ),
        (
            1120,
            1102,
            ProcessState::Sleeping,
            "kube-proxy",
            "/usr/local/bin/kube-proxy --config=/etc/kubernetes/kube-proxy.yaml --hostname-override=$(NODE_NAME)",
            402.7,
            44,
            10,
            0.3,
        ),
        (
            1188,
            1102,
            ProcessState::Running,
            "coredns",
            "/coredns -conf /etc/coredns/Corefile",
            640.0,
            52,
            13,
            1.1,
        ),
    ];
    if control_plane {
        spec.extend([
            (690, 1, ProcessState::Sleeping, "etcd", "/usr/local/bin/etcd --name=talos-cp --data-dir=/var/lib/etcd --listen-client-urls=https://0.0.0.0:2379 --listen-peer-urls=https://0.0.0.0:2380", 9120.6, 412, 21, 6.2),
            (1301, 702, ProcessState::Sleeping, "containerd-shim", "/bin/containerd-shim-runc-v2 -namespace k8s.io -id f20d77 -address /run/containerd/containerd.sock", 14.1, 15, 11, 0.0),
            (1322, 1301, ProcessState::Running, "kube-apiserver", "/usr/local/bin/kube-apiserver --advertise-address=10.20.0.11 --allow-privileged=true --authorization-mode=Node,RBAC --etcd-servers=https://localhost:2379 --secure-port=6443", 24410.4, 1180, 38, 14.7),
            (1340, 1301, ProcessState::Sleeping, "kube-controller-manager", "/usr/local/bin/kube-controller-manager --leader-elect=true --use-service-account-credentials", 3810.0, 186, 14, 1.9),
            (1355, 1301, ProcessState::Sleeping, "kube-scheduler", "/usr/local/bin/kube-scheduler --leader-elect=true", 1022.6, 74, 12, 0.5),
        ]);
    } else {
        spec.extend([
            (2101, 702, ProcessState::Sleeping, "containerd-shim", "/bin/containerd-shim-runc-v2 -namespace k8s.io -id 3d81aa -address /run/containerd/containerd.sock", 18.4, 16, 11, 0.0),
            (2140, 2101, ProcessState::Running, "postgres", "postgres: checkpointer", 2210.3, 512, 1, 8.4),
            (2141, 2101, ProcessState::Sleeping, "postgres", "postgres: walwriter", 840.0, 64, 1, 0.9),
            (2190, 702, ProcessState::Sleeping, "containerd-shim", "/bin/containerd-shim-runc-v2 -namespace k8s.io -id 77be02 -address /run/containerd/containerd.sock", 7.2, 12, 11, 0.0),
            (2215, 2190, ProcessState::Sleeping, "node", "node /srv/app/server.js --port 8080", 1902.1, 288, 11, 4.6),
        ]);
    }
    if degraded {
        spec.extend([
            (2216, 2215, ProcessState::Zombie, "node", "", 0.4, 0, 1, 0.0),
            (
                2302,
                702,
                ProcessState::DiskSleep,
                "fio",
                "fio --name=randwrite --ioengine=libaio --rw=randwrite --bs=4k --size=2g",
                66.0,
                24,
                1,
                0.0,
            ),
        ]);
    }
    // A slow wobble so successive example refreshes show CPU movement.
    let wobble = |pid: i32| ((tick as i32 + pid) % 7) as f32 * 0.15;
    let first_sample = tick == 0;
    let processes: Vec<ProcessSnapshotEntry> = spec
        .into_iter()
        .map(
            |(pid, ppid, state, command, args, cpu_time, memory_mib, threads, cpu)| {
                let executable = args
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                ProcessSnapshotEntry {
                    cpu_percent: (!first_sample).then(|| (cpu + wobble(pid)).max(0.)),
                    process: ProcessInfo {
                        pid,
                        ppid,
                        state,
                        threads,
                        cpu_time: cpu_time + tick as f64 * f64::from(cpu) / 10.,
                        virtual_memory: memory_mib * 1024 * 1024 * 3,
                        resident_memory: memory_mib * 1024 * 1024,
                        command: command.to_owned(),
                        executable: if executable.starts_with('/') {
                            executable
                        } else {
                            String::new()
                        },
                        args: args.to_owned(),
                    },
                }
            },
        )
        .collect();
    let mut state_counts = ProcessStateCounts::default();
    for entry in &processes {
        match entry.process.state {
            ProcessState::Running => state_counts.running += 1,
            ProcessState::Sleeping => state_counts.sleeping += 1,
            ProcessState::DiskSleep => state_counts.disk_sleep += 1,
            ProcessState::Zombie => state_counts.zombie += 1,
            _ => {}
        }
    }
    let total_bytes = 8 * 1024 * 1024 * 1024_u64;
    let used_bytes = if degraded {
        total_bytes / 100 * 91
    } else {
        total_bytes / 100 * 47
    };
    // Count processes as the sample's "tick" so the next refresh moves CPU.
    let next_sample = ProcessSampleState {
        cpu_times: processes
            .iter()
            .take(tick as usize + 1)
            .map(|entry| (entry.process.pid, entry.process.cpu_time))
            .collect(),
        ..ProcessSampleState::default()
    };
    ProcessInspectionSnapshot {
        target,
        sampled_at: Instant::now(),
        state_counts,
        system: ProcessSystemSnapshot {
            memory: Some(ProcessMemorySnapshot {
                total_bytes,
                used_bytes,
                used_percent: used_bytes as f32 / total_bytes as f32 * 100.,
            }),
            cpu_count: node.cores.or(Some(4)),
            cpu: Some(ProcessCpuSnapshot {
                totals: CpuStat::default(),
                usage_percent: (!first_sample).then_some(if degraded { 71.0 } else { 23.5 }),
                running_processes: state_counts.running as u64,
                blocked_processes: state_counts.disk_sleep as u64,
            }),
            load_average: Some(LoadAverageSnapshot {
                one_minute: if degraded { 5.12 } else { 0.84 },
                five_minutes: if degraded { 4.40 } else { 0.71 },
                fifteen_minutes: if degraded { 3.95 } else { 0.66 },
            }),
        },
        unavailable: if degraded {
            vec![InspectionUnavailable {
                source: InspectionSource::ProcessCpuInfo,
                message: "Example: the CPU information request timed out after 10 s".into(),
            }]
        } else {
            Vec::new()
        },
        processes,
        next_sample,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(
        name: &str,
        control_plane: bool,
        cores: Option<usize>,
        tick: u64,
    ) -> ProcessInspectionSnapshot {
        processes(
            &ExampleNode::new(name, control_plane, cores),
            InspectionTarget::new(name, "192.0.2.10"),
            tick,
        )
    }

    #[test]
    fn control_plane_and_worker_have_their_own_processes() {
        let control_plane = snapshot("example-control-plane", true, Some(8), 0);
        let worker = snapshot("example-worker", false, Some(16), 0);
        assert_eq!(
            control_plane.target,
            InspectionTarget::new("example-control-plane", "192.0.2.10")
        );
        assert_eq!(
            worker.target,
            InspectionTarget::new("example-worker", "192.0.2.10")
        );
        assert_eq!(control_plane.processes.len(), 19);
        assert_eq!(worker.processes.len(), 19);
        for (control_plane, worker) in control_plane
            .processes
            .iter()
            .zip(&worker.processes)
            .take(14)
        {
            assert_eq!(format!("{control_plane:?}"), format!("{worker:?}"));
        }
        let has = |snapshot: &ProcessInspectionSnapshot, command: &str| {
            snapshot
                .processes
                .iter()
                .any(|entry| entry.process.command == command)
        };
        for command in [
            "etcd",
            "kube-apiserver",
            "kube-controller-manager",
            "kube-scheduler",
        ] {
            assert!(has(&control_plane, command));
            assert!(!has(&worker, command));
        }
        for command in ["postgres", "node"] {
            assert!(has(&worker, command));
            assert!(!has(&control_plane, command));
        }
        assert_eq!(control_plane.system.cpu_count, Some(8));
        assert_eq!(worker.system.cpu_count, Some(16));
        assert!(!control_plane.is_partial());
        assert!(!worker.is_partial());
    }

    #[test]
    fn degraded_worker_reports_waits_memory_pressure_and_missing_cpu_info() {
        let normal = snapshot("example-worker", false, None, 3);
        let degraded = snapshot("talos-wk-fra1-02", false, None, 3);
        assert_eq!(normal.processes.len(), 19);
        assert_eq!(degraded.processes.len(), 21);
        assert_eq!(normal.state_counts.zombie, 0);
        assert_eq!(normal.state_counts.disk_sleep, 0);
        assert_eq!(degraded.state_counts.zombie, 1);
        assert_eq!(degraded.state_counts.disk_sleep, 1);
        assert!(
            degraded
                .processes
                .iter()
                .any(|entry| entry.process.pid == 2216
                    && entry.process.state == ProcessState::Zombie)
        );
        assert!(degraded.processes.iter().any(
            |entry| entry.process.pid == 2302 && entry.process.state == ProcessState::DiskSleep
        ));
        let memory = degraded.system.memory.unwrap();
        assert_eq!(memory.total_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(memory.used_bytes, memory.total_bytes / 100 * 91);
        assert_eq!(
            normal.system.memory.unwrap().used_bytes,
            memory.total_bytes / 100 * 47
        );
        assert_eq!(degraded.system.cpu_count, Some(4));
        assert_eq!(normal.system.cpu_count, Some(4));
        assert_eq!(
            degraded.system.cpu.as_ref().unwrap().usage_percent,
            Some(71.0)
        );
        assert_eq!(
            normal.system.cpu.as_ref().unwrap().usage_percent,
            Some(23.5)
        );
        assert_eq!(degraded.system.cpu.as_ref().unwrap().blocked_processes, 1);
        assert_eq!(
            degraded.system.load_average,
            Some(LoadAverageSnapshot {
                one_minute: 5.12,
                five_minutes: 4.40,
                fifteen_minutes: 3.95,
            })
        );
        assert_eq!(
            degraded.unavailable,
            vec![InspectionUnavailable {
                source: InspectionSource::ProcessCpuInfo,
                message: "Example: the CPU information request timed out after 10 s".into(),
            }]
        );
        assert!(normal.unavailable.is_empty());
    }

    #[test]
    fn tick_moves_only_cpu_figures_and_their_sample_bookkeeping() {
        for (name, control_plane) in [
            ("example-control-plane", true),
            ("example-worker", false),
            ("talos-wk-fra1-02", false),
        ] {
            let first = snapshot(name, control_plane, Some(8), 0);
            assert!(
                first
                    .processes
                    .iter()
                    .all(|entry| entry.cpu_percent.is_none())
            );
            assert!(first.system.cpu.as_ref().unwrap().usage_percent.is_none());
            assert_eq!(first.next_sample.cpu_times.len(), 1);
            for tick in [3, 4] {
                let mut later = snapshot(name, control_plane, Some(8), tick);
                assert_eq!(later.next_sample.cpu_times.len(), tick as usize + 1);
                for entry in later.processes.iter().take(tick as usize + 1) {
                    assert_eq!(
                        later.next_sample.cpu_times[&entry.process.pid],
                        entry.process.cpu_time
                    );
                }
                assert!(
                    later
                        .processes
                        .iter()
                        .all(|entry| entry.cpu_percent.is_some())
                );
                assert!(later.system.cpu.as_ref().unwrap().usage_percent.is_some());
                assert!(
                    later
                        .processes
                        .iter()
                        .zip(&first.processes)
                        .any(|(later, first)| later.process.cpu_time != first.process.cpu_time)
                );
                for (later, first) in later.processes.iter_mut().zip(&first.processes) {
                    later.cpu_percent = first.cpu_percent;
                    later.process.cpu_time = first.process.cpu_time;
                }
                later.system.cpu.as_mut().unwrap().usage_percent =
                    first.system.cpu.as_ref().unwrap().usage_percent;
                later.sampled_at = first.sampled_at;
                later.next_sample = first.next_sample.clone();
                assert_eq!(format!("{later:?}"), format!("{first:?}"));
            }
            let third = snapshot(name, control_plane, Some(8), 3);
            let fourth = snapshot(name, control_plane, Some(8), 4);
            assert!(
                third
                    .processes
                    .iter()
                    .zip(&fourth.processes)
                    .any(|(third, fourth)| third.cpu_percent != fourth.cpu_percent)
            );
        }
    }
}
