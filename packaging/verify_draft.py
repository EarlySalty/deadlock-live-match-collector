"""Verify all remote draft assets against the local, verified release files."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

from release_assets import verify_assets


def expected_assets(archive, dist=Path("dist")):
    files = verify_assets(archive, dist)
    expected = {}
    for path in files:
        if path.name in expected or not path.is_file() or path.is_symlink():
            raise ValueError("Missing or ambiguous local release asset")
        with path.open("rb") as handle:
            digest = hashlib.file_digest(handle, "sha256").hexdigest()
        expected[path.name] = (path.stat().st_size, f"sha256:{digest}")
    return expected


def verify_draft(release, assets, tag, expected, prerelease, draft=True):
    release_id = release.get("id")
    if (type(release_id) is not int or release_id <= 0
            or release.get("tag_name") != tag or release.get("draft") is not draft
            or release.get("prerelease") is not prerelease):
        raise ValueError("Unexpected release identity or mode")
    if not isinstance(assets, list) or len(assets) != len(expected):
        raise ValueError("Missing or unexpected remote release assets")
    seen, ids = set(), set()
    for asset in assets:
        name, asset_id = asset.get("name"), asset.get("id")
        if (name not in expected or name in seen or type(asset_id) is not int
                or asset_id <= 0 or asset_id in ids
                or asset.get("state") != "uploaded"
                or type(asset.get("size")) is not int
                or (asset["size"], asset.get("digest")) != expected[name]):
            raise ValueError("Remote asset identity or SHA-256 mismatch")
        seen.add(name)
        ids.add(asset_id)
    return release_id


def main():
    args = sys.argv[1:]
    archive = args[0]
    published = len(args) == 3 and args[1] == "--published" and bool(re.fullmatch(r"[1-9][0-9]*", args[2]))
    if len(args) != 1 and not published:
        raise ValueError("invalid release verification arguments")
    repo, tag, mode = (os.environ[k] for k in ("GITHUB_REPOSITORY", "RELEASE_TAG", "RELEASE_MODE"))
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo) or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag) or mode not in ("stable", "prerelease"):
        raise ValueError("Invalid release coordinates")
    expected = expected_assets(archive)
    # Drafts require an authorized token. Enumerate and fail if the matching tag
    # is absent or ambiguous; never use a public tag lookup as draft identity.
    releases = json.loads(subprocess.check_output(
        ["gh", "api", "--paginate", "--slurp", f"repos/{repo}/releases?per_page=100"], text=True))
    matches = [r for page in releases for r in page if r.get("tag_name") == tag]
    if len(matches) != 1:
        raise ValueError("Missing or ambiguous release")
    release = matches[0]
    release_id = release.get("id")
    if type(release_id) is not int or release_id <= 0 or (published and release_id != int(args[2])):
        raise ValueError("Invalid release ID")
    pages = json.loads(subprocess.check_output(
        ["gh", "api", "--paginate", "--slurp", f"repos/{repo}/releases/{release_id}/assets?per_page=100"], text=True))
    assets = [a for page in pages for a in page]
    print(verify_draft(release, assets, tag, expected, mode == "prerelease", draft=not published))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, TypeError, AttributeError, OSError, subprocess.SubprocessError):
        sys.exit("Remote draft verification failed")
