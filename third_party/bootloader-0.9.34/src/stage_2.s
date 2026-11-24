.section .boot, "awx"
.code16

# This stage sets the target operating mode, loads the kernel from disk,
# creates an e820 memory map, enters protected mode, and jumps to the
# third stage.

.set KERNEL_CHUNK_BLOCKS, 64
.set KERNEL_BOUNCE_SEGMENT, 0x7000
.set KERNEL_BOUNCE_LINEAR, 0x70000

second_stage_start_str: .asciz "Booting (second stage)..."
kernel_load_failed_str: .asciz "Failed to load kernel from disk"

kernel_load_failed:
    mov si, offset kernel_load_failed_str
    call real_mode_println
kernel_load_failed_spin:
    jmp kernel_load_failed_spin

stage_2:
    mov si, offset second_stage_start_str
    call real_mode_println

set_target_operating_mode:
    # Some BIOSs assume the processor will only operate in Legacy Mode. We change the Target
    # Operating Mode to "Long Mode Target Only", so the firmware expects each CPU to enter Long Mode
    # once and then stay in it. This allows the firmware to enable mode-specifc optimizations.
    # We save the flags, because CF is set if the callback is not supported (in which case, this is
    # a NOP)
    pushf
    mov ax, 0xec00
    mov bl, 0x2
    int 0x15
    popf

load_kernel_from_disk:
    # BOUCHAUD_CHARGEUR_PAR_LOTS_V1 (Bouchaud OS) : the upstream loop read the
    # kernel ONE sector per `int 0x13`. Under SeaBIOS every call is a full ATA
    # PIO command (seven register writes, status polling, PM timer reads):
    # a 13 MiB kernel cost ~7.2 million emulated port accesses before the
    # first kernel instruction -- one VM exit each under KVM/WHPX. We now read
    # KERNEL_CHUNK_BLOCKS sectors per call into a 32 KiB bounce buffer at
    # linear 0x70000 (segment 0x7000, inside one 64 KiB DMA page, above the
    # bootloader: see the ASSERT in linker.ld) and copy them to 4 MiB+.

    # bounce buffer
    mov word ptr [dap_buffer_addr], 0
    mov word ptr [dap_buffer_seg], KERNEL_BOUNCE_SEGMENT

    # number of start block
    mov eax, offset _kernel_start_addr
    mov ebx, offset _start
    sub eax, ebx
    shr eax, 9 # divide by 512 (block size)
    mov [dap_start_lba], eax

    # destination address
    mov edi, 0x400000

    # remaining block count
    mov ecx, offset _kernel_size
    add ecx, 511 # align up
    shr ecx, 9

load_next_kernel_chunk_from_disk:
    # this chunk: min(remaining, KERNEL_CHUNK_BLOCKS)
    mov ebx, ecx
    cmp ebx, KERNEL_CHUNK_BLOCKS
    jbe kernel_chunk_size_ok
    mov ebx, KERNEL_CHUNK_BLOCKS
kernel_chunk_size_ok:
    mov [dap_blocks], bx

    # load the chunk from disk
    mov si, offset dap
    mov ah, 0x42
    push ebx
    push ecx
    int 0x13
    pop ecx
    pop ebx
    jc kernel_load_failed

    # copy the chunk above 4 MiB (unreal mode: flat ds/es)
    push ecx
    mov ecx, ebx
    shl ecx, 7 # blocks * 512 / 4 dwords
    mov esi, KERNEL_BOUNCE_LINEAR
    rep movsd [edi], [esi]
    pop ecx

    # next chunk
    mov eax, [dap_start_lba]
    add eax, ebx
    mov [dap_start_lba], eax

    sub ecx, ebx
    jnz load_next_kernel_chunk_from_disk

create_memory_map:
    lea di, es:[_memory_map]
    call do_e820

video_mode_config:
    call config_video_mode

enter_protected_mode_again:
    cli
    lgdt [gdt32info]
    mov eax, cr0
    or al, 1    # set protected mode bit
    mov cr0, eax

    push 0x8
    mov eax, offset stage_3
    push eax
    retf

spin32:
    jmp spin32
