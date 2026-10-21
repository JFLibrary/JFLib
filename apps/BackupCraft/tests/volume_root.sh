#!/bin/bash
# volume_root.sh -- the volume backup through an LVM snapshot on a small test volume (as root,
# with lvm2 and e2fsprogs)
#
#   sudo bash apps/BackupCraft/tests/volume_root.sh
#
# Makes a 400 MB image on a loop device, a volume group "bcvg" with a 200 MB logical volume
# "bclv" (ext4, mounted, a few files), runs volume_smoke.py --volume /dev/bcvg/bclv (LVM
# snapshot, restore, ext4 signature, second run with few new blocks), then removes everything
# again -- also when the test fails.  Needs build/linux/bc-server.
set -euo pipefail
cd "$(dirname "$0")/../../.."
[ "$(id -u)" = 0 ] || { echo "bitte als root (sudo)"; exit 2; }
command -v lvcreate >/dev/null || { echo "lvm2 fehlt (apt install lvm2)"; exit 2; }
img=$(mktemp /tmp/bc-lvm-XXXXXX.img)
mnt=$(mktemp -d /tmp/bc-lvm-mnt-XXXXXX)
truncate -s 400M "$img"
loop=$(losetup -f --show "$img")
cleanup() {
    umount "$mnt" 2>/dev/null || true
    lvremove -f -y bcvg >/dev/null 2>&1 || true
    vgremove -f -y bcvg >/dev/null 2>&1 || true
    pvremove -f -y "$loop" >/dev/null 2>&1 || true
    losetup -d "$loop" 2>/dev/null || true
    rm -rf "$img" "$mnt"
}
trap cleanup EXIT
pvcreate -q "$loop"
vgcreate -q bcvg "$loop"
lvcreate -q -y -L 200M -n bclv bcvg
mkfs.ext4 -q /dev/bcvg/bclv
mount /dev/bcvg/bclv "$mnt"
for i in $(seq 1 20); do head -c 200000 /dev/urandom > "$mnt/datei$i.bin"; done
sync
python3 apps/BackupCraft/tests/volume_smoke.py --server build/linux/bc-server --volume /dev/bcvg/bclv
