//! Wire format and constants shared by the XDP "NIC" and the userspace lib.
#![cfg_attr(not(test), no_std)]

pub use zerocopy::{FromBytes, IntoBytes};
use zerocopy::{
    Immutable, KnownLayout, Unaligned,
    network_endian::{U16, U32, U64},
};

/// UDP port the XDP NIC serves.
pub const PORT: u16 = 18515;
/// Bytes per registered region (one `REGIONS` map entry).
pub const SLOT_SIZE: usize = 4096;
/// Number of regions (`REGIONS` max entries).
pub const NUM_SLOTS: u32 = 16;
/// Largest READ/WRITE payload; must fit in one packet.
pub const MAX_MSG: usize = 1024;
pub const HDR_LEN: usize = core::mem::size_of::<Header>();

pub const OP_WRITE: u8 = 1;
pub const OP_WRITE_ACK: u8 = 2;
pub const OP_READ: u8 = 3;
pub const OP_READ_RESP: u8 = 4;

pub const STATUS_OK: u8 = 0;
pub const STATUS_ERR: u8 = 1;

/// Request/response header, right after the UDP header. The struct *is* the wire
/// layout: fields in order, no padding, multi-byte fields big-endian.
#[derive(
    FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy, Debug, PartialEq, Eq,
)]
#[repr(C)]
pub struct Header {
    pub op: u8,
    pub status: u8,
    pub len: U16,
    pub region: U32,
    pub offset: U32,
    pub wr_id: U64,
}

impl Header {
    /// Remote access the NIC accepts. Offset and length are bounded separately so the
    /// verifier can prove `slot + offset .. + len` stays inside the slot.
    #[inline(always)]
    pub fn remote_ok(&self) -> bool {
        let len = self.len.get() as usize;
        self.region.get() < NUM_SLOTS
            && self.offset.get() as usize <= SLOT_SIZE - MAX_MSG
            && len > 0
            && len <= MAX_MSG
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(region: u32, offset: u32, len: u16) -> Header {
        Header {
            op: OP_READ,
            status: STATUS_OK,
            len: len.into(),
            region: region.into(),
            offset: offset.into(),
            wr_id: 7.into(),
        }
    }

    #[test]
    fn wire_layout_is_packed_big_endian() {
        let h = hdr(3, 0x0a0b0c0d, 0x0102);
        let b = h.as_bytes();
        assert_eq!(b.len(), 20);
        assert_eq!(&b[2..4], &[1, 2]);
        assert_eq!(&b[8..12], &[0x0a, 0x0b, 0x0c, 0x0d]);
        assert_eq!(b[19], 7);
        assert_eq!(Header::read_from_bytes(b).unwrap(), h);
    }

    #[test]
    fn remote_bounds() {
        let max_off = (SLOT_SIZE - MAX_MSG) as u32;
        assert!(hdr(NUM_SLOTS - 1, max_off, MAX_MSG as u16).remote_ok());
        assert!(!hdr(NUM_SLOTS, 0, 1).remote_ok());
        assert!(!hdr(0, max_off + 1, 1).remote_ok());
        assert!(!hdr(0, 0, 0).remote_ok());
        assert!(!hdr(0, 0, MAX_MSG as u16 + 1).remote_ok());
    }
}
