# pintos_rs — build & run in QEMU
#
# Recipes:
#   just build            # compile kernel, package build/os.dsk
#   just run ARGS...      # run in QEMU, e.g. just run "-q run alarm-single"
#   just runt TEST        # run a threads test, e.g. just runt alarm-single
#   just gdb              # QEMU waiting for gdb (attach: just gdb-attach)
#   just clean

pintos_util := "../pintos-kaist/utils/pintos"
arch        := "targets/x86_64-pintos.json"
elf         := "target/x86_64-pintos/debug/pintos_rs"

# Default: build the disk image.
build:
    #!/bin/sh
    set -e
    mkdir -p build
    cargo build
    cp target/x86_64-pintos/debug/pintos_rs build/kernel.elf
    # Flat binary from the ELF (uses LMA = 0x200000).
    objcopy -O binary build/kernel.elf build/kernel.bin
    # Pad kernel.bin to a 4096 multiple and compute loader page count.
    SIZE=$(stat -c %s build/kernel.bin)
    PAGES=$(( (SIZE + 4095) / 4096 ))
    PAD=$(( PAGES * 4096 - SIZE ))
    [ "$PAD" -ne 0 ] && dd if=/dev/zero bs=1 count=$PAD >> build/kernel.bin 2>/dev/null
    echo "kernel.bin: $SIZE bytes, KERNEL_LOAD_PAGES=$PAGES"
    # Loader: MBR at 0x7c00 with baked-in kernel size.
    gcc -c -DKERNEL_LOAD_PAGES=$PAGES -I asm/include -o build/loader.o asm/loader.S
    ld -m elf_x86_64 -Ttext 0x7c00 -o build/loader.elf build/loader.o
    objcopy -O binary build/loader.elf build/loader.bin
    # Final disk image.
    cat build/loader.bin build/kernel.bin > build/os.dsk

# Run in QEMU via the (unmodified) pintos harness. Args after recipe name
# are passed as KERNEL arguments, e.g.: just run "-q -threads-tests run priority-fifo"
run *ARGS: build
    #!/bin/sh
    set -e
    cd build
    exec python3 {{pintos_util}} --fs-disk=4 --swap-disk=4 -- {{ARGS}}

# Run a threads test: just runt alarm-single
runt TEST: build
    #!/bin/sh
    set -e
    cd build
    exec python3 {{pintos_util}} --fs-disk=4 --swap-disk=4 -- -q -threads-tests run {{TEST}}

# Release build of the image.
build-release:
    #!/bin/sh
    set -e
    mkdir -p build
    cargo build --release
    cp target/x86_64-pintos/release/pintos_rs build/kernel.elf
    objcopy -O binary build/kernel.elf build/kernel.bin
    SIZE=$(stat -c %s build/kernel.bin)
    PAGES=$(( (SIZE + 4095) / 4096 ))
    PAD=$(( PAGES * 4096 - SIZE ))
    [ "$PAD" -ne 0 ] && dd if=/dev/zero bs=1 count=$PAD >> build/kernel.bin 2>/dev/null
    gcc -c -DKERNEL_LOAD_PAGES=$PAGES -I asm/include -o build/loader.o asm/loader.S
    ld -m elf_x86_64 -Ttext 0x7c00 -o build/loader.elf build/loader.o
    objcopy -O binary build/loader.elf build/loader.bin
    cat build/loader.bin build/kernel.bin > build/os.dsk

# QEMU paused, waiting for gdb on :1234.
gdb: build
    cd build && python3 {{pintos_util}} --gdb --fs-disk=4 --swap-disk=4 -- -q

# Clean build artifacts.
clean:
    cargo clean
    rm -rf build
