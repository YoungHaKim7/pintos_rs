```bash
✽ Porting foundation modules…
◼ Port foundation modules (main.rs, intrinsic, mmu/pte/vaddr, list/ha…
◼ Port lib layer (string/stdio/stdlib/random/debug + console)
◼ Port devices (serial/vga/intq/input/kbd/timer/disk)
◻ Port threads core (thread/synch/interrupt/palloc/malloc/init) + priority donation
◻ Port userprog (gdt/tss/exception/process/syscall) + argument passing + syscalls
◻ Port vm (spt/lazy/anon/file/swap/cow) + implement assignments
```

```bash
# ##
# Done
Threads suite: ALL 18 PASS
filesys base: FAT ported + format works in QEMU

# ##
# Progress
userprog: Thread fields + process.rs (exec/load/args/wait/exit/fork)
userprog: syscall.rs full implementation
Build user programs with C tree; FAT injector tool; run userprog tests

# ##
# Waiting
Port vm (spt/lazy/anon/file/swap/cow) + mlfqs tests
Extended filesys (subdirs, symlink) if time permits
```
