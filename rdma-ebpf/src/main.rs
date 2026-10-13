//! The "NIC": serves one-sided READ/WRITE requests straight out of `REGIONS`,
//! without the receiving application being involved.
#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::{
        BPF_F_MMAPABLE,
        xdp_action::{XDP_DROP, XDP_PASS, XDP_TX},
    },
    helpers::{bpf_xdp_load_bytes, bpf_xdp_store_bytes},
    macros::{map, xdp},
    maps::Array,
    programs::XdpContext,
};
use rdma_common::*;

const IP_OFF: usize = 14;
const UDP_OFF: usize = IP_OFF + 20;
const HDR_OFF: usize = UDP_OFF + 8;
const DATA_OFF: usize = HDR_OFF + HDR_LEN;

/// Registered memory: entry `i` is region `i`. Userspace mmaps the whole array.
#[map]
static REGIONS: Array<[u8; SLOT_SIZE]> = Array::with_max_entries(NUM_SLOTS, BPF_F_MMAPABLE);

#[xdp]
pub fn rdma(ctx: XdpContext) -> u32 {
    let Some(p) = headers(&ctx) else {
        return XDP_PASS;
    };
    // Ethernet → IPv4 without options → UDP to our port; everything else goes to the stack.
    let ours = unsafe {
        *p.add(12) == 0x08
            && *p.add(13) == 0x00
            && *p.add(IP_OFF) == 0x45
            && *p.add(IP_OFF + 9) == 17
            && u16::from_be_bytes([*p.add(UDP_OFF + 2), *p.add(UDP_OFF + 3)]) == PORT
    };
    if !ours {
        return XDP_PASS;
    }
    let h = Header::decode(unsafe { &*(p.add(HDR_OFF) as *const [u8; HDR_LEN]) });
    let reply = match h.op {
        OP_WRITE => OP_WRITE_ACK,
        OP_READ => OP_READ_RESP,
        _ => return XDP_DROP,
    };
    let status = if access(&ctx, &h) {
        STATUS_OK
    } else {
        STATUS_ERR
    };

    // Turn the request into the response and bounce it back. Lengths are unchanged
    // (WRITE_ACK echoes the payload, READ came padded), so the IPv4 checksum stays
    // valid after swapping addresses.
    let Some(p) = headers(&ctx) else {
        return XDP_DROP;
    };
    unsafe {
        *p.add(HDR_OFF) = reply;
        *p.add(HDR_OFF + 1) = status;
        swap(p, 0, 6, 6); // MACs
        swap(p, IP_OFF + 12, IP_OFF + 16, 4); // IPs
        swap(p, UDP_OFF, UDP_OFF + 2, 2); // ports
        *p.add(UDP_OFF + 6) = 0; // UDP checksum: 0 = none (IPv4)
        *p.add(UDP_OFF + 7) = 0;
    }
    return XDP_TX;
}

/// Pointer to the packet start if all headers up to the payload are present.
#[inline(always)]
fn headers(ctx: &XdpContext) -> Option<*mut u8> {
    (ctx.data() + DATA_OFF <= ctx.data_end()).then_some(ctx.data() as *mut u8)
}

/// Copies payload ↔ region memory. The "DMA".
#[inline(always)]
fn access(ctx: &XdpContext, h: &Header) -> bool {
    if !remote_ok(h.region, h.offset, h.len) {
        return false;
    }
    let Some(slot) = REGIONS.get_ptr_mut(h.region) else {
        return false;
    };
    let mem = unsafe { (slot as *mut u8).add(h.offset as usize) }.cast();
    let len = core::hint::black_box(h.len as u32).clamp(1, MAX_MSG as u32);
    let r = unsafe {
        if h.op == OP_WRITE {
            bpf_xdp_load_bytes(ctx.ctx, DATA_OFF as u32, mem, len)
        } else {
            bpf_xdp_store_bytes(ctx.ctx, DATA_OFF as u32, mem, len)
        }
    };
    r == 0
}

#[inline(always)]
fn swap(p: *mut u8, a: usize, b: usize, n: usize) {
    for i in 0..n {
        unsafe { core::ptr::swap(p.add(a + i), p.add(b + i)) };
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
