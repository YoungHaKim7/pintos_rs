#include "threads/loader.h"

OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)			/* Kernel starts at "start" symbol. */

/* The loader copies one flat image to physical LOADER_PHYS_BASE and the
   kernel runs at LOADER_KERN_BASE + LOADER_PHYS_BASE.  Therefore every
   file-backed section must keep the constant VMA-LMA delta
   LOADER_KERN_BASE (GNU ld does this implicitly after an AT(); lld does
   not, so we spell it out with AT(ADDR(...) - LOADER_KERN_BASE)). */

SECTIONS
{
  /* Specifies the virtual address for the kernel base. */
	. = LOADER_KERN_BASE + LOADER_PHYS_BASE;

	PROVIDE(start = .);
  /* Kernel starts with code, followed by read-only data and writable data. */
	.text : AT(LOADER_PHYS_BASE) {
		KEEP(*(.entry))
		*(.text .text.* .ltext .ltext.* .stub .gnu.linkonce.t.*)
	} = 0x90

	.rodata : AT(ADDR(.rodata) - LOADER_KERN_BASE) {
		*(.rodata .rodata.* .lrodata .lrodata.* .gnu.linkonce.r.*)
	}

	. = ALIGN(0x1000);
	PROVIDE(_end_kernel_text = .);

	.data : AT(ADDR(.data) - LOADER_KERN_BASE) {
		*(.data .data.* .ldata .ldata.* .sdata .sdata.*)
	}

  /* BSS (zero-initialized data) is after everything else. */
  PROVIDE(_start_bss = .);
	.bss : {
		*(.bss .bss.* .lbss .lbss.* .sbss .sbss.* COMMON)
	}
  PROVIDE(_end_bss = .);

  PROVIDE(_end = .);

	/DISCARD/ : {
		*(.eh_frame .eh_frame_hdr .note.GNU-stack .note.* .stab .stabstr .comment)
	}
}
