"""Regenerate prefiltered BGRA logo variants from the original community PNG."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent
out = root / "logo-pixels"
out.mkdir(exist_ok=True)
for size in (64, 80, 96, 112, 128, 160, 192):
    raw = bytearray(subprocess.check_output(
        ["convert", str(root / "ddc-logo.png"), "-filter", "Lanczos",
         "-resize", f"{size}x{size}", "-depth", "8", "BGRA:-"]))
    assert len(raw) == size * size * 4
    for i in range(0, len(raw), 4):
        for j in range(3):
            raw[i + j] = (raw[i + j] * raw[i + 3] + 127) // 255
    (out / f"logo{size}.bgra").write_bytes(raw)
