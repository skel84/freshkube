//! Example disks and volumes for `--fixture`.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn disk(
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
pub(super) fn example(source: &ScreenSource) -> Result<StorageData, String> {
    let Some(node) = source.node() else {
        return Err("Example data has no such node".into());
    };
    if !node.responding {
        return Err(format!(
            "{} didn't answer the Talos API within 10 s (example)",
            node.name
        ));
    }
    let worker = node.role == crate::presentation::Role::Worker;
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
    Ok(StorageData {
        disks: Ok(disks),
        volumes: if baremetal {
            Err("Example: the volume status query timed out after 12 s".into())
        } else {
            Ok(volumes)
        },
    })
}
