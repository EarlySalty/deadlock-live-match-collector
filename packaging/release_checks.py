import json
import os
import re
import subprocess
import sys

ENDPOINT = "https://deutsche-deadlock-community.de/twitch/api/v3/live-match"
MAX_BODY_BYTES = 32768
PREFLIGHT_PAYLOAD = {
    "schema_version": 3,
    "writer_id": "123e4567-e89b-42d3-a456-426614174000",
    "sequence": 1,
    "match_epoch": 1,
    "hero": "Infernus",
    "game_seconds": 42,
}


def encode_preflight_payload():
    body = json.dumps(PREFLIGHT_PAYLOAD, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(body) > MAX_BODY_BYTES:
        raise ValueError(f"API preflight body exceeds {MAX_BODY_BYTES} UTF-8 bytes")
    return body


def preflight(url=ENDPOINT, timeout=10):
    protocol = "=https" if url == ENDPOINT else "=http,https"
    result = subprocess.run(
        ["curl", "--disable", "--silent", "--show-error", "--proto", protocol,
         "--connect-timeout", str(min(5, timeout)), "--max-time", str(timeout),
         "--output", os.devnull, "--write-out", "%{http_code}", "--request", "POST",
         "--header", "Content-Type: application/json",
         "--data-binary", "@-", url],
        input=encode_preflight_payload().decode("utf-8"), encoding="utf-8",
        capture_output=True, text=True, timeout=timeout + 5,
    )
    if result.returncode != 0:
        raise ValueError(f"API preflight transport failed (curl {result.returncode})")
    if result.stdout != "401":
        raise ValueError(f"API preflight expected HTTP 401, received {result.stdout}")
    print("API v3 preflight: HTTP 401 (unauthenticated routing/auth only; not schema or E2E evidence)")


def command_json(*args):
    return json.loads(subprocess.check_output(args, text=True))


def successful_run(payload, sha, repository):
    if not isinstance(payload, dict):
        raise ValueError("Invalid CI API response")
    runs = payload.get("workflow_runs")
    count = payload.get("total_count")
    if not isinstance(runs, list) or type(count) is not int or count != len(runs) or not runs:
        raise ValueError("Missing, incomplete or excessive CI run history")
    for run in runs:
        if (run.get("head_sha") != sha or run.get("head_branch") != "main"
                or run.get("event") != "push"
                or run.get("head_repository", {}).get("full_name") != repository
                or run.get("path") != ".github/workflows/ci.yml"
                or type(run.get("run_number")) is not int
                or type(run.get("run_attempt")) is not int
                or type(run.get("id")) is not int):
            raise ValueError("Unexpected CI run identity")
    run = max(runs, key=lambda item: (item["run_number"], item["run_attempt"]))
    if run.get("status") != "completed" or run.get("conclusion") != "success":
        raise ValueError("Latest CI run for the exact main commit is not successful")
    return run


def successful_job(payload, run):
    jobs = payload.get("jobs")
    if not isinstance(jobs, list) or payload.get("total_count") != len(jobs):
        raise ValueError("Incomplete CI job history")
    builds = [job for job in jobs if job.get("name") == "build"]
    if len(builds) != 1:
        raise ValueError("Missing or ambiguous CI build job")
    job = builds[0]
    if (job.get("run_id") != run["id"] or job.get("run_attempt", run["run_attempt"]) != run["run_attempt"]
            or job.get("head_sha") != run["head_sha"]
            or job.get("status") != "completed" or job.get("conclusion") != "success"):
        raise ValueError("CI build job did not succeed for the exact run attempt")
    return job


def require_ci(sha, repository):
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Invalid commit SHA")
    root = f"repos/{repository}/actions"
    runs = command_json("gh", "api", "--method", "GET",
                        f"{root}/workflows/ci.yml/runs",
                        "-f", f"head_sha={sha}", "-f", "branch=main",
                        "-f", "event=push", "-f", "per_page=100")
    run = successful_run(runs, sha, repository)
    jobs = command_json("gh", "api", f"{root}/runs/{run['id']}/attempts/{run['run_attempt']}/jobs?per_page=100")
    job = successful_job(jobs, run)
    print(f"Exact-SHA CI passed: {sha}, run {run['id']}, attempt {run['run_attempt']}, job {job['id']}")


def require_tag(tag, sha, ref, version, remote_refs):
    if ref != "refs/heads/main" or not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Release workflow must run from main")
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag) or tag != f"v{version}":
        raise ValueError("Tag does not match the package version")
    expected_ref = f"refs/tags/{tag}"
    refs = {}
    for line in remote_refs.splitlines():
        commit, name = line.split("\t")
        if name not in ("refs/heads/main", expected_ref, expected_ref + "^{}") or name in refs:
            raise ValueError("Unexpected remote tag reference")
        if not re.fullmatch(r"[0-9a-f]{40}", commit):
            raise ValueError("Invalid tag object SHA")
        refs[name] = commit
    if refs.get("refs/heads/main") != sha:
        raise ValueError("Remote main no longer equals the dispatched release commit")
    if expected_ref not in refs or refs.get(expected_ref + "^{}", refs[expected_ref]) != sha:
        raise ValueError("Remote tag does not resolve to the dispatched main commit")


def check_main():
    sha = os.environ["GITHUB_SHA"]
    if os.environ["GITHUB_REF"] != "refs/heads/main" or not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Release workflow must run from main")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    remote = subprocess.check_output(["git", "ls-remote", "origin", "refs/heads/main"], text=True).strip()
    if head != sha or remote != f"{sha}\trefs/heads/main":
        raise ValueError("Checkout or remote main changed")
    metadata = command_json("cargo", "metadata", "--locked", "--no-deps", "--format-version", "1")
    version, = [p["version"] for p in metadata["packages"] if p["name"] == "live-match-collector"]
    if os.environ["RELEASE_TAG"] != f"v{version}":
        raise ValueError("Tag does not match package version")
    print(f"Exact main checkout and version verified: {sha}, v{version}")


def check_tag():
    tag = os.environ["RELEASE_TAG"]
    sha = os.environ["GITHUB_SHA"]
    ref = os.environ["GITHUB_REF"]
    metadata = command_json("cargo", "metadata", "--locked", "--no-deps", "--format-version", "1")
    version, = [p["version"] for p in metadata["packages"] if p["name"] == "live-match-collector"]
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag):
        raise ValueError("Invalid release tag")
    remote = subprocess.check_output(
        ["git", "ls-remote", "origin", "refs/heads/main", f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}"],
        text=True,
    )
    require_tag(tag, sha, ref, version, remote)
    subprocess.run(["git", "merge-base", "--is-ancestor", sha, "origin/main"], check=True)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if head != sha:
        raise ValueError("Checkout does not match the release commit")
    print(f"Remote main and tag {tag} resolve to checked-out commit {sha}")


def main():
    try:
        mode, = sys.argv[1:]
        if mode == "preflight":
            preflight()
        elif mode == "ci":
            require_ci(os.environ["GITHUB_SHA"], os.environ["GITHUB_REPOSITORY"])
        elif mode == "main":
            check_main()
        elif mode == "tag":
            check_tag()
        else:
            raise ValueError("Unknown release check")
    except (ValueError, KeyError, TypeError, AttributeError, OSError, subprocess.SubprocessError) as error:
        sys.exit(f"Release check failed: {error}")


if __name__ == "__main__":
    main()
