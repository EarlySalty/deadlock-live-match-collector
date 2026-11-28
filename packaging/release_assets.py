import hashlib
import os
from pathlib import Path, PurePosixPath
import re
import sys
import zipfile

EVIDENCE = (
    "run-evidence.txt", "SHA256SUMS", "SHA256SUMS.exe", "Cargo.lock.sha256",
    "build-environment.txt", "dependency-gate.json", "cargo-sbom.cdx.json",
    "clamav-report.txt", "virustotal-report.json", "virustotal-report.md",
    "audit-status.txt", "reproducibility.txt", "pe-evidence.txt",
)
NOTICES = Path(__file__).resolve().parent / "third-party-notices.zip"


def checked_file(path):
    if not path.is_file() or path.is_symlink() or not path.stat().st_size:
        raise ValueError(f"Missing or invalid release file: {path.name}")
    return path


def sha256(path):
    with checked_file(path).open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def notice_contents(path, package=False):
    entries = {}
    with zipfile.ZipFile(checked_file(path)) as archive:
        for entry in archive.infolist():
            name = entry.filename.removeprefix("./")
            if entry.is_dir():
                continue
            if package and not name.startswith("Lizenzen/"):
                continue
            if (not name.startswith("Lizenzen/") or ".." in PurePosixPath(name).parts
                    or name in entries):
                raise ValueError("Invalid or duplicate third-party notice")
            entries[name] = hashlib.sha256(archive.read(entry)).hexdigest()
    if not entries:
        raise ValueError("Missing third-party notices")
    return entries


def verify_notices(archive, notices=NOTICES):
    if notice_contents(Path(archive), package=True) != notice_contents(notices):
        raise ValueError("Tagged third-party notices differ from the Windows package")


def verify_assets(archive, dist=Path("dist"), notices=NOTICES):
    archive = checked_file(Path(archive))
    exe = checked_file(dist / "live-match-collector.exe")
    for name in EVIDENCE:
        checked_file(dist / name)
    for path, checksum in ((archive, "SHA256SUMS"), (exe, "SHA256SUMS.exe")):
        if (dist / checksum).read_text().split() != [sha256(path), path.name]:
            raise ValueError("Release checksum mismatch")
    verify_notices(archive, notices)
    return [exe]


def download_notes(archive, repo, tag, run, artifact, dist=Path("dist"), notices=NOTICES):
    if (repo != "EarlySalty/deadlock-live-match-collector"
            or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag)
            or not re.fullmatch(r"[1-9][0-9]*", run)
            or not re.fullmatch(r"[1-9][0-9]*", artifact)):
        raise ValueError("Invalid release or evidence coordinates")
    exe, = verify_assets(archive, dist, notices)
    base = f"https://github.com/{repo}"
    source = f"{base}/blob/{tag}"
    return (
        "## Download\n\n"
        f"**[Download for Windows (EXE)]({base}/releases/download/{tag}/{exe.name})**\n\n"
        f"[Setup guide (DE)]({source}/packaging/Anleitung-Direkt.md) · "
        f"[Third-party notices]({source}/packaging/third-party-notices.zip)\n\n"
        "<details>\n<summary>Security and verification</summary>\n\n"
        f"SHA-256 (Windows EXE): `{sha256(exe)}`\n\n"
        f"[Build and scan evidence]({base}/actions/runs/{run}/artifacts/{artifact}) "
        f"(retained for 90 days; GitHub sign-in may be required) · "
        f"[Release workflow]({base}/actions/runs/{run}) · "
        f"[Security policy]({source}/docs/release-security.md) · "
        f"[VirusTotal report](https://www.virustotal.com/gui/file/{sha256(exe)})\n\n"
        f"Verify build provenance: `gh attestation verify {exe.name} --repo {repo}`\n\n"
        "</details>\n\n"
    )


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3 or sys.argv[1] not in ("verify", "notes", "notices"):
            raise ValueError("Expected verify, notes or notices and package path")
        if sys.argv[1] == "verify":
            verify_assets(sys.argv[2])
        elif sys.argv[1] == "notices":
            verify_notices(sys.argv[2])
        else:
            print(download_notes(sys.argv[2], *(os.environ[k] for k in (
                "GITHUB_REPOSITORY", "RELEASE_TAG", "GITHUB_RUN_ID", "RELEASE_EVIDENCE_ARTIFACT_ID"
            ))), end="")
    except (ValueError, KeyError, OSError, zipfile.BadZipFile, RuntimeError) as error:
        sys.exit(f"Release asset verification failed: {error}")
