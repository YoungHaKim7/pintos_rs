// Build script for the pintos_rs kernel.
//
// Responsibilities:
//  1. Preprocess `asm/kernel.lds.S` (CPP) into a plain linker script.
//  2. Assemble the kernel's GNU-as sources (`start.S`, `intr-stubs.S`,
//     `syscall-entry.S`) with the host `gcc` into ELF objects, archive them,
//     and hand the archive to rustc so `rust-lld` links them in.
//     (`loader.S` is NOT assembled here: it must be built after linking with
//     `-DKERNEL_LOAD_PAGES=<kernel.bin/4096>`; the justfile handles it.)
//  3. Emit link args: the linker script and section-discarding flags.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run(cmd: &mut Command) {
    println!("cargo:rerun-if-changed=build.rs");
    let status = cmd.status().expect("failed to spawn command");
    if !status.success() {
        panic!("command failed: {:?}", cmd);
    }
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let asm = manifest.join("asm");

    println!("cargo:rerun-if-changed=asm/");
    println!("cargo:rerun-if-changed=asm/include/threads/loader.h");

    // ---- 1. Linker script: kernel.lds.S -> kernel.lds --------------------
    let lds = out_dir.join("kernel.lds");
    run(Command::new("gcc")
        .arg("-P")
        .arg("-E")
        .arg("-I")
        .arg(asm.join("include"))
        .arg("-o")
        .arg(&lds)
        .arg(asm.join("kernel.lds.S")));

    // ---- 2. Assemble kernel .S files -------------------------------------
    // Same flags as the C build's ASFLAGS (`Make.config`):
    //   -mcmodel=large, no red zone, no SSE.
    // The objects are passed to the linker as direct inputs (not via an
    // archive): archive members are only extracted when they resolve an
    // undefined symbol, but `_start` (start.o) is referenced only by the
    // linker script's ENTRY(), so an archive would drop it.
    let asm_sources = ["start.S", "intr-stubs.S", "syscall-entry.S"];
    for src in asm_sources {
        let obj = out_dir.join(src.replace(".S", ".o"));
        run(Command::new("gcc")
            .arg("-c")
            .arg("-mcmodel=large")
            .arg("-mno-red-zone")
            .arg("-mno-sse")
            .arg("-I")
            .arg(asm.join("include"))
            .arg("-o")
            .arg(&obj)
            .arg(asm.join(src)));
        println!("cargo:rustc-link-arg={}", obj.display());
    }

    // ---- 3. Tell cargo how to link ---------------------------------------
    println!("cargo:rustc-link-arg=-T{}", lds.display());
    // Do not pull in default objects/libs; we are freestanding.
    println!("cargo:rustc-link-arg=-nostdlib");

    let _ = Path::new("build").metadata(); // build/ is created by the justfile
}
