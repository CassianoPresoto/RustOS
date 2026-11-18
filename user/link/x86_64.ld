/* Linker script dos programas de usuário do x86_64 (SDD 17, "O que muda no linker e no alvo").
   ET_EXEC em 0x400000, uma página por permissão: cabeçalhos e .rodata (R), .text (RX), .data e .bss (RW).
   O `core/elf` recusa W+X, PT_TLS, PT_DYNAMIC e segmentos fora de ordem. */
ENTRY(_start)
EXTERN(_start)

SECTIONS
{
    . = 0x400000 + SIZEOF_HEADERS;

    .rodata : ALIGN(16) { *(.rodata .rodata.*) }

    . = ALIGN(0x1000);
    .text : { *(.text._start) *(.text .text.*) }

    . = ALIGN(0x1000);
    .data : { *(.data .data.*) *(.got .got.*) }
    .bss : { *(.bss .bss.*) *(COMMON) }

    /DISCARD/ : { *(.eh_frame*) *(.comment) *(.note*) *(.gcc_except_table*) }
}
