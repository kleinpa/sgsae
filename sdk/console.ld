/* Cartridge layout. Must agree with the memory map in sgsae/console.rs. */
ENTRY(_start)

MEMORY {
  ROM (rx)  : ORIGIN = 0x00000000, LENGTH = 1M
  RAM (rwx) : ORIGIN = 0x02000000, LENGTH = 64K
}

SECTIONS {
  .text : {
    KEEP(*(.text._start))
    *(.text .text.*)
  } > ROM

  .rodata : ALIGN(4) { *(.rodata .rodata.*) } > ROM

  /* .data is stored in ROM and copied to RAM by _start. */
  .data : ALIGN(4) {
    __data_start = .;
    *(.data .data.*)
    . = ALIGN(4);
    __data_end = .;
  } > RAM AT > ROM
  __data_rom = LOADADDR(.data);

  .bss (NOLOAD) : ALIGN(4) {
    __bss_start = .;
    *(.bss .bss.* COMMON)
    . = ALIGN(4);
    __bss_end = .;
  } > RAM

  __stack_top = ORIGIN(RAM) + LENGTH(RAM);

  /DISCARD/ : { *(.eh_frame .eh_frame_hdr .comment .riscv.attributes) }
}
