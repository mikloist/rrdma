//! Wire format and constants shared by the XDP "NIC" and the userspace lib.
#![cfg_attr(not(test), no_std)]

/// UDP port the XDP NIC serves.
pub const PORT: u16 = 18515;
/// Bytes per registered region (one `REGIONS` map entry).
pub const SLOT_SIZE: usize = 4096;
/// Number of regions (`REGIONS` max entries).
pub const NUM_SLOTS: u32 = 16;
/// Largest READ/WRITE payload; must fit in one packet.
pub const MAX_MSG: usize = 1024;
pub const HDR_LEN: usize = 20;

pub const OP_WRITE: u8 = 1;
pub const OP_WRITE_ACK: u8 = 2;
pub const OP_READ: u8 = 3;
pub const OP_READ_RESP: u8 = 4;

pub const STATUS_OK: u8 = 0;
pub const STATUS_ERR: u8 = 1;

/// Request/response header, big-endian on the wire, right after the UDP header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub op: u8,
    pub status: u8,
    pub len: u16,
    pub region: u32,
    pub offset: u32,
    pub wr_id: u64,
}

impl Header {
    pub fn encode(&self) -> [u8; HDR_LEN] {
        let mut b = [0; HDR_LEN];
        b[0] = self.op;
        b[1] = self.status;
        b[2..4].copy_from_slice(&self.len.to_be_bytes());
        b[4..8].copy_from_slice(&self.region.to_be_bytes());
        b[8..12].copy_from_slice(&self.offset.to_be_bytes());
        b[12..20].copy_from_slice(&self.wr_id.to_be_bytes());
        b
    }

    pub fn decode(b: &[u8; HDR_LEN]) -> Self {
        Self {
            op: b[0],
            status: b[1],
            len: u16::from_be_bytes([b[2], b[3]]),
            region: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            offset: u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            wr_id: u64::from_be_bytes([b[12], b[13], b[14], b[15], b[16], b[17], b[18], b[19]]),
        }
    }
}

#[inline(always)]
pub fn remote_ok(region: u32, offset: u32, len: u16) -> bool {
    let len = len as usize;
    region < NUM_SLOTS && offset as usize <= SLOT_SIZE - MAX_MSG && len > 0 && len <= MAX_MSG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_is_big_endian() {
        let h = Header {
            op: OP_READ,
            status: STATUS_OK,
            len: 0x0102,
            region: 3,
            offset: 0x0a0b0c0d,
            wr_id: 7,
        };
        let b = h.encode();
        assert_eq!(&b[2..4], &[1, 2]);
        assert_eq!(&b[8..12], &[0x0a, 0x0b, 0x0c, 0x0d]);
        assert_eq!(Header::decode(&b), h);
    }

    #[test]
    fn remote_bounds() {
        assert!(remote_ok(
            NUM_SLOTS - 1,
            (SLOT_SIZE - MAX_MSG) as u32,
            MAX_MSG as u16
        ));
        assert!(!remote_ok(NUM_SLOTS, 0, 1));
        assert!(!remote_ok(0, (SLOT_SIZE - MAX_MSG) as u32 + 1, 1));
        assert!(!remote_ok(0, 0, 0));
        assert!(!remote_ok(0, 0, MAX_MSG as u16 + 1));
    }
}
