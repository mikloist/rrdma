use std::{thread, time::Duration};

fn main() -> anyhow::Result<()> {
    let [_, iface, listen] = std::env::args()
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| anyhow::anyhow!("usage: hello_b <iface> <listen addr>"))?;
    let mut nic = rdma::Nic::open(&iface)?;
    let region = nic.register()?;
    nic.mem(region)[5..11].copy_from_slice(b" world");
    println!("listening on {listen}, region {region}");
    rdma::publish(listen.parse()?, region, rdma::SLOT_SIZE as u32)?;
    println!("published, going idle");
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
