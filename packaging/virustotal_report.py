"""Fail-closed exact-file VirusTotal gate; no credentials or raw responses retained."""
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
import uuid
from collections import Counter
from pathlib import Path

BASE = "https://www.virustotal.com/api/v3"
MAX_BYTES = 32_000_000
MAX_AGE = 7 * 86400
MIN_VERDICTS = 60
CATEGORIES = {"malicious", "suspicious", "undetected", "harmless",
              "timeout", "confirmed-timeout", "failure", "type-unsupported"}

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None

class GateError(ValueError):
    pass

def request(key, path, data=None, content_type=None):
    headers = {"x-apikey": key, "Accept": "application/json"}
    if content_type:
        headers["Content-Type"] = content_type
    req = urllib.request.Request(BASE + path, data=data, headers=headers)
    for attempt in range(3):
        try:
            with urllib.request.build_opener(NoRedirect()).open(req, timeout=40) as response:
                raw = response.read(8_000_001)
                if len(raw) > 8_000_000:
                    raise GateError("response too large")
                return json.loads(raw)["data"]
        except urllib.error.HTTPError as error:
            # Never retry a submission of uncertain outcome. Respect quota; no retry storm.
            if data is not None or error.code not in (429, 500, 502, 503, 504) or attempt == 2:
                raise
            retry = error.headers.get("Retry-After", "60") if error.headers else "60"
            if not retry.isdecimal() or int(retry) > 120:
                raise
            time.sleep(max(60, int(retry)))

def safe_label(value):
    if not isinstance(value, str) or not 1 <= len(value) <= 200 or not value.isprintable():
        raise GateError("invalid engine label")
    return value

def validate(file, sha, now):
    if file.get("id") != sha or file.get("type") != "file":
        raise GateError("report identity mismatch")
    attrs = file["attributes"]
    if attrs.get("sha256") != sha:
        raise GateError("attribute hash mismatch")
    date = attrs.get("last_analysis_date")
    stats = attrs.get("last_analysis_stats")
    results = attrs.get("last_analysis_results")
    if type(date) is not int or not isinstance(stats, dict) or set(stats) != CATEGORIES:
        raise GateError("invalid report metadata")
    if any(type(v) is not int or v < 0 for v in stats.values()):
        raise GateError("invalid statistics")
    if not isinstance(results, dict) or not results:
        raise GateError("missing engine results")
    engines = []
    counts = Counter()
    for name, entry in sorted(results.items()):
        name = safe_label(name)
        category = entry["category"]
        if category not in CATEGORIES:
            raise GateError("invalid engine category")
        result = entry.get("result")
        if category in ("malicious", "suspicious") or result is not None:
            result = safe_label(result)
        counts[category] += 1
        engines.append({"engine": name, "category": category, "result": result})
    if any(counts[k] != stats[k] for k in CATEGORIES):
        raise GateError("engine counts inconsistent")
    # Preserve valid detections even if old or coverage is inadequate.
    status = "passed"
    if counts["malicious"] or counts["suspicious"]:
        status = "detections"
    elif not 0 <= now - date <= MAX_AGE:
        status = "stale-or-future-report"
    elif counts["undetected"] + counts["harmless"] < MIN_VERDICTS:
        status = "insufficient-coverage"
    return {"status": status, "analysis_date": date, "stats": stats, "engines": engines}

def scan(key, blob, sha, allow_upload=False):
    try:
        file = request(key, "/files/" + sha)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        if not allow_upload:
            raise GateError("report absent; submission not authorized")
        boundary = uuid.uuid4().hex
        body = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="live-match-collector.exe"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode()
                + blob + f'\r\n--{boundary}--\r\n'.encode())
        analysis = request(key, "/files", body, "multipart/form-data; boundary=" + boundary)
        analysis_id = analysis["id"]
        if not re.fullmatch(r"[A-Za-z0-9_=-]{1,256}", analysis_id):
            raise GateError("invalid analysis ID")
        for _ in range(8):
            time.sleep(60)
            result = request(key, "/analyses/" + analysis_id)
            status = result["attributes"]["status"]
            if result.get("id") != analysis_id:
                raise GateError("analysis identity mismatch")
            if status == "completed":
                file = request(key, "/files/" + sha)
                break
            if status not in ("queued", "in-progress"):
                raise GateError("invalid analysis status")
        else:
            raise GateError("analysis pending")
    return validate(file, sha, int(time.time()))

def exact_exe(directory):
    exe = directory / "live-match-collector.exe"
    if not 0 < exe.stat().st_size <= MAX_BYTES:
        raise GateError("EXE outside supported size")
    blob = exe.read_bytes()
    sha = hashlib.sha256(blob).hexdigest()
    if (directory / "SHA256SUMS.exe").read_text().split() != [sha, exe.name]:
        raise GateError("packaged EXE checksum mismatch")
    return blob, sha

def main(directory, verify=False):
    directory = Path(directory)
    if verify:
        _, sha = exact_exe(directory)
        report = json.loads((directory / "virustotal-report.json").read_text())
        if report["sha256"] != sha or report["exit_code"] != 0 or report["status"] != "passed":
            return 1
        # Revalidate the retained engine data rather than trusting a pass label.
        results = {e["engine"]: {"category": e["category"], "result": e["result"]} for e in report["engines"]}
        if len(results) != len(report["engines"]):
            return 1
        file = {"id": sha, "type": "file", "attributes": {"sha256": sha,
                "last_analysis_date": report["analysis_date"], "last_analysis_stats": report["stats"],
                "last_analysis_results": results}}
        return int(validate(file, sha, int(time.time()))["status"] != "passed")
    report = {"sha256": None, "status": "unverified", "exit_code": 1}
    try:
        blob, sha = exact_exe(directory)
        report["sha256"] = sha
        key = os.getenv("VIRUSTOTAL_API_KEY", "").strip()
        if not key:
            raise GateError("API key not configured")
        verified = scan(key, blob, sha, os.getenv("VT_ALLOW_UPLOAD") == "true")
        if key in json.dumps(verified, ensure_ascii=False):
            raise GateError("unexpected credential in response")
        report.update(verified)
        report["exit_code"] = int(report["status"] != "passed")
    except urllib.error.HTTPError as error:
        report["status"] = f"API HTTP {error.code}"
    except GateError as error:
        report["status"] = str(error)
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        report["status"] = "invalid or unavailable evidence"
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "virustotal-report.json").write_text(json.dumps(report, indent=2, ensure_ascii=True) + "\n")
    lines = ["## VirusTotal", "", f"SHA-256: {report['sha256']}", f"Status: {report['status']}",
             f"Exit code: {report['exit_code']}"]
    if report["sha256"]:
        lines += [f"https://www.virustotal.com/gui/file/{report['sha256']}"]
    if "stats" in report:
        lines += [f"Statistics: {json.dumps(report['stats'], sort_keys=True)}"]
    lines += ["", "Engine-level results: virustotal-report.json. Zero detections do not establish safety."]
    (directory / "virustotal-report.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    return report["exit_code"]

if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1], "--verify" in sys.argv[2:]))
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        sys.exit("VirusTotal evidence invalid or unavailable")