import hashlib
from pathlib import Path
import sys
import zipfile

EVIDENCE = (
    "run-evidence.txt", "SHA256SUMS", "SHA256SUMS.exe", "Cargo.lock.sha256",
    "build-environment.txt", "dependency-gate.json", "cargo-sbom.cdx.json",
    "clamav-report.txt", "virustotal-report.json", "virustotal-report.md",
    "audit-status.txt", "reproducibility.txt", "pe-evidence.txt",
)


def checked_file(path):
    if not path.is_file() or path.is_symlink() or not path.stat().st_size:
        raise ValueError(f"Missing or invalid release file: {path.name}")
    return path


def download_files(archive, dist):
    files = [Path(archive), dist / "live-match-collector.exe", dist / "security-evidence.zip"]
    if len({path.name for path in files}) != len(files):
        raise ValueError("Ambiguous download filenames")
    return [checked_file(path) for path in files]


def checksum_bytes(files):
    lines = []
    for path in files:
        with path.open("rb") as handle:
            digest = hashlib.file_digest(handle, "sha256").hexdigest()
        lines.append(f"{digest}  {path.name}\n")
    return "".join(lines).encode("ascii")


def build_assets(archive, dist=Path("dist")):
    sources = [checked_file(dist / name) for name in EVIDENCE]
    bundle = dist / "security-evidence.zip"
    with zipfile.ZipFile(bundle, "x", compression=zipfile.ZIP_DEFLATED) as output:
        for source in sources:
            output.write(source, source.name)
    public = dist / "public"
    public.mkdir(exist_ok=True)
    with (public / "SHA256SUMS").open("xb") as output:
        output.write(checksum_bytes(download_files(archive, dist)))
    return verify_assets(archive, dist)


def verify_assets(archive, dist=Path("dist")):
    files = download_files(archive, dist)
    with zipfile.ZipFile(files[2]) as bundle:
        if sorted(bundle.namelist()) != sorted(EVIDENCE):
            raise ValueError("Missing, duplicate or unexpected evidence files")
        for name in EVIDENCE:
            if bundle.read(name) != checked_file(dist / name).read_bytes():
                raise ValueError(f"Evidence mismatch: {name}")
    sums = checked_file(dist / "public" / "SHA256SUMS")
    if sums.read_bytes() != checksum_bytes(files):
        raise ValueError("Public checksum mismatch")
    return [*files, sums]


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3 or sys.argv[1] not in ("build", "verify"):
            raise ValueError("Expected build or verify and package path")
        action = build_assets if sys.argv[1] == "build" else verify_assets
        action(sys.argv[2])
    except (ValueError, OSError, zipfile.BadZipFile, RuntimeError) as error:
        sys.exit(f"Release asset verification failed: {error}")
