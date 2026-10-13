#!/usr/bin/env bash
# Boots two QEMU VMs (host kernel + tiny initramfs) linked by a QEMU socket network
# and runs hello_b (10.99.0.2) / hello_a (10.99.0.1). Runs unprivileged; needs /dev/kvm.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --examples
kver=$(uname -r)
out=target/vm
rm -rf "$out" && mkdir -p "$out/root"/{bin,lib,lib64,proc,sys,dev}
root=$out/root

# Host binaries + their shared libs.
for bin in bash ip mount insmod sleep target/debug/examples/hello_a target/debug/examples/hello_b; do
    path=$(command -v "$bin" || echo "$bin")
    cp "$path" "$root/bin/"
    for lib in $(ldd "$path" | grep -o '/[^ ]*'); do
        mkdir -p "$root$(dirname "$lib")" && cp -n "$lib" "$root$lib"
    done
done
for mod in failover net_failover virtio_net; do
    xz -dc "$(modinfo -k "$kver" -n "$mod")" > "$root/lib/$mod.ko"
done

cat > "$root/init" <<'EOF'
#!/bin/bash
export PATH=/bin
mount -t devtmpfs dev /dev
exec </dev/console >/dev/console 2>&1
mount -t proc proc /proc
mount -t sysfs sys /sys
for m in failover net_failover virtio_net; do insmod /lib/$m.ko; done
ip link set lo up
ip link set eth0 up
ip addr add "$addr/24" dev eth0
if [ "$role" = b ]; then
    hello_b eth0 0.0.0.0:7000
else
    hello_a eth0 10.99.0.2:7000 && echo "RESULT: ok" || echo "RESULT: fail"
fi
# Power off instead of exiting: init exiting is a kernel panic.
echo o > /proc/sysrq-trigger
while :; do sleep 1; done
EOF
chmod +x "$root/init"
(cd "$root" && find . | cpio -o -H newc --quiet) > "$out/initramfs.cpio"

# Serial console goes straight to the log file: with -nographic, QEMU stays silent
# when stdin is a terminal but stdout is not (e.g. `| tee`).
vm() { # role addr mac netdev-opt
    exec timeout 60 qemu-system-x86_64 -enable-kvm -cpu host -m 512 -no-reboot \
        -display none -monitor none -serial "file:$out/$1.log" \
        -kernel "/boot/vmlinuz-$kver" -initrd "$out/initramfs.cpio" \
        -append "console=ttyS0 quiet panic=-1 rdinit=/init role=$1 addr=$2" \
        -netdev "socket,id=n,$4" -device "virtio-net-pci,netdev=n,mac=$3"
}

: > "$out/b.log"
vm b 10.99.0.2 52:54:00:00:00:02 listen=127.0.0.1:18600 &
b=$!
# B idles forever by design; it is powered off (killed) once A is done.
trap 'kill $b 2>/dev/null || true' EXIT
until grep -q "listening" "$out/b.log"; do
    kill -0 $b 2>/dev/null || { cat "$out/b.log"; exit 1; }
    sleep 0.2
done
vm a 10.99.0.1 52:54:00:00:00:01 connect=127.0.0.1:18600 &
wait $! || true
grep -q "RESULT: ok" "$out/a.log"
