# small rust remote dma lib

Small RDMA lib for me to understand how RDMA works, what the basics are, how everything fits together.

# AI usage
I used AI for this, especially for discovery work, as typing substitute and for testing harness. However basic design and code structure is my idea.

# Model
eBPF (XDP) plays the NIC. Remote memory is written/read by XDP directly, without the remote app or CPU being involved.

- Memory regions = fixed-size slots in one `BPF_MAP_TYPE_ARRAY` with `BPF_F_MMAPABLE`; app `mmap`s it, XDP accesses it.
- Remote address = (region id, offset), not a virtual address (we skip NIC VA translation).
- Transport: UDP. Userspace sends requests; XDP on the receiver handles them:
    - WRITE: copy payload into region.
    - READ: rewrite packet with region data, `XDP_TX` back to requester.
- Max message size: one packet, capped length so the verifier can bound copies.

# Features
- Each side allocates and registers its own memory (region id, len), no auth
    - All regions live in one mmapable `BPF_MAP_TYPE_ARRAY`, split into fixed-size slots
    - Region id = slot index; remote address = `id * SLOT_SIZE + offset`, XDP checks `offset + len <= SLOT_SIZE`
    - Region id doubles as a fake `rkey` (guessable, no protection; shows where the key sits on the wire)
- Region info exchanged with the peer over a plain TCP handshake
- One-sided READ/WRITE only (no two-sided SEND/RECV) + completion queue
- Completions posted by userspace when the ACK / READ response arrives
- Read/Write:
    - Write: source local address, length, destination remote (region id, offset)
    - Read: destination local address, length, source remote (region id, offset)
- Reliability: no retransmits; WRITE completes only when ACKed
- Wire format: own header
- Tooling: `aya`
- XDP mode: generic XDP

# Testing
- Two QEMU VMs linked by a virtual network
- App B: registers region with " world" hardcoded at offset 5, publishes region info, then stays passive
- App A: WRITEs "Hello" at offset 0 of B's region, then READs "Hello world" back
