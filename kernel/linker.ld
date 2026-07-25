OUTPUT_FORMAT(elf64-x86-64)
ENTRY(kmain)

PHDRS
{
    text    PT_LOAD;
    rodata  PT_LOAD;
    data    PT_LOAD;
    note    PT_NOTE;
}

SECTIONS
{

    . = 0xffffffff80000000;

    .text : {
        *(.text .text.*)
    } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .data : {
        *(.data .data.*)
        *(.got .got.*)

        KEEP(*(.requests))
    } :data

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    .note.pvh : ALIGN(4) {
        KEEP(*(.note.pvh))
    } :note

    /DISCARD/ : {
        *(.eh_frame*)
    }
}