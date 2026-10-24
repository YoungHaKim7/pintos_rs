#!/usr/bin/env python3
"""Self-contained replacement for pintos-kaist's utils/pintos.

Patches the kernel command line into the boot sector of a disk
image built by the justfile (loader protocol: 4-byte little-endian
argument count at 0x7d7a, NUL-separated arguments in the 128-byte
area at 0x7d7e, 0xaa55 signature at 0x7dfe), then boots it in QEMU
with serial on stdio.  The kernel powers QEMU off itself by writing
0x2000 to port 0x604 (ACPI sleep).

Usage:
  tools/pintos.py [--disk OS.DSK] [--fs-disk MB] [--swap-disk MB]
                  [--timeout SEC] [-- KERNEL-ARGS...]

Example:
  tools/pintos.py -- -q -threads-tests run alarm-single
"""

import argparse
import os
import signal
import struct
import subprocess
import sys

# Boot-sector file offsets (loader.h constants minus LOADER_BASE).
LOADER_ARG_CNT_OFF = 0x17A
LOADER_ARGS_OFF = 0x17E
LOADER_ARGS_LEN = 128
LOADER_SIG_OFF = 0x1FE

QEMU = "qemu-system-x86_64"


def patch_args(disk: bytes, args: list[str]) -> bytes:
    blob = b"".join(a.encode() + b"\0" for a in args)
    if len(blob) > LOADER_ARGS_LEN:
        sys.exit(f"pintos: kernel arguments exceed {LOADER_ARGS_LEN} bytes")
    data = bytearray(disk)
    assert data[LOADER_SIG_OFF : LOADER_SIG_OFF + 2] == b"\x55\xaa", (
        "missing boot signature: not a loader disk image"
    )
    data[LOADER_ARGS_OFF : LOADER_ARGS_OFF + LOADER_ARGS_LEN] = blob.ljust(
        LOADER_ARGS_LEN, b"\0"
    )
    struct.pack_into("<I", data, LOADER_ARG_CNT_OFF, len(args))
    return bytes(data)


def attach_drives(build: str, fs_mb: int, swap_mb: int) -> list[str]:
    """Creates sparse fs/swap images on demand; returns -drive flags."""
    drives = []
    for index, (name, mb) in enumerate(
        (("filesys.dsk", fs_mb), ("swap.dsk", swap_mb)), start=1
    ):
        if mb <= 0:
            continue
        path = os.path.join(build, name)
        if not os.path.exists(path):
            mb_bytes = mb * 1024 * 1024
            with open(path, "wb") as f:
                f.truncate(mb_bytes)
        drives += ["-drive", f"file={path},format=raw,index={index},media=disk"]
    return drives


def main() -> None:
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--disk", default="os.dsk")
    ap.add_argument("--fs-disk", type=int, default=0, metavar="MB")
    ap.add_argument("--swap-disk", type=int, default=0, metavar="MB")
    ap.add_argument("--timeout", type=float, default=60.0, metavar="SEC")
    ap.add_argument("--gdb", action="store_true", help="wait for gdb on :1234")
    ap.add_argument("kernel_args", nargs="*", default=[])
    opts, passthrough = ap.parse_known_args()
    if passthrough:
        # Tolerate a "--" separator before the kernel arguments.
        opts.kernel_args += [a for a in passthrough if a != "--"]

    disk_path = os.path.abspath(opts.disk)
    with open(disk_path, "rb") as f:
        image = patch_args(f.read(), opts.kernel_args)
    run_path = disk_path + ".run"
    with open(run_path, "wb") as f:
        f.write(image)

    build = os.path.dirname(disk_path)
    cmd = [
        QEMU,
        "-drive", f"file={run_path},format=raw,index=0,media=disk",
        *attach_drives(build, opts.fs_disk, opts.swap_disk),
        "-display", "none",
        "-serial", "stdio",
        "-monitor", "none",
        "-no-reboot",
        "-m", "128M",
    ]
    if opts.gdb:
        cmd += ["-S", "-gdb", "tcp::1234"]

    proc = subprocess.Popen(cmd)
    try:
        rc = proc.wait(timeout=opts.timeout)
    except subprocess.TimeoutExpired:
        print(f"\npintos: timeout after {opts.timeout:g}s, killing QEMU",
              file=sys.stderr)
        os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        proc.wait()
        sys.exit(1)
    sys.exit(rc)


if __name__ == "__main__":
    main()
