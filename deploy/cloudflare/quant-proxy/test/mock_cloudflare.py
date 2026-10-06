"""A tiny stand-in for the parts of the Cloudflare API that deploy.sh uses, plus the public
hone-quant endpoints it verifies. State lives in memory; GET /_state returns it as JSON and
POST /_route adds a route directly (to simulate a conflicting route). Secret values are never
stored: only whether they matched MOCK_EXPECTED_SECRET.

    MOCK_API_TOKEN=t MOCK_EXPECTED_SECRET=s python3 mock_cloudflare.py PORT
"""

import json
import os
import re
import sys
from email.parser import BytesParser
from email.policy import default as default_policy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

API_TOKEN = os.environ["MOCK_API_TOKEN"]
EXPECTED_SECRET = os.environ["MOCK_EXPECTED_SECRET"]
STATE = {"routes": [], "scripts": {}, "next_id": 1, "calls": []}


def ok(result=None):
    return 200, {"success": True, "errors": [], "messages": [], "result": result}


def err(status, code, message):
    return status, {"success": False, "errors": [{"code": code, "message": message}], "result": None}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, status, payload, headers=None):
        body = json.dumps(payload).encode() if payload is not None else b""
        self.send_response(status)
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        if payload is not None:
            self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def body(self):
        length = int(self.headers.get("content-length") or 0)
        return self.rfile.read(length) if length else b""

    def handle_any(self):
        path = self.path.split("?", 1)[0]
        # Public endpoints checked by deploy.sh's verification step.
        if path == "/quant/api/health":
            return self.reply(200, {"ok": True, "db": True})
        if path == "/quant/api/auth/me":
            return self.reply(401, {"error": "unauthorized"})
        if path == "/quant":
            return self.reply(308, None, {"location": "/quant/"})
        if path == "/_state":
            return self.reply(200, STATE)
        if path == "/_route" and self.command == "POST":
            route = json.loads(self.body())
            route["id"] = f"r{STATE['next_id']}"
            STATE["next_id"] += 1
            STATE["routes"].append(route)
            return self.reply(200, {"ok": True})

        if self.headers.get("authorization") != f"Bearer {API_TOKEN}":
            return self.reply(*err(403, 9109, "Invalid access token"))
        STATE["calls"].append(f"{self.command} {path}")

        m = re.fullmatch(r"/client/v4/user/tokens/verify", path)
        if m and self.command == "GET":
            return self.reply(*ok({"id": "tok", "status": "active"}))

        m = re.fullmatch(r"/client/v4/zones/([0-9a-f]+)/workers/routes(?:/([\w-]+))?", path)
        if m:
            route_id = m.group(2)
            if self.command == "GET" and not route_id:
                return self.reply(*ok(STATE["routes"]))
            if self.command == "POST" and not route_id:
                route = json.loads(self.body())
                if any(r["pattern"] == route["pattern"] for r in STATE["routes"]):
                    return self.reply(*err(409, 10020, "duplicate route"))
                route["id"] = f"r{STATE['next_id']}"
                STATE["next_id"] += 1
                STATE["routes"].append(route)
                return self.reply(*ok({"id": route["id"]}))
            if self.command == "DELETE" and route_id:
                before = len(STATE["routes"])
                STATE["routes"] = [r for r in STATE["routes"] if r["id"] != route_id]
                if len(STATE["routes"]) == before:
                    return self.reply(*err(404, 10019, "route not found"))
                return self.reply(*ok({"id": route_id}))

        m = re.fullmatch(r"/client/v4/accounts/([0-9a-f]+)/workers/scripts/([\w-]+)(/secrets|/subdomain)?", path)
        if m:
            name, sub = m.group(2), m.group(3)
            script = STATE["scripts"].get(name)
            if sub is None and self.command == "PUT":
                content_type = self.headers.get("content-type", "")
                message = BytesParser(policy=default_policy).parsebytes(
                    f"Content-Type: {content_type}\r\n\r\n".encode() + self.body()
                )
                parts = {p.get_param("name", header="content-disposition"): p for p in message.iter_parts()}
                metadata = json.loads(parts["metadata"].get_content())
                module = parts[metadata["main_module"]]
                kept = script["secrets"] if script and "secret_text" in metadata.get("keep_bindings", []) else {}
                STATE["scripts"][name] = {
                    "metadata": metadata,
                    "module_type": module.get_content_type(),
                    "module_bytes": len(module.get_payload(decode=True)),
                    "secrets": kept,
                    "subdomain": script["subdomain"] if script else {"enabled": True, "previews_enabled": True},
                }
                return self.reply(*ok({"id": name}))
            if script is None:
                return self.reply(*err(404, 10007, "workers.api.error.script_not_found"))
            if sub is None and self.command == "DELETE":
                del STATE["scripts"][name]
                return self.reply(*ok(None))
            if sub == "/secrets" and self.command == "PUT":
                secret = json.loads(self.body())
                script["secrets"][secret["name"]] = {
                    "type": secret["type"],
                    "matched_expected": secret["text"] == EXPECTED_SECRET,
                }
                return self.reply(*ok({"name": secret["name"], "type": secret["type"]}))
            if sub == "/secrets" and self.command == "GET":
                return self.reply(*ok([{"name": k, "type": v["type"]} for k, v in script["secrets"].items()]))
            if sub == "/subdomain" and self.command == "POST":
                script["subdomain"] = json.loads(self.body())
                return self.reply(*ok(script["subdomain"]))

        return self.reply(*err(404, 7003, f"no mock for {self.command} {path}"))

    do_GET = do_POST = do_PUT = do_DELETE = handle_any


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
