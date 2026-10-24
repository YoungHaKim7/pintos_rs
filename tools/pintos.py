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
                  [-p HOST[:GUEST]]... [--timeout SEC]
                  [-- KERNEL-ARGS...]

Files named with -p are staged onto a scratch disk (attached at
IDE index 2) with "PUT" headers; the kernel command line gains a
matching `put GUEST' action (port of filesys/fsutil.c flow).

Example:
  tools/pintos.py -p build/args-single -- -q run args-single
"""

import argparse
import os
import signal
import struct
import subprocess
import sys
import tempfile

# Boot-sector file offsets (loader.h constants minus LOADER_BASE).
LOADER_ARG_CNT_OFF = 0x17A
LOADER_ARGS_OFF = 0x17E
LOADER_ARGS_LEN = 128
LOADER_SIG_OFF = 0x1FE

QEMU = "qemu-system-x86_64"


def align512(data: bytes) -> bytes:
    if len(data) % 512:
        data += b"\0" * (512 - len(data) % 512)
    return data


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


def stage_scratch(puts: list[str]) -> str | None:
    """Writes the -p files into a scratch disk image (index 2)."""
    if not puts:
        return None
    fd, path = tempfile.mkstemp(suffix=".dsk", prefix="scratch-")
    with os.fdopen(fd, "wb") as disk:
        for spec in puts:
            host, _, guest = spec.partition(":")
            guest = guest or os.path.basename(host)
            with open(host, "rb") as hf:
                data = hf.read()
            disk.write(b"PUT\0" + struct.pack("<I", len(data)) + b"\0" * 504)
            disk.write(align512(data))
            staged_guests.append(guest)
    return path


staged_guests: list[str] = []


def main() -> None:
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--disk", default="os.dsk")
    ap.add_argument("--fs-disk", type=int, default=0, metavar="MB")
    ap.add_argument("--swap-disk", type=int, default=0, metavar="MB")
    ap.add_argument("--timeout", type=float, default=60.0, metavar="SEC")
    ap.add_argument("--gdb", action="store_true", help="wait for gdb on :1234")
    ap.add_argument("-p", "--put-file", dest="puts", action="append", default=[],
                    metavar="HOST[:GUEST]")
    ap.add_argument("kernel_args", nargs="*", default=[])
    opts, passthrough = ap.parse_known_args()
    if passthrough:
        # Tolerate a "--" separator before the kernel arguments.
        opts.kernel_args += [a for a in passthrough if a != "--"]

    disk_path = os.path.abspath(opts.disk)
    with open(disk_path, "rb") as f:
        image = f.read()

    scratch = stage_scratch(opts.puts)

    # Kernel arguments: options first, then the `put' actions, then
    # the remaining (action) arguments.
    kernel_args = list(opts.kernel_args)
    puts_at = len(kernel_args)
    for i, a in enumerate(kernel_args):
        if not a.startswith("-"):
            puts_at = i
            break
    put_actions = [x for g in staged_guests for x in ("put", g)]
    kernel_args[puts_at:puts_at] = put_actions

    run_path = disk_path + ".run"
    with open(run_path, "wb") as f:
        f.write(patch_args(image, kernel_args))

    build = os.path.dirname(disk_path)
    cmd = [
        QEMU,
        "-drive", f"file={run_path},format=raw,index=0,media=disk",
        *attach_drives(build, opts.fs_disk, opts.swap_disk),
    ]
    if scratch:
        cmd += ["-drive", f"file={scratch},format=raw,index=2,media=disk"]
    cmd += [
        "-display", "none",
        "-serial", "stdio",
        "-monitor", "none",
        "-no-reboot",
        "-m", "128M",
    ]
    if opts.gdb:
        cmd += ["-S", "-gdb", "tcp::1234"]

    try:
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
    finally:
        if scratch:
            try:
                os.unlink(scratch)
            except OSError:
                pass


if __name__ == "__main__":
    main()
