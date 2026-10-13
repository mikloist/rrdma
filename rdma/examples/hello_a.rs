use anyhow::ensure;
use rdma::{Addr, Completion};

fn main() -> anyhow::Result<()> {
    let [_, iface, peer] = std::env::args()
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| anyhow::anyhow!("usage: hello_a <iface> <peer addr>"))?;
    let peer: std::net::SocketAddr = peer.parse()?;
    let mut nic = rdma::Nic::open(&iface)?;
    let (region, _len) = rdma::lookup(peer)?;
    nic.connect(peer.ip())?;
    let remote = Addr { region, offset: 0 };

    let local = nic.register()?;
    nic.mem(local)[..5].copy_from_slice(b"Hello");
    nic.post_write(
        1,
        Addr {
            region: local,
            offset: 0,
        },
        remote,
        5,
    )?;
    ensure!(
        nic.poll_cq()? == Completion { wr_id: 1, ok: true },
        "WRITE failed"
    );

    nic.post_read(
        2,
        Addr {
            region: local,
            offset: 100,
        },
        remote,
        11,
    )?;
    ensure!(
        nic.poll_cq()? == Completion { wr_id: 2, ok: true },
        "READ failed"
    );
    let got = String::from_utf8_lossy(&nic.mem(local)[100..111]).into_owned();
    ensure!(got == "Hello world", "read back {got:?}");
    println!("PASS: read back {got:?}");

    nic.mem(local)[200..212].copy_from_slice(b"Hello world!");
    nic.post_write(
        3,
        Addr {
            region: local,
            offset: 200,
        },
        remote,
        12,
    )?;
    ensure!(
        nic.poll_cq()? == Completion { wr_id: 3, ok: true },
        "WRITE failed"
    );

    nic.post_read(
        4,
        Addr {
            region: local,
            offset: 300,
        },
        remote,
        12,
    )?;
    ensure!(
        nic.poll_cq()? == Completion { wr_id: 4, ok: true },
        "READ failed"
    );
    let got = String::from_utf8_lossy(&nic.mem(local)[300..312]).into_owned();
    ensure!(got == "Hello world!", "read back {got:?}");
    println!("PASS: read back {got:?}");
    Ok(())
}
