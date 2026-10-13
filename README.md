# pintos_rs

```bash
# pintos_rs — build & run in QEMU
#
# Recipes:
#   just build            # compile kernel, package build/os.dsk
#   just run ARGS...      # run in QEMU, e.g. just run "-q run alarm-single"
#   just runt TEST        # run a threads test, e.g. just runt alarm-single
#   just gdb              # QEMU waiting for gdb (attach: just gdb-attach)
#   just clean
```

# build

```bash
$ eza -la -TL2
drwxr-xr-x@    - gy 14 Oct 07:27 .
drwxr-xr-x@    - gy 14 Oct 07:25 └── build
.rwxr-xr-x@ 135k gy 14 Oct 07:25     ├── kernel.bin
.rwxr-xr-x@ 2.9M gy 14 Oct 07:25     ├── kernel.elf
.rwxr-xr-x@  512 gy 14 Oct 07:25     ├── loader.bin
.rwxr-xr-x@ 1.7k gy 14 Oct 07:25     ├── loader.elf
.rw-r--r--@ 1.8k gy 14 Oct 07:25     ├── loader.o
.rw-r--r--@ 136k gy 14 Oct 07:25     └── os.dsk
```

