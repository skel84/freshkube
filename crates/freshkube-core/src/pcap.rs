//! Classic-PCAP framing and the bounded capture buffer, shared by the
//! frontends that save packet captures.
//!
//! A capture stream arrives as arbitrary transport chunks. [`PcapFraming`]
//! validates the global header and record headers incrementally, and
//! [`CaptureState`] keeps a capped buffer in which only the validated header
//! and fully received records are saveable.

#[derive(Clone, Debug, Default, PartialEq)]
pub enum SaveState {
    #[default]
    Idle,
    Saving,
    Saved(String),
    Cancelled,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcapEndian {
    Little,
    Big,
}

impl PcapEndian {
    pub fn u16(self, bytes: &[u8]) -> u16 {
        let bytes = [bytes[0], bytes[1]];
        match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        }
    }
    pub fn u32(self, bytes: &[u8]) -> u32 {
        let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }
}

/// Incremental classic-PCAP framing. The backing buffer is independently capped;
/// only the validated global header and fully received records are saveable.
#[derive(Default)]
pub struct PcapFraming {
    pub endian: Option<PcapEndian>,
    pub nanoseconds: bool,
    pub snap_len: u32,
    pub complete: usize,
    pub records: usize,
}

impl PcapFraming {
    pub fn advance(&mut self, bytes: &[u8]) -> Result<(), String> {
        if self.endian.is_none() {
            if bytes.len() < 4 {
                return Ok(());
            }
            let (endian, nanoseconds) = match bytes[..4] {
                [0xd4, 0xc3, 0xb2, 0xa1] => (PcapEndian::Little, false),
                [0xa1, 0xb2, 0xc3, 0xd4] => (PcapEndian::Big, false),
                [0x4d, 0x3c, 0xb2, 0xa1] => (PcapEndian::Little, true),
                [0xa1, 0xb2, 0x3c, 0x4d] => (PcapEndian::Big, true),
                _ => return Err("Invalid classic-PCAP magic (PCAPNG is not supported)".into()),
            };
            if bytes.len() < 24 {
                return Ok(());
            }
            if endian.u16(&bytes[4..6]) != 2 || endian.u16(&bytes[6..8]) != 4 {
                return Err("Unsupported PCAP version; expected 2.4".into());
            }
            let snap_len = endian.u32(&bytes[16..20]);
            if snap_len == 0 {
                return Err("Invalid PCAP snap length of zero".into());
            }
            self.endian = Some(endian);
            self.nanoseconds = nanoseconds;
            self.snap_len = snap_len;
            self.complete = 24;
        }
        let endian = self.endian.expect("validated header");
        while bytes.len().saturating_sub(self.complete) >= 16 {
            let header = &bytes[self.complete..self.complete + 16];
            let fraction = endian.u32(&header[4..8]);
            let included = endian.u32(&header[8..12]);
            let original = endian.u32(&header[12..16]);
            if fraction
                >= if self.nanoseconds {
                    1_000_000_000
                } else {
                    1_000_000
                }
            {
                return Err("Invalid PCAP timestamp fraction".into());
            }
            if included > self.snap_len || included > original {
                return Err("Invalid PCAP record length (exceeds snap/original length)".into());
            }
            let end = self
                .complete
                .checked_add(16)
                .and_then(|start| start.checked_add(included as usize))
                .ok_or_else(|| "PCAP record length overflow".to_string())?;
            if end > bytes.len() {
                break;
            }
            self.complete = end;
            self.records += 1;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct CaptureState {
    pub bytes: Vec<u8>,
    pub framing: PcapFraming,
    pub discarded: usize,
    pub max_bytes: usize,
    pub active: bool,
    pub status: String,
    pub save: SaveState,
    pub generation: u64,
}

impl CaptureState {
    pub fn start(&mut self, max_bytes: usize) -> Result<(), String> {
        if self.active || self.save == SaveState::Saving {
            return Err("Capture/save already running".into());
        }
        if max_bytes < 24 {
            return Err("Capture limit must fit the 24-byte PCAP header".into());
        }
        self.generation = self.generation.wrapping_add(1);
        self.bytes = Vec::new();
        self.max_bytes = max_bytes;
        self.active = true;
        self.framing = PcapFraming::default();
        self.discarded = 0;
        self.status = "Opening packet capture…".into();
        self.save = SaveState::Idle;
        Ok(())
    }
    pub fn append(&mut self, chunk: &[u8]) -> bool {
        if !self.active {
            return false;
        }
        let retained = chunk
            .len()
            .min(self.max_bytes.saturating_sub(self.bytes.len()));
        // Keep buffer capacity bounded too, while avoiding a reallocation for
        // every split transport fragment.
        let required = self.bytes.len().saturating_add(retained);
        if required > self.bytes.capacity() {
            let capacity = required
                .max(self.bytes.capacity().saturating_mul(2))
                .min(self.max_bytes);
            self.bytes.reserve_exact(capacity - self.bytes.len());
        }
        self.bytes.extend_from_slice(&chunk[..retained]);
        self.discarded = self.discarded.saturating_add(chunk.len() - retained);
        if let Err(error) = self.framing.advance(&self.bytes) {
            self.stop(format!("Capture failed: {error}"));
            return false;
        }
        if retained < chunk.len() || self.bytes.len() == self.max_bytes {
            self.stop("Stopped at byte cap".into());
            return false;
        }
        self.status = format!(
            "Capturing: {} bytes, {} complete packets",
            self.bytes.len(),
            self.framing.records
        );
        true
    }
    pub fn stop(&mut self, status: String) {
        self.active = false;
        self.discarded = self
            .discarded
            .saturating_add(self.bytes.len().saturating_sub(self.framing.complete));
        self.bytes.truncate(self.framing.complete);
        self.status = status;
        if self.framing.endian.is_none() {
            self.status
                .push_str("; no complete valid PCAP header — nothing saveable");
        }
        if self.discarded > 0 {
            self.status.push_str(&format!(
                "; discarded {} trailing/unretained bytes (only complete PCAP records saved)",
                self.discarded
            ));
        }
    }
    pub fn save_bytes(&self) -> Option<&[u8]> {
        if self.active || self.framing.endian.is_none() || self.framing.complete < 24 {
            return None;
        }
        self.bytes.get(..self.framing.complete)
    }
    pub fn can_save(&self) -> bool {
        self.save != SaveState::Saving && self.save_bytes().is_some()
    }
    pub fn finish_save(&mut self, generation: u64, outcome: SaveState) {
        if self.generation == generation {
            self.save = outcome;
        }
    }
}

/// Writes `value` into `bytes` (exactly two bytes) in `endian` byte order.
pub fn put16(bytes: &mut [u8], value: u16, endian: PcapEndian) {
    bytes.copy_from_slice(&match endian {
        PcapEndian::Little => value.to_le_bytes(),
        PcapEndian::Big => value.to_be_bytes(),
    });
}

/// Writes `value` into `bytes` (exactly four bytes) in `endian` byte order.
pub fn put32(bytes: &mut [u8], value: u32, endian: PcapEndian) {
    bytes.copy_from_slice(&match endian {
        PcapEndian::Little => value.to_le_bytes(),
        PcapEndian::Big => value.to_be_bytes(),
    });
}

/// A classic-PCAP global header (version 2.4, snap length 65535, Ethernet).
/// Used for synthetic captures and tests.
pub fn pcap_header(endian: PcapEndian, nanoseconds: bool) -> Vec<u8> {
    let mut bytes = vec![0u8; 24];
    put32(
        &mut bytes[..4],
        if nanoseconds { 0xa1b23c4d } else { 0xa1b2c3d4 },
        endian,
    );
    put16(&mut bytes[4..6], 2, endian);
    put16(&mut bytes[6..8], 4, endian);
    put32(&mut bytes[16..20], 65535, endian);
    put32(&mut bytes[20..24], 1, endian);
    bytes
}

/// One classic-PCAP record holding `payload`, with a fixed timestamp.
pub fn pcap_record(endian: PcapEndian, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; 16];
    put32(&mut bytes[..4], 42, endian);
    put32(&mut bytes[4..8], 123, endian);
    put32(&mut bytes[8..12], payload.len() as u32, endian);
    put32(&mut bytes[12..16], payload.len() as u32, endian);
    bytes.extend_from_slice(payload);
    bytes
}

#[cfg(test)]
mod tests {
    use super::{CaptureState, PcapEndian, pcap_header, pcap_record};

    #[test]
    fn pcap_split_framing_supports_both_endians_and_timestamp_resolutions() {
        for endian in [PcapEndian::Little, PcapEndian::Big] {
            for nanos in [false, true] {
                let mut fixture = pcap_header(endian, nanos);
                fixture.extend(pcap_record(endian, b"first packet"));
                fixture.extend(pcap_record(endian, b"second packet"));
                for split in 0..=fixture.len() {
                    let mut capture = CaptureState::default();
                    capture.start(fixture.len() + 10).unwrap();
                    assert!(capture.append(&fixture[..split]));
                    assert!(!capture.can_save());
                    assert!(capture.append(&fixture[split..]));
                    capture.stop("Stopped by user".into());
                    assert_eq!(capture.framing.records, 2);
                    assert_eq!(capture.save_bytes().unwrap(), fixture);
                    assert_eq!(capture.discarded, 0);
                }
                let mut capture = CaptureState::default();
                capture.start(1024).unwrap();
                for byte in &fixture {
                    assert!(capture.append(&[*byte]));
                }
                capture.stop("Stopped".into());
                assert_eq!(capture.save_bytes().unwrap(), fixture);
            }
        }
    }
}
