//! Userspace side: loads the XDP "NIC", exposes registered memory, posts
//! one-sided READ/WRITE requests and turns responses into completions.

use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    os::fd::{AsFd, AsRawFd},
    time::Duration,
};

use anyhow::{Context as _, bail, ensure};
use aya::{
    Ebpf,
    maps::Map,
    programs::{Xdp, XdpMode},
};
use rdma_common::*;
pub use rdma_common::{MAX_MSG, NUM_SLOTS, SLOT_SIZE};

/// Address in registered memory: region id (= slot index) + byte offset.
#[derive(Clone, Copy, Debug)]
pub struct Addr {
    pub region: u32,
    pub offset: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Completion {
    pub wr_id: u64,
    pub ok: bool,
}

pub struct Nic {
    /// Keeps the program attached and the map alive.
    _ebpf: Ebpf,
    mem: *mut u8,
    sock: UdpSocket,
    next_region: u32,
    /// Outstanding READs: wr_id → where the data lands locally.
    reads: HashMap<u64, (Addr, u16)>,
}

const MEM_LEN: usize = SLOT_SIZE * NUM_SLOTS as usize;

impl Nic {
    /// Loads the NIC program, attaches it to `iface` in generic XDP mode and maps `REGIONS`.
    pub fn open(iface: &str) -> anyhow::Result<Self> {
        let mut ebpf = Ebpf::load(aya::include_bytes_aligned!(concat!(
            env!("OUT_DIR"),
            "/rdma"
        )))?;
        let prog: &mut Xdp = ebpf
            .program_mut("rdma")
            .context("no rdma program")?
            .try_into()?;
        prog.load()?;
        prog.attach(iface, XdpMode::Skb)
            .with_context(|| format!("attach XDP to {iface}"))?;

        let Some(Map::Array(regions)) = ebpf.map("REGIONS") else {
            bail!("no REGIONS array")
        };
        let mem = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                MEM_LEN,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                regions.fd().as_fd().as_raw_fd(),
                0,
            )
        };
        if mem == libc::MAP_FAILED {
            return Err(io::Error::last_os_error()).context("mmap REGIONS");
        }

        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.set_read_timeout(Some(Duration::from_secs(1)))?;
        Ok(Self {
            _ebpf: ebpf,
            mem: mem.cast(),
            sock,
            next_region: 0,
            reads: HashMap::new(),
        })
    }

    /// Allocates the next free region; its memory is `mem(id)`.
    pub fn register(&mut self) -> anyhow::Result<u32> {
        ensure!(
            self.next_region < NUM_SLOTS,
            "all {NUM_SLOTS} regions in use"
        );
        self.next_region += 1;
        Ok(self.next_region - 1)
    }

    /// Local view of a region; the remote NIC reads/writes the same bytes.
    pub fn mem(&mut self, region: u32) -> &mut [u8] {
        assert!(region < NUM_SLOTS, "region {region} out of range");
        unsafe {
            std::slice::from_raw_parts_mut(self.mem.add(region as usize * SLOT_SIZE), SLOT_SIZE)
        }
    }

    /// Sets the peer whose NIC serves our requests.
    pub fn connect(&mut self, peer: IpAddr) -> io::Result<()> {
        self.sock.connect((peer, PORT))
    }

    /// Copies `len` bytes from local `local` to remote `remote`.
    pub fn post_write(
        &mut self,
        wr_id: u64,
        local: Addr,
        remote: Addr,
        len: u16,
    ) -> io::Result<()> {
        let o = local.offset as usize;
        let mut pkt = header(OP_WRITE, wr_id, remote, len).to_vec();
        pkt.extend_from_slice(&self.mem(local.region)[o..o + len as usize]);
        self.sock.send(&pkt).map(drop)
    }

    /// Copies `len` bytes from remote `remote` to local `local`.
    pub fn post_read(&mut self, wr_id: u64, local: Addr, remote: Addr, len: u16) -> io::Result<()> {
        // Padded to the response size so the remote NIC fills it in place.
        let mut pkt = header(OP_READ, wr_id, remote, len).to_vec();
        pkt.resize(HDR_LEN + len as usize, 0);
        self.sock.send(&pkt)?;
        self.reads.insert(wr_id, (local, len));
        Ok(())
    }

    /// Blocks for the next completion; times out after 1 s.
    pub fn poll_cq(&mut self) -> io::Result<Completion> {
        let mut buf = [0; HDR_LEN + MAX_MSG];
        loop {
            let n = self.sock.recv(&mut buf)?;
            let Some(h) = buf[..n].first_chunk().map(Header::decode) else {
                continue;
            };
            let mut ok = h.status == STATUS_OK;
            match h.op {
                OP_WRITE_ACK => {}
                OP_READ_RESP => {
                    let Some((local, len)) = self.reads.remove(&h.wr_id) else {
                        continue;
                    };
                    match buf[..n].get(HDR_LEN..HDR_LEN + len as usize) {
                        Some(data) if ok => {
                            let o = local.offset as usize;
                            self.mem(local.region)[o..o + data.len()].copy_from_slice(data);
                        }
                        _ => ok = false,
                    }
                }
                _ => continue,
            }
            return Ok(Completion { wr_id: h.wr_id, ok });
        }
    }
}

impl Drop for Nic {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.mem.cast(), MEM_LEN) };
    }
}

fn header(op: u8, wr_id: u64, remote: Addr, len: u16) -> [u8; HDR_LEN] {
    Header {
        op,
        status: STATUS_OK,
        len,
        region: remote.region,
        offset: remote.offset,
        wr_id,
    }
    .encode()
}

/// Out-of-band exchange: serves `(region, len)` to the first peer that connects.
pub fn publish(listen: SocketAddr, region: u32, len: u32) -> io::Result<()> {
    let (mut s, _) = TcpListener::bind(listen)?.accept()?;
    s.write_all(&[region.to_be_bytes(), len.to_be_bytes()].concat())
}

/// Fetches the `(region, len)` a peer published.
pub fn lookup(peer: SocketAddr) -> io::Result<(u32, u32)> {
    let mut b = [0; 8];
    TcpStream::connect(peer)?.read_exact(&mut b)?;
    Ok((
        u32::from_be_bytes(b[..4].try_into().unwrap()),
        u32::from_be_bytes(b[4..].try_into().unwrap()),
    ))
}
