//! Port of `threads/interrupt.c` + `include/threads/interrupt.h` —
//! interrupt level control, the 8259A Programmable Interrupt
//! Controllers, the Interrupt Descriptor Table, and dispatch of
//! interrupts, faults, and exceptions to registered handlers.

use crate::intrinsic::{DescPtr, cli, lidt, ltr, outb, rcr2, read_eflags, sti};
use crate::lib::stdio::Arg;
use crate::threads::loader::{SEL_KCSEG, SEL_TSS};

/* Number of x86_64 interrupts. */
const INTR_CNT: usize = 256;

/* EFLAGS bit for the interrupt flag (include/threads/flags.h). */
const FLAG_IF: u64 = 1 << 9;

/* Interrupts on or off?  (C's `enum intr_level`.) */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrLevel {
    Off, /* Interrupts disabled. */
    On,  /* Interrupts enabled. */
}

/* Interrupt stack frame. */

/* General-purpose registers pushed by `intr_entry` in intr-stubs.S.
These are the interrupted task's saved registers.  The field order
is fixed by the pushes in the assembly: intr_entry stores %r15 at
offset 0 ... %rax at offset 112. */
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct GpRegisters {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
}

/* Complete interrupt stack frame, as laid out on the stack by the
CPU, the intrNN_stub routines, and intr_entry.  The layout must
match intr-stubs.S exactly. */
#[repr(C, packed)]
#[allow(non_snake_case)] /* the `R` member is named by interrupt.h */
#[derive(Clone, Copy)]
pub struct IntrFrame {
    /* Pushed by intr_entry in intr-stubs.S.
    These are the interrupted task's saved registers. */
    pub R: GpRegisters,
    /* intr_entry does `subq $16,%rsp; movw %es,0(%rsp); movw %ds,8(%rsp)':
    two 8-byte slots holding 16-bit segment registers. */
    pub es: u16,
    pub __pad1: u16,
    pub __pad2: u32,
    pub ds: u16,
    pub __pad3: u16,
    pub __pad4: u32,
    /* Pushed by intrNN_stub in intr-stubs.S. */
    pub vec_no: u64, /* Interrupt vector number. */
    /* Sometimes pushed by the CPU,
    otherwise for consistency pushed as 0 by intrNN_stub.
    The CPU puts it just under `eip', but we move it here. */
    pub error_code: u64,
    /* Pushed by the CPU.
    These are the interrupted task's saved registers. */
    pub rip: u64,
    pub cs: u16,
    pub __pad5: u16,
    pub __pad6: u32,
    pub eflags: u64,
    pub rsp: u64,
    pub ss: u16,
    pub __pad7: u16,
    pub __pad8: u32,
}

/* An interrupt handler function (C's `typedef void intr_handler_func
(struct intr_frame *)').  Handlers are invoked with a pointer to
the interrupted thread's register frame. */
pub type IntrHandlerFunc = extern "C" fn(frame: *mut IntrFrame);

/* Creates an gate that invokes FUNCTION.

The gate has descriptor privilege level DPL, meaning that it
can be invoked intentionally when the processor is in the DPL
or lower-numbered ring.  In practice, DPL==3 allows user mode
to call into the gate and DPL==0 prevents such calls.  Faults
and exceptions that occur in user mode still cause gates with
DPL==0 to be invoked.

TYPE must be either 14 (for an interrupt gate) or 15 (for a
trap gate).  The difference is that entering an interrupt gate
disables interrupts, but entering a trap gate does not.  See
[IA32-v3a] section 5.12.1.2 "Flag Usage By Exception- or
Interrupt-Handler Procedure" for discussion. */

/* The 16-byte IDT gate descriptor.  C declared this as a struct of
   bit-fields; Rust has no bit-fields, so the fields are hand-packed
   into two 64-bit words at the same bit positions GCC assigned them
   on x86-64 (little-endian):

     word 0, bits  0..16: off_15_0   low 16 bits of offset in segment
               bits 16..32: ss         segment selector
               bits 32..35: ist        # args, 0 for interrupt/trap gates
               bits 35..40: rsv1       reserved(should be zero I guess)
               bits 40..44: type       type(STS_{TG,IG32,TG32})
               bit  44    : s          must be 0 (system)
               bits 45..47: dpl        descriptor(meaning new) privilege level
               bit  47    : p          Present
               bits 48..64: off_31_16  high bits of offset in segment
     word 1, bits  0..32: off_32_63
               bits 32..64: rsv2
*/
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Gate {
    w: [u64; 2],
}

impl Gate {
    pub const ZERO: Gate = Gate { w: [0; 2] };
}

/* The Interrupt Descriptor Table (IDT).  The format is fixed by
the CPU.  See [IA32-v3a] sections 5.10 "Interrupt Descriptor
   Table (IDT)", 5.11 "IDT Descriptors", 5.12.1.2 "Flag Usage By
   Exception- or Interrupt-Handler Procedure". */
static mut IDT: [Gate; INTR_CNT] = [Gate::ZERO; INTR_CNT];

static mut IDT_DESC: DescPtr = DescPtr {
    size: (INTR_CNT * core::mem::size_of::<Gate>() - 1) as u16,
    /* Filled in with `&IDT' by intr_init (C initialized it
    statically; Rust statics cannot address other statics). */
    address: 0,
};

/* The interrupt stub entry points, one per vector, from
intr-stubs.S (C's `extern intr_stub_func *intr_stubs[256]').  The
assembly defines `intr_stubs' as an array of 256 quadword
pointers in .data. */
unsafe extern "C" {
    #[link_name = "intr_stubs"]
    static INTR_STUBS: [*mut u8; INTR_CNT];
}

/* Creates a gate that invokes FUNCTION with the layout described
above.  (Port of the C `make_gate' macro.) */
unsafe fn make_gate(g: *mut Gate, function: *mut u8, d: u64, t: u64) {
    let f = function as u64;

    crate::ASSERT!(!function.is_null());
    crate::ASSERT!(d <= 3);
    crate::ASSERT!(t <= 15);

    unsafe {
        *g = Gate {
            w: [
                (f & 0xffff) /* off_15_0 */
                    | ((SEL_KCSEG as u64) << 16) /* ss */
                    /* ist = 0, rsv1 = 0, s = 0 */
                    | (t << 40) /* type */
                    | (d << 45) /* dpl */
                    | (1u64 << 47) /* p */
                    | (((f >> 16) & 0xffff) << 48), /* off_31_16 */
                /* off_32_63; rsv2 = 0 */
                (f >> 32) & 0xffff_ffff,
            ],
        };
    }
}

/* Creates an interrupt gate that invokes FUNCTION with the given DPL. */
unsafe fn make_intr_gate(g: *mut Gate, function: *mut u8, dpl: u64) {
    unsafe { make_gate(g, function, dpl, 14) }
}

/* Creates a trap gate that invokes FUNCTION with the given DPL. */
unsafe fn make_trap_gate(g: *mut Gate, function: *mut u8, dpl: u64) {
    unsafe { make_gate(g, function, dpl, 15) }
}

/* Interrupt handler functions for each interrupt. */
static mut INTR_HANDLERS: [Option<IntrHandlerFunc>; INTR_CNT] = [None; INTR_CNT];

/* Names for each interrupt, for debugging purposes. */
static mut INTR_NAMES: [*const u8; INTR_CNT] = [core::ptr::null(); INTR_CNT];

/* External interrupts are those generated by devices outside the
CPU, such as the timer.  External interrupts run with
interrupts turned off, so they never nest, nor are they ever
pre-empted.  Handlers for external interrupts also may not
sleep, although they may invoke intr_yield_on_return() to
request that a new process be scheduled just before the
interrupt returns. */
static mut IN_EXTERNAL_INTR: bool = false; /* Are we processing an external interrupt? */
static mut YIELD_ON_RETURN: bool = false; /* Should we yield on interrupt return? */

/* Returns the current interrupt status. */
#[inline]
pub fn intr_get_level() -> IntrLevel {
    /* Push the flags register on the processor stack, then pop the
    value off the stack into `flags'.  See [IA32-v2b] "PUSHF"
    and "POP" and [IA32-v3a] 5.8.1 "Masking Maskable Hardware
    Interrupts". */
    if unsafe { read_eflags() } & FLAG_IF != 0 {
        IntrLevel::On
    } else {
        IntrLevel::Off
    }
}

/* Enables or disables interrupts as specified by LEVEL and
returns the previous interrupt status. */
#[inline]
pub fn intr_set_level(level: IntrLevel) -> IntrLevel {
    if level == IntrLevel::On {
        intr_enable()
    } else {
        intr_disable()
    }
}

/* Enables interrupts and returns the previous interrupt status. */
#[inline]
pub fn intr_enable() -> IntrLevel {
    let old_level = intr_get_level();
    crate::ASSERT!(!intr_context());

    /* Enable interrupts by setting the interrupt flag.

    See [IA32-v2b] "STI" and [IA32-v3a] 5.8.1 "Masking Maskable
    Hardware Interrupts". */
    unsafe { sti() };

    old_level
}

/* Disables interrupts and returns the previous interrupt status. */
#[inline]
pub fn intr_disable() -> IntrLevel {
    let old_level = intr_get_level();

    /* Disable interrupts by clearing the interrupt flag.
    See [IA32-v2b] "CLI" and [IA32-v3a] 5.8.1 "Masking Maskable
    Hardware Interrupts". */
    unsafe { cli() };

    old_level
}

/* Initializes the interrupt system. */
pub unsafe fn intr_init() {
    unsafe {
        /* Initialize interrupt controller. */
        pic_init();

        /* Initialize IDT. */
        for i in 0..INTR_CNT {
            make_intr_gate(&raw mut IDT[i], INTR_STUBS[i], 0);
            INTR_NAMES[i] = b"unknown\0".as_ptr();
        }

        /* Load TSS.  (C guards this with #ifdef USERPROG; this
        kernel always builds the userprog facility.) */
        ltr(SEL_TSS);

        /* Load IDT register. */
        IDT_DESC.address = core::ptr::addr_of_mut!(IDT) as u64;
        lidt(core::ptr::addr_of!(IDT_DESC));

        /* Initialize intr_names. */
        INTR_NAMES[0] = b"#DE Divide Error\0".as_ptr();
        INTR_NAMES[1] = b"#DB Debug Exception\0".as_ptr();
        INTR_NAMES[2] = b"NMI Interrupt\0".as_ptr();
        INTR_NAMES[3] = b"#BP Breakpoint Exception\0".as_ptr();
        INTR_NAMES[4] = b"#OF Overflow Exception\0".as_ptr();
        INTR_NAMES[5] = b"#BR BOUND Range Exceeded Exception\0".as_ptr();
        INTR_NAMES[6] = b"#UD Invalid Opcode Exception\0".as_ptr();
        INTR_NAMES[7] = b"#NM Device Not Available Exception\0".as_ptr();
        INTR_NAMES[8] = b"#DF Double Fault Exception\0".as_ptr();
        INTR_NAMES[9] = b"Coprocessor Segment Overrun\0".as_ptr();
        INTR_NAMES[10] = b"#TS Invalid TSS Exception\0".as_ptr();
        INTR_NAMES[11] = b"#NP Segment Not Present\0".as_ptr();
        INTR_NAMES[12] = b"#SS Stack Fault Exception\0".as_ptr();
        INTR_NAMES[13] = b"#GP General Protection Exception\0".as_ptr();
        INTR_NAMES[14] = b"#PF Page-Fault Exception\0".as_ptr();
        INTR_NAMES[16] = b"#MF x87 FPU Floating-Point Error\0".as_ptr();
        INTR_NAMES[17] = b"#AC Alignment Check Exception\0".as_ptr();
        INTR_NAMES[18] = b"#MC Machine-Check Exception\0".as_ptr();
        INTR_NAMES[19] = b"#XF SIMD Floating-Point Exception\0".as_ptr();
    }
}

/* Registers interrupt VEC_NO to invoke HANDLER with descriptor
privilege level DPL.  Names the interrupt NAME for debugging
purposes.  The interrupt handler will be invoked with
interrupt status set to LEVEL. */
unsafe fn register_handler(
    vec_no: u8,
    dpl: i32,
    level: IntrLevel,
    handler: IntrHandlerFunc,
    name: *const u8,
) {
    unsafe {
        crate::ASSERT!(INTR_HANDLERS[vec_no as usize].is_none());
        if level == IntrLevel::On {
            make_trap_gate(
                &raw mut IDT[vec_no as usize],
                INTR_STUBS[vec_no as usize],
                dpl as u64,
            );
        } else {
            make_intr_gate(
                &raw mut IDT[vec_no as usize],
                INTR_STUBS[vec_no as usize],
                dpl as u64,
            );
        }
        INTR_HANDLERS[vec_no as usize] = Some(handler);
        INTR_NAMES[vec_no as usize] = name;
    }
}

/* Registers external interrupt VEC_NO to invoke HANDLER, which
is named NAME for debugging purposes.  The handler will
execute with interrupts disabled. */
pub unsafe fn intr_register_ext(vec_no: u8, handler: IntrHandlerFunc, name: *const u8) {
    unsafe {
        crate::ASSERT!(vec_no >= 0x20 && vec_no <= 0x2f);
        register_handler(vec_no, 0, IntrLevel::Off, handler, name);
    }
}

/* Registers internal interrupt VEC_NO to invoke HANDLER, which
is named NAME for debugging purposes.  The interrupt handler
will be invoked with interrupt status LEVEL.

The handler will have descriptor privilege level DPL, meaning
that it can be invoked intentionally when the processor is in
the DPL or lower-numbered ring.  In practice, DPL==3 allows
user mode to invoke the interrupts and DPL==0 prevents such
invocation.  Faults and exceptions that occur in user mode
still cause interrupts with DPL==0 to be invoked.  See
[IA32-v3a] sections 4.5 "Privilege Levels" and 4.8.1.1
"Accessing Nonconforming Code Segments" for further
discussion. */
pub unsafe fn intr_register_int(
    vec_no: u8,
    dpl: i32,
    level: IntrLevel,
    handler: IntrHandlerFunc,
    name: *const u8,
) {
    unsafe {
        crate::ASSERT!((vec_no as i32) < 0x20 || (vec_no as i32) > 0x2f);
        register_handler(vec_no, dpl, level, handler, name);
    }
}

/* Returns true during processing of an external interrupt
and false at all other times. */
#[inline]
pub fn intr_context() -> bool {
    unsafe { IN_EXTERNAL_INTR }
}

/* During processing of an external interrupt, directs the
interrupt handler to yield to a new process just before
returning from the interrupt.  May not be called at any other
time. */
pub unsafe fn intr_yield_on_return() {
    crate::ASSERT!(intr_context());
    unsafe {
        YIELD_ON_RETURN = true;
    }
}

/* 8259A Programmable Interrupt Controller. */

/* Every PC has two 8259A Programmable Interrupt Controller (PIC)
chips.  One is a "master" accessible at ports 0x20 and 0x21.
The other is a "slave" cascaded onto the master's IRQ 2 line
and accessible at ports 0xa0 and 0xa1.  Accesses to port 0x20
set the A0 line to 0 and accesses to 0x21 set the A1 line to
1.  The situation is similar for the slave PIC.

By default, interrupts 0...15 delivered by the PICs will go to
interrupt vectors 0...15.  Unfortunately, those vectors are
also used for CPU traps and exceptions.  We reprogram the PICs
so that interrupts 0...15 are delivered to interrupt vectors
32...47 (0x20...0x2f) instead. */

/* Initializes the PICs.  Refer to [8259A] for details. */
unsafe fn pic_init() {
    unsafe {
        /* Mask all interrupts on both PICs. */
        outb(0x21, 0xff);
        outb(0xa1, 0xff);

        /* Initialize master. */
        outb(0x20, 0x11); /* ICW1: single mode, edge triggered, expect ICW4. */
        outb(0x21, 0x20); /* ICW2: line IR0...7 -> irq 0x20...0x27. */
        outb(0x21, 0x04); /* ICW3: slave PIC on line IR2. */
        outb(0x21, 0x01); /* ICW4: 8086 mode, normal EOI, non-buffered. */

        /* Initialize slave. */
        outb(0xa0, 0x11); /* ICW1: single mode, edge triggered, expect ICW4. */
        outb(0xa1, 0x28); /* ICW2: line IR0...7 -> irq 0x28...0x2f. */
        outb(0xa1, 0x02); /* ICW3: slave ID is 2. */
        outb(0xa1, 0x01); /* ICW4: 8086 mode, normal EOI, non-buffered. */

        /* Unmask all interrupts. */
        outb(0x21, 0x00);
        outb(0xa1, 0x00);
    }
}

/* Sends an end-of-interrupt signal to the PIC for the given IRQ.
If we don't acknowledge the IRQ, it will never be delivered to
us again, so this is important.  */
unsafe fn pic_end_of_interrupt(irq: i32) {
    crate::ASSERT!(irq >= 0x20 && irq < 0x30);

    unsafe {
        /* Acknowledge master PIC. */
        outb(0x20, 0x20);

        /* Acknowledge slave PIC if this is a slave interrupt. */
        if irq >= 0x28 {
            outb(0xa0, 0x20);
        }
    }
}

/* Interrupt handlers. */

/* Handler for all interrupts, faults, and exceptions.  This
function is called by the assembly language interrupt stubs in
intr-stubs.S.  FRAME describes the interrupt and the
interrupted thread's registers. */
#[unsafe(no_mangle)]
pub extern "C" fn intr_handler(frame: *mut IntrFrame) {
    unsafe {
        /* External interrupts are special.
        We only handle one at a time (so interrupts must be off)
        and they need to be acknowledged on the PIC (see below).
        An external interrupt handler cannot sleep. */
        let external = (*frame).vec_no >= 0x20 && (*frame).vec_no < 0x30;
        if external {
            crate::ASSERT!(intr_get_level() == IntrLevel::Off);
            crate::ASSERT!(!intr_context());

            IN_EXTERNAL_INTR = true;
            YIELD_ON_RETURN = false;
        }

        /* Invoke the interrupt's handler. */
        let handler = INTR_HANDLERS[(*frame).vec_no as usize];
        if let Some(handler) = handler {
            handler(frame);
        } else if (*frame).vec_no == 0x27 || (*frame).vec_no == 0x2f {
            /* There is no handler, but this interrupt can trigger
            spuriously due to a hardware fault or hardware race
            condition.  Ignore it. */
        } else {
            /* No handler and not spurious.  Invoke the unexpected
            interrupt handler. */
            intr_dump_frame(frame);
            crate::PANIC!("Unexpected interrupt");
        }

        /* Complete the processing of an external interrupt. */
        if external {
            crate::ASSERT!(intr_get_level() == IntrLevel::Off);
            crate::ASSERT!(intr_context());

            IN_EXTERNAL_INTR = false;
            pic_end_of_interrupt((*frame).vec_no as i32);

            if YIELD_ON_RETURN {
                crate::threads::thread::thread_yield();
            }
        }
    }
}

/* Dumps interrupt frame F to the console, for debugging. */
pub unsafe fn intr_dump_frame(f: *const IntrFrame) {
    unsafe {
        let f = &*f;

        /* CR2 is the linear address of the last page fault.
        See [IA32-v2a] "MOV--Move to/from Control Registers" and
        [IA32-v3a] 5.14 "Interrupt 14--Page Fault Exception
        (#PF)". */
        let cr2 = rcr2();
        crate::printf!(
            "Interrupt %#04llx (%s) at rip=%llx\n",
            f.vec_no,
            Arg::Raw(INTR_NAMES[f.vec_no as usize]),
            f.rip
        );
        crate::printf!(" cr2=%016llx error=%16llx\n", cr2, f.error_code);
        crate::printf!(
            "rax %016llx rbx %016llx rcx %016llx rdx %016llx\n",
            f.R.rax,
            f.R.rbx,
            f.R.rcx,
            f.R.rdx
        );
        crate::printf!(
            "rsp %016llx rbp %016llx rsi %016llx rdi %016llx\n",
            f.rsp,
            f.R.rbp,
            f.R.rsi,
            f.R.rdi
        );
        crate::printf!(
            "rip %016llx r8 %016llx  r9 %016llx r10 %016llx\n",
            f.rip,
            f.R.r8,
            f.R.r9,
            f.R.r10
        );
        crate::printf!(
            "r11 %016llx r12 %016llx r13 %016llx r14 %016llx\n",
            f.R.r11,
            f.R.r12,
            f.R.r13,
            f.R.r14
        );
        crate::printf!("r15 %016llx rflags %08llx\n", f.R.r15, f.eflags);
        crate::printf!(
            "es: %04x ds: %04x cs: %04x ss: %04x\n",
            f.es,
            f.ds,
            f.cs,
            f.ss
        );
    }
}

/* Returns the name of interrupt VEC. */
pub unsafe fn intr_name(vec: u8) -> *const u8 {
    unsafe { INTR_NAMES[vec as usize] }
}
