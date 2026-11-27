"""Ensure embedded variants have valid pixel sizes and premultiplied alpha."""
import unittest
from pathlib import Path
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent

class RenderingAssetsTest(unittest.TestCase):
    def test_logo_variants_are_premultiplied_bgra(self):
        for size in (16, 24, 32, 64, 80, 96, 112, 128, 160, 192):
            with self.subTest(size=size):
                data = (ROOT / "logo-pixels" / f"logo{size}.bgra").read_bytes()
                self.assertEqual(len(data), size * size * 4)
                self.assertTrue(all(max(data[i:i+3]) <= data[i+3]
                                    for i in range(0, len(data), 4)))

    def test_manifest_explicitly_enables_per_monitor_v2_without_elevation(self):
        doc = ET.parse(ROOT / "collector.manifest")
        dpi = doc.find(".//{http://schemas.microsoft.com/SMI/2016/WindowsSettings}dpiAwareness")
        self.assertEqual(dpi.text, "PerMonitorV2, PerMonitor")
        level = doc.find(".//{urn:schemas-microsoft-com:asm.v3}requestedExecutionLevel")
        self.assertEqual(level.attrib, {"level": "asInvoker", "uiAccess": "false"})
