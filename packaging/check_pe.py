"""Read-only static PE checks; never change or execute the binary."""
import hashlib
import struct
import sys
from pathlib import Path


def check(data):
    if data[:2] != b"MZ":
        raise ValueError("missing DOS header")
    off, = struct.unpack_from("<I", data, 0x3C)
    if data[off:off+4] != b"PE\0\0":
        raise ValueError("missing PE header")
    machine, sections, timestamp, _, _, size, _ = struct.unpack_from("<HHIIIHH", data, off+4)
    optional = off + 24
    magic, = struct.unpack_from("<H", data, optional)
    flags, = struct.unpack_from("<H", data, optional+70)
    if machine != 0x8664 or magic != 0x20B or not sections or size < 112:
        raise ValueError("not Windows x64 PE32+")
    if flags & 0x160 != 0x160:
        raise ValueError("missing ASLR, high entropy VA or NX")
    if timestamp != 0:
        raise ValueError("non-deterministic COFF timestamp")
    for i in range(sections):
        offset = optional + size + i*40
        characteristics, = struct.unpack_from("<I", data, offset+36)
        if characteristics & 0xA0000000 == 0xA0000000:
            raise ValueError("writable and executable section")
    return {"machine": "x64", "format": "PE32+", "coff_timestamp": timestamp,
            "dll_characteristics": hex(flags), "sections": sections}


if __name__ == "__main__":
    try:
        data = Path(sys.argv[1]).read_bytes()
        print("exe_sha256=" + hashlib.sha256(data).hexdigest())
        for key, value in check(data).items():
            print(f"{key}={value}")
    except (OSError, ValueError, struct.error, IndexError) as exc:
        print(f"PE check failed: {exc}", file=sys.stderr)
        sys.exit(1)
