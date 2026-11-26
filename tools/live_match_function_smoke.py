#!/usr/bin/env python3
"""Loopback-only synthetic HTTP smoke check. Not a production or real-game test."""
import argparse
import getpass
import http.client
import json
import re
import sys
import time
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from threading import Thread
from urllib.parse import urlencode

UPLOAD = "/twitch/api/v3/live-match"
DETAIL = "/twitch/api/v3/streamer/live-match-details"
PUBLIC = "/twitch/api/v2/public/live-match"
CAP = 32768
SUMMARY = ("hero", "game_seconds", "kills", "deaths", "assists", "team_souls",
           "enemy_souls", "game_mode", "paused", "midboss")
GROUPS = ("scoreboard", "teams", "own_loadout", "structures", "objectives")


class Failure(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Failure(message)


def validate(body):
    require(isinstance(body, dict) and body.get("schema_version") == 3, "Expected schema 3")
    require(all(k in body and body[k] is not None for k in SUMMARY), "Use a populated synthetic summary")
    details = body.get("details")
    require(isinstance(details, dict) and details.get("schema_version") == 1, "Expected details schema 1")
    require(all(details.get(k) for k in GROUPS), "All five detail groups must be populated")
    require(all(k in body for k in ("writer_id", "sequence", "match_epoch")), "Missing writer/order fields")
    encoded = json.dumps(body, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
    require(len(encoded) <= CAP, "Payload exceeds 32768 bytes")
    return encoded


def request(port, method, path, body=None, headers=None):
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    try:
        conn.request(method, path, body=body, headers=headers or {})
        res = conn.getresponse()
        data = res.read(CAP * 4 + 1)
        require(len(data) <= CAP * 4, "Response too large")
        return res.status, data
    finally:
        conn.close()


def obj(data):
    try:
        result = json.loads(data)
    except (ValueError, UnicodeError):
        raise Failure("Expected JSON object") from None
    require(isinstance(result, dict), "Expected JSON object")
    return result


def check(port, body, key, cookie, channel, envelope="", delay=1.1):
    encoded = validate(body)
    require(re.fullmatch(r"dlm_[0-9a-fA-F]{64}", key) is not None, "Invalid test key shape")
    require(re.fullmatch(r"[a-z0-9_]{3,25}", channel) is not None, "Invalid test channel")
    require(bool(cookie) and "\r" not in cookie and "\n" not in cookie, "Invalid test session cookie")
    auth = {"Authorization": "Bearer " + key, "Content-Type": "application/json"}
    status, _ = request(port, "POST", UPLOAD, b"{", {"Content-Type": "application/json"})
    require(status == 401, "Unauthenticated malformed upload must be rejected before parsing")
    status, _ = request(port, "GET", DETAIL)
    require(status in (401, 403), "Anonymous detail read must be denied")
    status, _ = request(port, "POST", UPLOAD, encoded, auth)
    require(status == 204, "Full synthetic upload must return 204")
    status, data = request(port, "GET", DETAIL, headers={"Cookie": cookie})
    require(status == 200, "Authorized detail read must return 200")
    snapshot = obj(data)
    if envelope:
        snapshot = snapshot.get(envelope)
        require(isinstance(snapshot, dict), "Missing configured snapshot envelope")
    for field in SUMMARY + ("details",):
        require(field in snapshot and snapshot[field] == body[field], "Authorized output differs: " + field)
    status, data = request(port, "GET", PUBLIC + "?" + urlencode({"streamer": channel}))
    require(status == 200, "Public read must return 200")
    public = obj(data)
    require(set(public) <= {"status", "hero", "game_seconds", "paused", "midboss", "received_at"},
            "Unexpected public field")
    require(public.get("status") == "live", "Public projection not live")
    for field in ("hero", "game_seconds", "paused", "midboss"):
        require(public.get(field) == body[field], "Public projection differs: " + field)
    time.sleep(delay)
    status, _ = request(port, "POST", UPLOAD, encoded, auth)
    require(status == 409, "Accepted sequence replay must return 409")
    return len(encoded)


class SmokeTests(unittest.TestCase):
    def test_loopback_flow_and_leak_detection(self):
        payload = dict(schema_version=3, writer_id="123e4567-e89b-42d3-a456-426614174000",
                       sequence=1, match_epoch=1, hero="Synthetic Hero", game_seconds=42,
                       kills=1, deaths=2, assists=3, team_souls=100, enemy_souls=200,
                       game_mode="normal", paused=False, midboss={"present": True, "source": "game"},
                       details=dict(schema_version=1, scoreboard=[{"side": "own"}],
                                    teams=[{"side": "own"}], own_loadout={"items": [1]},
                                    structures=[{"kind": "guardian"}], objectives={"midboss": {"present": True}}))
        state = {"uploaded": False, "leak": False}

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def send(self, code, value=None):
                self.send_response(code)
                self.end_headers()
                if value is not None:
                    self.wfile.write(json.dumps(value).encode())

            def do_POST(self):
                self.rfile.read(int(self.headers.get("Content-Length", 0)))
                if not self.headers.get("Authorization"):
                    self.send(401)
                elif state["uploaded"]:
                    self.send(409)
                else:
                    state["uploaded"] = True
                    self.send(204)

            def do_GET(self):
                if self.path == DETAIL:
                    if not self.headers.get("Cookie"):
                        self.send(401)
                    else:
                        self.send(200, {"snapshot": payload})
                else:
                    value = {k: payload[k] for k in ("hero", "game_seconds", "paused", "midboss")}
                    value["status"] = "live"
                    if state["leak"]:
                        value["details"] = payload["details"]
                    self.send(200, value)

        server = HTTPServer(("127.0.0.1", 0), Handler)
        thread = Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            self.assertGreater(check(server.server_port, payload, "dlm_" + "a" * 64,
                                     "test=synthetic", "test_channel", "snapshot", 0), 0)
            state.update(uploaded=False, leak=True)
            with self.assertRaisesRegex(Failure, "Unexpected public field"):
                check(server.server_port, payload, "dlm_" + "a" * 64,
                      "test=synthetic", "test_channel", "snapshot", 0)
            with self.assertRaises(Failure):
                validate({"schema_version": 2})
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--port", type=int)
    parser.add_argument("--fixture", help="Full synthetic schema-3 upload JSON, not a real game payload")
    parser.add_argument("--channel", help="Dedicated isolated test channel")
    parser.add_argument("--envelope", default="", help="Top-level response key containing snapshot, if any")
    args = parser.parse_args()
    if args.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(SmokeTests))
        return 0 if result.wasSuccessful() else 1
    if not (args.port and args.fixture and args.channel and 1 <= args.port <= 65535):
        parser.error("Provide port, fixture and channel")
    try:
        with open(args.fixture, encoding="utf-8") as f:
            body = json.load(f)
        key = getpass.getpass("Dedicated test key: ")
        cookie = getpass.getpass("Dedicated own-channel session cookie: ")
        size = check(args.port, body, key, cookie, args.channel, args.envelope)
        print(f"PASS: synthetic HTTP upload, authorized full projection, public allowlist, replay ({size} bytes)")
        print("NOT tested: Windows client, in-process bot consumer, RAM-only storage, expiry, or real game.")
        return 0
    except Failure as exc:
        print("FAIL: " + str(exc), file=sys.stderr)
        return 1
    except (OSError, ValueError, http.client.HTTPException, TypeError):
        print("FAIL: local connection, fixture or response error", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
