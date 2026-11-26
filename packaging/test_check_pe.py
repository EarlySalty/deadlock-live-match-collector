import struct
import unittest

from check_pe import check


def header():
    # Minimal header-only fixture, deliberately not an executable.
    data = bytearray(512)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 128)
    data[128:132] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 132, 0x8664, 1, 0, 0, 0, 240, 0)
    struct.pack_into("<H", data, 152, 0x20B)
    struct.pack_into("<H", data, 222, 0x160)
    struct.pack_into("<I", data, 428, 0x60000020)
    return data


class CheckPeTests(unittest.TestCase):
    def test_expected_flags(self):
        self.assertEqual(check(header())["machine"], "x64")

    def test_reject_machine_timestamp_and_missing_flags(self):
        for offset, fmt, value in [(132, "<H", 0x14C), (136, "<I", 1), (222, "<H", 0x140)]:
            with self.subTest(offset=offset):
                data = header()
                struct.pack_into(fmt, data, offset, value)
                with self.assertRaises(ValueError):
                    check(data)

    def test_reject_writable_executable_section(self):
        data = header()
        struct.pack_into("<I", data, 428, 0xE0000020)
        with self.assertRaises(ValueError):
            check(data)
