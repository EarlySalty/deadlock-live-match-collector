"""Owner-authorized v0.3.0 preview only; stable source gate stays closed."""
import json
import os
import re
import sys


def allowed(env, event):
    sha = env.get("GITHUB_SHA", "")
    repo = env.get("GITHUB_REPOSITORY")
    if not (repo == "EarlySalty/deadlock-live-match-collector"
            and env.get("GITHUB_REF") == "refs/heads/main"
            and env.get("RELEASE_MODE") == "prerelease"
            and env.get("RELEASE_TAG") == "v0.3.0"
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
        with open(os.environ["GITHUB_EVENT_PATH"]) as f:
            event = json.load(f)
        if not allowed(os.environ, event):
            raise ValueError("scope denied")
    except (ValueError, OSError, KeyError, TypeError, AttributeError):
        sys.exit("Release blocked: no authorized live source or owner-enabled exact desktop-preview scope")
