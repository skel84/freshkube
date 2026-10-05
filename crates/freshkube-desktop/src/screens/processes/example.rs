//! Example processes for `--fixture`.
use super::*;

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
pub(super) fn example(
    source: &ScreenSource,
    tick: u64,
) -> Result<ProcessInspectionSnapshot, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let control_plane = node.role == crate::presentation::Role::ControlPlane;
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
    Ok(ProcessInspectionSnapshot {
        target: source.inspection_target(),
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
                source: freshkube_core::inspection::InspectionSource::ProcessCpuInfo,
                message: "Example: the CPU information request timed out after 10 s".into(),
            }]
        } else {
            Vec::new()
        },
        processes,
        next_sample,
    })
}
