# qumu test

```bash
qemu-system-x86_64 -nographic -drive 
  file=os.dsk,format=raw,index=0,media=disk -s -S -no-reboot -serial mon:stdio <
  /dev/null > /dev/null 2>&1 &) && sleep 2 && gdb kernel.
    -ex 'set architecture i386:x86-64' \
    -ex 'target remote :1234' \
    -ex 'hbreak *0x200000' \
    -ex 'c' \
    -ex 'x/8i $pc' \
    -ex 'si 40' \
    -ex 'info registers rip cr0 cr3 cr4 efer cs' 2>&1 | tail -25; pkill -f "qemu-system-x86_64.*1234" 2>/dev/null; true)
Error: Exit code 144
The target architecture is set to "i386:x86-64".
0x000000000000fff0 in ?? ()
Hardware assisted breakpoint 1 at 0x200000

Breakpoint 1, 0x0000000000200000 in ?? ()
=> 0x200000:       add    $0x0,%al
   0x200002:       add    %al,(%rax)
   0x200004:       and    %al,(%rax)
   0x200006:       add    %al,(%rax)
   0x200008:       add    $0x47000000,%eax
   0x20000d:       rex.WRX push %rbp
   0x20000f:       add    %al,(%rcx)
   0x200011:       add    %al,(%rcx)
0x0000000000200071 in ?? ()
rip            0x200071            0x200071
cr0            0x11                [ ET PE ]
cr3            0x0                 [ PDBR=0 PCID=0 ]
cr4            0x220               [ OSFXSR PAE ]
efer           0x0                 [ ]
cs             0x8                 8
[Inferior 1 (process 1) detached]

∴ The GDB session worked: at 0x200000, the code bytes as decoded by GDB in 16-bit-ish interpretation look off (GDB decodes as 16-bit real-mode at start — that's fine, the actual CPU is in 32-bit after loader's far jump... wait, the loader jumps to the kernel in 32-bit protected mode with CS=0x8? QEMU dump earlier showed CS=0008 CS32 at the fault. But here at 0x200000 GDB shows add $0x0,%al — first bytes are 00 00... The first instruction bytes at kernel offset 0 should be bootstrap's start: start.S begins .section .entry, "ax"\n.code32\nbootstrap:\n cli; cld...? Let me check start.S first bytes.
```

- test2

```bash
$ timeout 8 qemu-system-x86_64 -nographic -drive file=os.dsk,format=raw,index=0,media=disk -d int,guest_errors -D /tmp/q8.log -no-reboot -serial none < /dev/null > /dev/null 2>&1; grep -m1 -A10 "v=" /tmp/q8.log | head -14; grep -m1 "IP=" /tmp/q8.log | grep -oE "IP=[0-9a-f:]+"
```
