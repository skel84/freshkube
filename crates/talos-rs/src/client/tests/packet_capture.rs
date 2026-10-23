use super::*;

#[tokio::test]
async fn packet_capture_preserves_chunks_and_applies_backpressure() {
    let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = polls.clone();
    let chunks = futures::stream::iter(vec![
        Ok(crate::proto::common::Data {
            metadata: None,
            bytes: b"pcap header".to_vec(),
        }),
        Ok(crate::proto::common::Data {
            metadata: None,
            bytes: vec![0, 255, 128],
        }),
    ])
    .map(move |chunk| {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        chunk
    });
    let stream = packet_capture_chunks(chunks);
    futures::pin_mut!(stream);
    assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(stream.next().await.unwrap().unwrap(), b"pcap header");
    assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(stream.next().await.unwrap().unwrap(), vec![0, 255, 128]);
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn packet_capture_surfaces_proxy_metadata_errors_and_stops() {
    for (error, code, message) in [
        (
            "capture denied",
            Some(7),
            "capture denied (upstream status 7: permission denied)",
        ),
        ("", Some(7), "upstream status 7: permission denied"),
        ("capture denied", None, "capture denied"),
    ] {
        let stream = packet_capture_chunks(futures::stream::iter(vec![
            Ok(crate::proto::common::Data {
                metadata: Some(crate::proto::common::Metadata {
                    hostname: "fixture-node".into(),
                    error: error.into(),
                    status: code.map(|code| crate::proto::google::rpc::Status {
                        code,
                        message: "permission denied".into(),
                        details: vec![],
                    }),
                }),
                bytes: b"undefined bytes must not be saved".to_vec(),
            }),
            Ok(crate::proto::common::Data {
                metadata: None,
                bytes: b"must not be consumed".to_vec(),
            }),
        ]));
        futures::pin_mut!(stream);
        let TalosError::Grpc(status) = stream.next().await.unwrap().unwrap_err() else {
            panic!("expected proxied gRPC status");
        };
        assert_eq!(
            status.code(),
            if code.is_some() {
                tonic::Code::PermissionDenied
            } else {
                tonic::Code::Unknown
            }
        );
        assert_eq!(status.message(), message);
        assert!(stream.next().await.is_none());
    }
}

#[tokio::test]
async fn packet_capture_surfaces_transport_errors_and_stops() {
    let stream = packet_capture_chunks(futures::stream::iter(vec![
        Err(tonic::Status::unavailable("capture disconnected")),
        Ok(crate::proto::common::Data {
            metadata: None,
            bytes: vec![1],
        }),
    ]));
    futures::pin_mut!(stream);
    let TalosError::Grpc(status) = stream.next().await.unwrap().unwrap_err() else {
        panic!("expected transport status");
    };
    assert_eq!(status.code(), tonic::Code::Unavailable);
    assert_eq!(status.message(), "capture disconnected");
    assert!(stream.next().await.is_none());
}

struct IdleCaptureSource(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl futures::Stream for IdleCaptureSource {
    type Item = Result<crate::proto::common::Data, tonic::Status>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}

impl Drop for IdleCaptureSource {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[tokio::test]
async fn packet_capture_drop_retires_idle_transport_immediately() {
    for poll_first in [false, true] {
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut stream = Box::pin(packet_capture_chunks(IdleCaptureSource(dropped.clone())));
        if poll_first {
            assert!(futures::poll!(futures::StreamExt::next(&mut stream)).is_pending());
        }
        assert!(!dropped.load(std::sync::atomic::Ordering::SeqCst));
        drop(stream);
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }
}

/// Minimal classic-BPF interpreter for the existing port exclusion filters.
fn capture_filter_accepts(filter: &[crate::proto::machine::BpfInstruction], packet: &[u8]) -> bool {
    let (mut accumulator, mut index, mut pc) = (0u32, 0usize, 0usize);
    for _ in 0..128 {
        let instruction = &filter[pc];
        match instruction.op {
            0x28 | 0x48 => {
                let offset =
                    instruction.k as usize + if instruction.op == 0x48 { index } else { 0 };
                let Some(bytes) = packet.get(offset..offset + 2) else {
                    return false;
                };
                accumulator = u16::from_be_bytes([bytes[0], bytes[1]]) as u32;
            }
            0x30 | 0xb1 => {
                let Some(byte) = packet.get(instruction.k as usize) else {
                    return false;
                };
                if instruction.op == 0xb1 {
                    index = ((byte & 0xf) * 4) as usize;
                } else {
                    accumulator = *byte as u32;
                }
            }
            0x54 => accumulator &= instruction.k,
            0x15 | 0x45 => {
                let matches = if instruction.op == 0x15 {
                    accumulator == instruction.k
                } else {
                    accumulator & instruction.k != 0
                };
                pc += if matches {
                    instruction.jt
                } else {
                    instruction.jf
                } as usize;
            }
            0x06 => return instruction.k != 0,
            other => panic!("unsupported BPF opcode {other:#x}"),
        }
        pc += 1;
        assert!(pc < filter.len(), "BPF jump outside program");
    }
    panic!("BPF program did not terminate");
}

fn capture_packet_fixture(
    ethernet: bool,
    ipv6: bool,
    protocol: u8,
    source_port: u16,
    destination_port: u16,
) -> Vec<u8> {
    let ip = if ethernet { 14 } else { 0 };
    let header = if ipv6 { 40 } else { 20 };
    let mut packet = vec![0u8; ip + header + 20];
    if ethernet {
        let ether_type: u16 = if ipv6 { 0x86dd } else { 0x0800 };
        packet[12..14].copy_from_slice(&ether_type.to_be_bytes());
    }
    packet[ip] = if ipv6 { 0x60 } else { 0x45 };
    packet[ip + if ipv6 { 6 } else { 9 }] = protocol;
    packet[ip + header..ip + header + 2].copy_from_slice(&source_port.to_be_bytes());
    packet[ip + header + 2..ip + header + 4].copy_from_slice(&destination_port.to_be_bytes());
    packet
}

#[test]
fn packet_capture_api_filter_excludes_management_traffic_for_both_link_types() {
    for interface in ["eth0", "ens1", "lo", "kubespan", "wg0", "tun0"] {
        let ethernet = TalosClient::detect_link_type(interface) == LinkType::EN10MB;
        let filter = TalosClient::packet_capture_api_exclusion_filter(interface);
        for ipv6 in [false, true] {
            for protocol in [6, 17, 132] {
                for (source, destination, expected) in [
                    (50000, 1234, false),
                    (1234, 50000, false),
                    (1234, 6443, true),
                ] {
                    let packet =
                        capture_packet_fixture(ethernet, ipv6, protocol, source, destination);
                    assert_eq!(
                        capture_filter_accepts(&filter, &packet),
                        expected,
                        "{interface}, ipv6={ipv6}, protocol={protocol}, {source}->{destination}"
                    );
                }
            }
        }
    }
}

/// The filters read ports only from a header that follows the IP
/// header directly, as tcpdump's `not port` does, and keep what they
/// can't place. A later IPv4 fragment carries no TCP or UDP header, so
/// it is kept, management traffic's too, and its payload is never read
/// as ports; the first, which has them, is dropped. An IPv6 packet whose
/// next header is Fragment (44) is kept whether first or later, since
/// its ports, when it has them, sit behind the fragment header.
#[test]
fn packet_capture_api_filter_keeps_fragments_whose_ports_it_cannot_read() {
    for interface in ["eth0", "kubespan"] {
        let ethernet = TalosClient::detect_link_type(interface) == LinkType::EN10MB;
        let filter = TalosClient::packet_capture_api_exclusion_filter(interface);
        let ip = if ethernet { 14 } else { 0 };
        for protocol in [6, 17, 132] {
            // IPv4: the payload where a header would be reads as port
            // 50000 both ways. Offset 185 (1480 bytes in), more to come.
            let mut later = capture_packet_fixture(ethernet, false, protocol, 50000, 50000);
            later[ip + 6..ip + 8].copy_from_slice(&0x20b9u16.to_be_bytes());
            assert!(
                capture_filter_accepts(&filter, &later),
                "{interface}, IPv4, protocol={protocol}: a later fragment is kept"
            );
            // Offset 0, more to come: the ports are there.
            let mut first = capture_packet_fixture(ethernet, false, protocol, 1234, 50000);
            first[ip + 6..ip + 8].copy_from_slice(&0x2000u16.to_be_bytes());
            assert!(
                !capture_filter_accepts(&filter, &first),
                "{interface}, IPv4, protocol={protocol}: the first fragment is dropped"
            );

            // IPv6: a fragment header, then the protocol's header with
            // port 50000 both ways, first (offset 0) and later (185).
            for (offset, which) in [(0x0001u16, "first"), (0x05c9, "later")] {
                let mut packet = capture_packet_fixture(ethernet, true, 44, 0, 0);
                let fragment = ip + 40;
                packet[fragment] = protocol;
                packet[fragment + 2..fragment + 4].copy_from_slice(&offset.to_be_bytes());
                packet[fragment + 8..fragment + 10].copy_from_slice(&50000u16.to_be_bytes());
                packet[fragment + 10..fragment + 12].copy_from_slice(&50000u16.to_be_bytes());
                assert!(
                    capture_filter_accepts(&filter, &packet),
                    "{interface}, IPv6, protocol={protocol}: the {which} fragment is kept"
                );
            }
        }
    }
}
