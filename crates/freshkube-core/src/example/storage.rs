//! Storage example data for a Talos node.
use talos_rs::{DiskInfo, VolumeStatus};

use super::ExampleNode;

#[allow(clippy::too_many_arguments)]
pub fn disk(
    id: &str,
    bytes: u64,
    pretty: &str,
    model: &str,
    serial: &str,
    transport: &str,
    rotational: bool,
    readonly: bool,
    cdrom: bool,
) -> DiskInfo {
    DiskInfo {
        id: id.to_owned(),
        dev_path: format!("/dev/{id}"),
        size: bytes,
        size_pretty: pretty.to_owned(),
        model: Some(model.to_owned()),
        serial: Some(serial.to_owned()),
        transport: Some(transport.to_owned()),
        rotational,
        readonly,
        cdrom,
        wwid: Some(format!("eui.0025385{serial}00a1b2c3d4")),
        bus_path: Some(format!(
            "/pci0000:00/0000:00:1f.2/ata1/host0/target0:0:0/{id}"
        )),
    }
}

fn volume(
    id: &str,
    phase: &str,
    size: &str,
    filesystem: Option<&str>,
    mount: Option<&str>,
    encryption: Option<&str>,
) -> VolumeStatus {
    VolumeStatus {
        id: id.to_owned(),
        encryption_provider: encryption.map(str::to_owned),
        phase: phase.to_owned(),
        size: size.to_owned(),
        filesystem: filesystem.map(str::to_owned),
        mount_location: mount.map(str::to_owned),
    }
}

/// Example disks and volumes for `--fixture`. The degraded worker has a
/// volume still waiting; the bare-metal control plane's volume query times
/// out, so only its disks show.
pub fn storage(node: &ExampleNode) -> (Vec<DiskInfo>, Result<Vec<VolumeStatus>, String>) {
    let worker = node.worker;
    let baremetal = node.name.contains("baremetal");
    let degraded = node.name.contains("wk-fra1-02");
    let mut disks = Vec::new();
    if baremetal {
        disks.push(disk(
            "nvme0n1",
            1_000_204_886_016,
            "1.0 TB",
            "Samsung SSD 980 PRO 1TB",
            "S5GXNX0T",
            "nvme",
            false,
            false,
            false,
        ));
    } else if worker {
        disks.push(disk(
            "nvme0n1",
            512_110_190_592,
            "512 GB",
            "KXG60ZNV512G TOSHIBA",
            "Y9TS1021",
            "nvme",
            false,
            false,
            false,
        ));
        disks.push(disk(
            "sda",
            2_000_398_934_016,
            "2.0 TB",
            "ST2000NM0033-9ZM",
            "Z1X0AB12",
            "sata",
            true,
            false,
            false,
        ));
    } else {
        disks.push(disk(
            "vda",
            128_849_018_880,
            "129 GB",
            "QEMU HARDDISK",
            "drive-virtio0",
            "virtio",
            false,
            false,
            false,
        ));
    }
    if degraded {
        disks.push(disk(
            "sr0",
            1_073_741_312,
            "1.1 GB",
            "QEMU DVD-ROM",
            "QM00003",
            "sata",
            false,
            true,
            true,
        ));
    }
    let ephemeral = if degraded {
        volume("EPHEMERAL", "waiting", "", None, None, None)
    } else if worker {
        volume(
            "EPHEMERAL",
            "ready",
            "460 GB",
            Some("xfs"),
            Some("/var"),
            Some("luks2"),
        )
    } else {
        volume(
            "EPHEMERAL",
            "ready",
            "116 GB",
            Some("xfs"),
            Some("/var"),
            Some("luks2"),
        )
    };
    let mut volumes = vec![
        volume(
            "EFI",
            "ready",
            "105 MB",
            Some("vfat"),
            Some("/system/efi"),
            None,
        ),
        volume("META", "ready", "1.0 MB", None, None, None),
        volume(
            "STATE",
            "ready",
            "105 MB",
            Some("xfs"),
            Some("/system/state"),
            Some("luks2"),
        ),
        ephemeral,
    ];
    if worker {
        volumes.push(volume(
            "u-data",
            "ready",
            "2.0 TB",
            Some("xfs"),
            Some("/var/mnt/data"),
            None,
        ));
    }
    let volumes = if baremetal {
        Err("Example: the volume status query timed out after 12 s".into())
    } else {
        Ok(volumes)
    };
    (disks, volumes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baremetal_has_its_nvme_disk_and_timed_out_volumes() {
        let (disks, volumes) = storage(&ExampleNode::new("example-baremetal", true, None));
        assert_eq!(disks.len(), 1);
        let disk = &disks[0];
        assert_eq!(disk.id, "nvme0n1");
        assert_eq!(disk.dev_path, "/dev/nvme0n1");
        assert_eq!(disk.size, 1_000_204_886_016);
        assert_eq!(disk.size_pretty, "1.0 TB");
        assert_eq!(disk.model.as_deref(), Some("Samsung SSD 980 PRO 1TB"));
        assert_eq!(disk.serial.as_deref(), Some("S5GXNX0T"));
        assert_eq!(disk.transport.as_deref(), Some("nvme"));
        assert!(!disk.rotational && !disk.readonly && !disk.cdrom);
        assert_eq!(disk.wwid.as_deref(), Some("eui.0025385S5GXNX0T00a1b2c3d4"));
        assert_eq!(
            disk.bus_path.as_deref(),
            Some("/pci0000:00/0000:00:1f.2/ata1/host0/target0:0:0/nvme0n1")
        );
        assert_eq!(
            volumes.unwrap_err(),
            "Example: the volume status query timed out after 12 s"
        );
    }

    #[test]
    fn degraded_worker_has_an_optical_disk_and_a_waiting_ephemeral_volume() {
        let (disks, volumes) = storage(&ExampleNode::new("example-wk-fra1-02", false, None));
        assert_eq!(
            disks
                .iter()
                .map(|disk| disk.id.as_str())
                .collect::<Vec<_>>(),
            ["nvme0n1", "sda", "sr0"]
        );
        let optical = &disks[2];
        assert_eq!(optical.size, 1_073_741_312);
        assert_eq!(optical.model.as_deref(), Some("QEMU DVD-ROM"));
        assert!(!optical.rotational && optical.readonly && optical.cdrom);
        let volumes = volumes.unwrap();
        assert_eq!(volumes.len(), 5);
        let waiting = volumes
            .iter()
            .find(|volume| volume.id == "EPHEMERAL")
            .unwrap();
        assert_eq!(waiting.phase, "waiting");
        assert_eq!(waiting.size, "");
        assert_eq!(waiting.filesystem, None);
        assert_eq!(waiting.mount_location, None);
        assert_eq!(waiting.encryption_provider, None);
        assert!(
            volumes
                .iter()
                .filter(|volume| volume.id != "EPHEMERAL")
                .all(|volume| volume.phase == "ready")
        );
    }

    #[test]
    fn control_plane_and_worker_keep_their_disks_and_only_worker_has_data_volume() {
        let (control_disks, control_volumes) =
            storage(&ExampleNode::new("example-control", true, None));
        let (worker_disks, worker_volumes) =
            storage(&ExampleNode::new("example-worker", false, None));
        assert_eq!(control_disks.len(), 1);
        assert_eq!(control_disks[0].id, "vda");
        assert_eq!(control_disks[0].size, 128_849_018_880);
        assert_eq!(control_disks[0].transport.as_deref(), Some("virtio"));
        assert_eq!(worker_disks.len(), 2);
        assert_eq!(worker_disks[0].size, 512_110_190_592);
        assert_eq!(worker_disks[1].size, 2_000_398_934_016);
        assert!(worker_disks[1].rotational);
        let control = control_volumes.unwrap();
        let worker = worker_volumes.unwrap();
        assert_eq!(
            control
                .iter()
                .map(|volume| volume.id.as_str())
                .collect::<Vec<_>>(),
            ["EFI", "META", "STATE", "EPHEMERAL"]
        );
        assert_eq!(
            worker
                .iter()
                .map(|volume| volume.id.as_str())
                .collect::<Vec<_>>(),
            ["EFI", "META", "STATE", "EPHEMERAL", "u-data"]
        );
        assert_eq!(control[3].size, "116 GB");
        assert_eq!(worker[3].size, "460 GB");
        for ephemeral in [&control[3], &worker[3]] {
            assert_eq!(ephemeral.phase, "ready");
            assert_eq!(ephemeral.filesystem.as_deref(), Some("xfs"));
            assert_eq!(ephemeral.mount_location.as_deref(), Some("/var"));
            assert_eq!(ephemeral.encryption_provider.as_deref(), Some("luks2"));
        }
        let data = &worker[4];
        assert_eq!(data.phase, "ready");
        assert_eq!(data.size, "2.0 TB");
        assert_eq!(data.filesystem.as_deref(), Some("xfs"));
        assert_eq!(data.mount_location.as_deref(), Some("/var/mnt/data"));
        assert_eq!(data.encryption_provider, None);
    }

    #[test]
    fn unknown_role_is_not_a_worker() {
        let node = ExampleNode::new("example-unknown", false, None).with_storage(false);
        let (disks, volumes) = storage(&node);
        assert_eq!(disks.len(), 1);
        assert_eq!(disks[0].id, "vda");
        assert!(!volumes.unwrap().iter().any(|volume| volume.id == "u-data"));
    }
}
