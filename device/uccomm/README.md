# ucComm (pinned copy)

COBS, CBOR, CRC-32C and the ring buffer (`cb`) from
[UnitCircleInc/ucComm](https://github.com/UnitCircleInc/ucComm) `src/`, tag **v5** (`a00bdc4`),
copied unchanged, for Makefile and CMake builds and for the release archive. Zephyr builds use the
ucComm Zephyr module when the workspace has one (`west.yml` pins the same tag), and these files
otherwise.

To update: copy `src/*.c` and `src/*.h` from the new tag, change the tag above and in `../west.yml`,
and rebuild the [test firmware](https://github.com/UnitCircleInc/uclog/blob/main/firmware).
