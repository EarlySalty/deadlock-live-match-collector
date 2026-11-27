"""Private, immutable, digest-bound transfer of a release candidate.
No binary is published as a GitHub Actions artifact.
"""
import base64
import datetime as dt
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile

BASE = ("live-match-collector.exe", "SHA256SUMS", "SHA256SUMS.exe",
        "clamav-report.txt", "build-environment.txt", "cargo-sbom.cdx.json",
        "dependency-gate.json", "Cargo.lock.sha256", "run-evidence.txt",
        "audit-status.txt", "reproducibility.txt", "pe-evidence.txt")
REPORT = ("virustotal-report.md", "virustotal-report.json")
LIMIT = 64 * 1024 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def coordinates(stage):
    sha, run, attempt, tag = (os.environ[k] for k in
        ("GITHUB_SHA", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT", "RELEASE_TAG"))
    if (not re.fullmatch(r"[a-f0-9]{40}", sha)
            or not all(re.fullmatch(r"[1-9][0-9]*", v) for v in (run, attempt))
            or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag)
            or stage not in ("build", "scan")):
        raise ValueError("invalid release coordinates")
    return {"sha": sha, "run": run, "attempt": attempt, "tag": tag, "stage": stage}


def names(meta):
    return (*BASE, f"live-match-collector-{meta['tag'][1:]}-win64.zip",
            *(REPORT if meta["stage"] == "scan" else ()))


def pack(dist, meta):
    files = {}
    for name in names(meta):
        path = dist / name
        if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= LIMIT:
            raise ValueError("invalid release member")
        files[name] = path.read_bytes()
    manifest = dict(meta, files={n: digest(b) for n, b in files.items()})
    files["manifest.json"] = json.dumps(manifest, sort_keys=True).encode()
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w") as tar:
        for name, data in sorted(files.items()):
            member = tarfile.TarInfo(name)
            member.size = len(data)
            member.mode = 0o600
            tar.addfile(member, io.BytesIO(data))
    blob = out.getvalue()
    if len(blob) > LIMIT:
        raise ValueError("handoff exceeds size bound")
    return blob


def unpack(blob, dist, meta):
    if not 0 < len(blob) <= LIMIT:
        raise ValueError("invalid handoff size")
    expected = set(names(meta)) | {"manifest.json"}
    files = {}
    with tarfile.open(fileobj=io.BytesIO(blob), mode="r:") as tar:
        for member in tar:
            if (member.name not in expected or member.name in files
                    or not member.isfile() or not 0 < member.size <= LIMIT):
                raise ValueError("invalid handoff member")
            files[member.name] = tar.extractfile(member).read()
    if set(files) != expected:
        raise ValueError("incomplete handoff")
    manifest = json.loads(files.pop("manifest.json"))
    if manifest != dict(meta, files={n: digest(b) for n, b in files.items()}):
        raise ValueError("handoff manifest mismatch")
    dist.mkdir(mode=0o700, parents=False, exist_ok=False)
    for name, data in files.items():
        with (dist / name).open("xb") as f:
            f.write(data)


def aws(*args):
    region = os.environ["AWS_REGION"]
    if not re.fullmatch(r"[a-z]{2}(?:-[a-z]+)+-[0-9]+", region):
        raise ValueError("invalid storage region")
    if not os.environ.get("AWS_ACCESS_KEY_ID") or not os.environ.get("AWS_SECRET_ACCESS_KEY"):
        raise ValueError("scoped handoff credentials missing")
    env = dict(os.environ, AWS_EC2_METADATA_DISABLED="true")
    return json.loads(subprocess.check_output(
        ["aws", "--no-cli-pager", "--region", region, "--output", "json", "s3api", *args],
        text=True, env=env))


def storage(meta, sha):
    bucket = os.environ["RELEASE_HANDOFF_BUCKET"]
    owner = os.environ["RELEASE_HANDOFF_ACCOUNT"]
    if (not re.fullmatch(r"[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]", bucket)
            or not re.fullmatch(r"[0-9]{12}", owner)
            or not re.fullmatch(r"[a-f0-9]{64}", sha)):
        raise ValueError("invalid handoff storage coordinates")
    key = "collector/" + "/".join(meta[k] for k in ("sha", "run", "attempt", "stage")) + "/" + sha + ".tar"
    return ["--bucket", bucket, "--expected-bucket-owner", owner, "--key", key]


def checksum(sha):
    return base64.b64encode(bytes.fromhex(sha)).decode()


def check_head(head, version, sha, size):
    if (head.get("VersionId") != version or head.get("ChecksumSHA256") != checksum(sha)
            or head.get("ContentLength") != size
            or head.get("ObjectLockMode") != "COMPLIANCE"
            or head.get("ServerSideEncryption") not in ("AES256", "aws:kms")):
        raise ValueError("storage integrity or immutability not verified")
    until = dt.datetime.fromisoformat(head["ObjectLockRetainUntilDate"].replace("Z", "+00:00"))
    if until <= dt.datetime.now(dt.timezone.utc):
        raise ValueError("object retention expired")


def put(stage, dist):
    meta = coordinates(stage)
    blob = pack(dist, meta)
    sha = digest(blob)
    location = storage(meta, sha)
    until = (dt.datetime.now(dt.timezone.utc) + dt.timedelta(days=7)).isoformat()
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "bundle.tar"
        path.write_bytes(blob)
        receipt = aws("put-object", *location, "--body", str(path),
                      "--if-none-match", "*", "--server-side-encryption", "AES256",
                      "--checksum-algorithm", "SHA256", "--checksum-sha256", checksum(sha),
                      "--object-lock-mode", "COMPLIANCE",
                      "--object-lock-retain-until-date", until)
    version = receipt["VersionId"]
    if not valid_version(version):
        raise ValueError("missing immutable object version")
    head = aws("head-object", *location, "--version-id", version, "--checksum-mode", "ENABLED")
    check_head(head, version, sha, len(blob))
    with open(os.environ["GITHUB_OUTPUT"], "a") as f:
        f.write(f"digest={sha}\nversion={version}\n")
    print(f"Private {stage} handoff SHA-256: {sha}")


def valid_version(version):
    return isinstance(version, str) and version != "null" and bool(re.fullmatch(r"[A-Za-z0-9._~+/=-]{1,1024}", version))


def get(stage, dist):
    meta = coordinates(stage)
    sha, version = os.environ["HANDOFF_DIGEST"], os.environ["HANDOFF_VERSION"]
    if not valid_version(version):
        raise ValueError("invalid immutable object version")
    location = storage(meta, sha)
    head = aws("head-object", *location, "--version-id", version, "--checksum-mode", "ENABLED")
    size = head["ContentLength"]
    if type(size) is not int or not 0 < size <= LIMIT:
        raise ValueError("invalid remote handoff size")
    check_head(head, version, sha, size)
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "bundle.tar"
        aws("get-object", *location, "--version-id", version, "--checksum-mode", "ENABLED", str(path))
        if path.stat().st_size != size:
            raise ValueError("handoff size mismatch")
        blob = path.read_bytes()
    if digest(blob) != sha:
        raise ValueError("handoff digest mismatch")
    unpack(blob, dist, meta)


if __name__ == "__main__":
    try:
        action, stage = sys.argv[1:]
        if action not in ("put", "get"):
            raise ValueError("invalid handoff action")
        (put if action == "put" else get)(stage, Path("dist"))
    except (ValueError, KeyError, TypeError, OSError, tarfile.TarError, subprocess.SubprocessError):
        sys.exit("Private release handoff failed; no publication authorized")
