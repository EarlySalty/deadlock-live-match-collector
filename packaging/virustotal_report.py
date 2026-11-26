"""Submit only the exact packaged EXE to VirusTotal; no runtime client integration."""
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

BASE = "https://www.virustotal.com/api/v3"
MAX_BYTES = 32_000_000

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None

def request(key, path, data=None, content_type=None):
    headers = {"x-apikey": key, "Accept": "application/json"}
    if content_type:
        headers["Content-Type"] = content_type
    req = urllib.request.Request(BASE + path, data=data, headers=headers)
    with urllib.request.build_opener(NoRedirect()).open(req, timeout=40) as response:
        return json.load(response)["data"]

def scan(key, blob, sha):
    try:
        file = request(key, "/files/" + sha)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
        boundary = uuid.uuid4().hex
        body = (f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="live-match-collector.exe"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode()
                + blob + f'\r\n--{boundary}--\r\n'.encode())
        analysis = request(key, "/files", body, "multipart/form-data; boundary=" + boundary)
        analysis_id = analysis["id"]
        if not re.fullmatch(r"[A-Za-z0-9_=+/.-]+", analysis_id) or "/" in analysis_id:
            raise ValueError("Invalid analysis ID")
        for _ in range(8):
            time.sleep(20)
            result = request(key, "/analyses/" + analysis_id)
            if result["attributes"]["status"] == "completed":
                file = request(key, "/files/" + sha)
                break
        else:
            return "submitted; analysis pending", None
    if file["id"] != sha:
        raise ValueError("Report hash mismatch")
    attrs = file["attributes"]
    stats = attrs.get("last_analysis_stats")
    if not isinstance(stats, dict) or any(type(stats.get(k)) is not int or stats[k] < 0 for k in ("malicious", "suspicious", "undetected", "harmless")):
        return "report found; engine results not yet available", None
    return "report retrieved", stats

def main():
    directory = Path(sys.argv[1])
    exe = directory / "live-match-collector.exe"
    if not 0 < exe.stat().st_size <= MAX_BYTES:
        raise ValueError("EXE outside supported size")
    blob = exe.read_bytes()
    sha = hashlib.sha256(blob).hexdigest()
    expected = (directory / "SHA256SUMS.exe").read_text().split()
    if expected != [sha, exe.name]:
        raise ValueError("Packaged EXE checksum mismatch")
    key = os.getenv("VIRUSTOTAL_API_KEY", "").strip()
    status, stats = "not submitted: API key not configured", None
    if key:
        try:
            status, stats = scan(key, blob, sha)
        except urllib.error.HTTPError as error:
            status = f"API unavailable (HTTP {error.code}); report not verified"
        except (OSError, ValueError, KeyError, TypeError):
            status = "API response unavailable or invalid; report not verified"
    url = f"https://www.virustotal.com/gui/file/{sha}"
    lines = ["## VirusTotal", "", f"[Report for this exact EXE]({url})", "", f"SHA-256: `{sha}`", "", f"Status: {status}."]
    if stats is not None:
        lines += ["", f"Malicious: {stats['malicious']}; suspicious: {stats['suspicious']}; undetected: {stats['undetected']}; harmless: {stats['harmless']}."]
    lines += ["", "A report link is not proof of a completed scan. Results may change; zero detections do not establish safety.", ""]
    report = "\n".join(lines)
    (directory / "virustotal-report.md").write_text(report, encoding="utf-8")
    print(report)
    if stats is not None and (stats["malicious"] or stats["suspicious"]):
        sys.exit("VirusTotal detections require manual investigation before publication.")

if __name__ == "__main__":
    main()
