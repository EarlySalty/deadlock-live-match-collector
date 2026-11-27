"""Narrow, fail-closed exception to the stable source gate."""
import os
import re
import sys


def allowed(env):
    sha = env.get("GITHUB_SHA", "")
    return (env.get("GITHUB_REF") == "refs/heads/main"
            and env.get("GITHUB_EVENT_NAME") == "workflow_dispatch"
            and env.get("RELEASE_MODE") == "prerelease"
            and env.get("RELEASE_TAG") == "v0.3.0"
            and bool(re.fullmatch(r"[a-f0-9]{40}", sha))
            and env.get("RELEASE_PREVIEW_SHA") == sha
            and env.get("RELEASE_HANDOFF_APPROVED") == "true")


if __name__ == "__main__":
    if not allowed(os.environ):
        sys.exit("Release blocked: no authorized complete live source or exact approved desktop-preview exception/private handoff")
