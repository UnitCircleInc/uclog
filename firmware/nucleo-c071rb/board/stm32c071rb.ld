/* STM32C071RB: 128 KiB flash, 24 KiB SRAM. */

ENTRY(Reset_Handler)

MEMORY
{
  FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 128K
  RAM (rwx)   : ORIGIN = 0x20000000, LENGTH = 24K
  /* Log strings (not loaded). Target digit 0: 0xF0000000 + 0x100000 * target. */
  LOGDATA     : ORIGIN = 0xF0000000, LENGTH = 1M
}

_estack = ORIGIN(RAM) + LENGTH(RAM);
_stack_size = 2K;
_heap_min = 1K; /* newlib-nano stdio (stdout buffer, FILE state) */

SECTIONS
{
  .isr_vector :
  {
    KEEP(*(.isr_vector))
  } > FLASH

  .text :
  {
    *(.text .text.*)
    *(.rodata .rodata.*)
    KEEP(*(.init))
    KEEP(*(.fini))
    . = ALIGN(4);
  } > FLASH

  .ARM.exidx :
  {
    *(.ARM.exidx* .gnu.linkonce.armexidx.*)
  } > FLASH

  .preinit_array : { KEEP(*(.preinit_array*)) } > FLASH
  .init_array :
  {
    KEEP(*(SORT(.init_array.*)))
    KEEP(*(.init_array*))
  } > FLASH
  .fini_array : { KEEP(*(.fini_array*)) } > FLASH

  /* SHA-512 of the image with this field zeroed (device/tools/hash.py). */
  .apphash :
  {
    . = ALIGN(4);
    KEEP(*(.apphash))
  } > FLASH

  .data :
  {
    . = ALIGN(4);
    _sdata = .;
    *(.data .data.*)
    . = ALIGN(4);
    _edata = .;
  } > RAM AT > FLASH
  _sidata = LOADADDR(.data);

  .bss (NOLOAD) :
  {
    . = ALIGN(4);
    _sbss = .;
    *(.bss .bss.*)
    *(COMMON)
    . = ALIGN(4);
    _ebss = .;
  } > RAM

  /* Heap from _end up to the stack. */
  . = ALIGN(8);
  _end = .;
  _heap_limit = _estack - _stack_size;
  ASSERT(_end + _heap_min <= _heap_limit, "RAM overflow")

  INCLUDE log.ld
}
