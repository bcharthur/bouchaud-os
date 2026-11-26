/* BOUCHAUD_VDSO_V1 -- disposition du vDSO (d'apres le vdso-layout de Linux).
 * Tout tient dans un seul segment executable en lecture seule ; la page de
 * donnees du noyau est projetee 4 Kio AVANT l'en-tete ELF. */
SECTIONS
{
    bo_vvar_page = . - 4096;

    . = SIZEOF_HEADERS;

    .hash           : { *(.hash) }                  :text
    .gnu.hash       : { *(.gnu.hash) }
    .dynsym         : { *(.dynsym) }
    .dynstr         : { *(.dynstr) }
    .gnu.version    : { *(.gnu.version) }
    .gnu.version_d  : { *(.gnu.version_d) }
    .gnu.version_r  : { *(.gnu.version_r) }
    .dynamic        : { *(.dynamic) }               :text :dynamic
    .rodata         : { *(.rodata*) }               :text
    .note           : { *(.note.*) }                :text :note
    .eh_frame_hdr   : { *(.eh_frame_hdr) }          :text :eh_frame_hdr
    .eh_frame       : { KEEP (*(.eh_frame)) }       :text
    .text           : { *(.text*) }                 :text

    /DISCARD/ : { *(.data*) *(.bss*) *(.got*) *(.plt*) *(.comment) *(.note.GNU-stack) }
}

PHDRS
{
    text            PT_LOAD         FLAGS(5) FILEHDR PHDRS; /* R_E */
    dynamic         PT_DYNAMIC      FLAGS(4);               /* R__ */
    note            PT_NOTE         FLAGS(4);
    eh_frame_hdr    PT_GNU_EH_FRAME;
}

VERSION
{
    LINUX_2.6 {
    global:
        clock_gettime;
        __vdso_clock_gettime;
        gettimeofday;
        __vdso_gettimeofday;
        time;
        __vdso_time;
        getcpu;
        __vdso_getcpu;
    local: *;
    };
}
