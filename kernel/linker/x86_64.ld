/* Kernel na metade alta, onde o Limine o carrega; um segmento por permissão. */
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(kmain)

PHDRS
{
    requests PT_LOAD;
    text     PT_LOAD;
    rodata   PT_LOAD;
    data     PT_LOAD;
    dynamic  PT_DYNAMIC;
}

SECTIONS
{
    . = 0xffffffff80000000;
    __image_start = .;

    /* Marcadores e pedidos ao Limine, nesta ordem (boot/limine). */
    .requests : {
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))
    } :requests

    /* Relocação do Limine (KASLR): o PT_DYNAMIC acha a `.rela.dyn`. Só leitura depois. */
    .dynamic : { *(.dynamic) } :requests :dynamic
    .got : { *(.got .got.*) } :requests

    /* Cada faixa começa e termina em página de 4 KiB: o kernel mapeia cada uma
       com a própria permissão (kernel/src/vm.rs). */
    . = ALIGN(CONSTANT(MAXPAGESIZE));
    __requests_end = .;
    __text_start = .;
    .text : { *(.text .text.*) } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    __text_end = .;
    __rodata_start = .;
    .rodata : { *(.rodata .rodata.*) } :rodata
    .dynsym : { *(.dynsym) } :rodata
    .gnu.hash : { *(.gnu.hash) } :rodata
    .hash : { *(.hash) } :rodata
    .dynstr : { *(.dynstr) } :rodata
    .rela.dyn : { *(.rela.dyn) } :rodata
    /* Tabelas de ponteiros relocados (vtables, handlers): só leitura depois do Limine relocar. */
    .data.rel.ro : {
        __relro_start = .;
        *(.data.rel.ro .data.rel.ro.*)
        __relro_end = .;
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    __rodata_end = .;
    __data_start = .;
    .data : { *(.data .data.*) } :data
    .bss : { *(.bss .bss.*) *(COMMON) } :data

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    __data_end = .;

    /DISCARD/ : { *(.eh_frame*) *(.note .note.*) *(.comment) }
}
