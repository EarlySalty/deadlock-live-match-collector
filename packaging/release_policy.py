"""Owner-authorized automatic desktop previews; stable source gate stays closed."""
import json
import os
import re
import sys
import tomllib
from pathlib import Path


VERSION = r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"


def package_tag(path=Path(__file__).resolve().parent.parent / "Cargo.toml"):
    with path.open("rb") as f:
        package = tomllib.load(f)["package"]
    version = package["version"]
    if package["name"] != "live-match-collector" or not re.fullmatch(VERSION, version):
        raise ValueError("Invalid collector version")
    return f"v{version}"


def allowed(env, event, expected_tag):
    sha = env.get("GITHUB_SHA", "")
    repo = env.get("GITHUB_REPOSITORY")
    if not (repo == "EarlySalty/deadlock-live-match-collector"
            and env.get("GITHUB_REF") == "refs/heads/main"
            and env.get("RELEASE_MODE") == "prerelease"
            and bool(re.fullmatch("v" + VERSION, expected_tag))
            and env.get("RELEASE_TAG") == expected_tag
            and env.get("RELEASE_AUTO_PREVIEW") == "true"
            and bool(re.fullmatch(r"[a-f0-9]{40}", sha))
            and env.get("RELEASE_PREVIEW_ENABLED") == "true"):
        return False
    if env.get("GITHUB_EVENT_NAME") == "workflow_dispatch":
        return event.get("ref") == "refs/heads/main"
    if env.get("GITHUB_EVENT_NAME") != "workflow_run":
        return False
    run = event.get("workflow_run", {})
    return (event.get("action") == "completed" and run.get("event") == "push"
            and run.get("conclusion") == "success"
            and run.get("head_branch") == "main"
            and run.get("head_sha") == sha
            and run.get("head_repository", {}).get("full_name") == repo
            and run.get("path") == ".github/workflows/ci.yml")


if __name__ == "__main__":
    try:
        tag = package_tag()
        if sys.argv[1:] == ["--tag"]:
            print(tag)
            sys.exit(0)
        if sys.argv[1:]:
            raise ValueError("Unexpected arguments")
        with open(os.environ["GITHUB_EVENT_PATH"]) as f:
            event = json.load(f)
        if not allowed(os.environ, event, tag):
            raise ValueError("scope denied")
    except (ValueError, OSError, KeyError, TypeError, AttributeError):
        sys.exit("Release blocked: no authorized live source or owner-enabled automatic desktop-preview scope")
