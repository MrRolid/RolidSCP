#!/usr/bin/env python3
# A static file server for packaging/ci/appimage-update.sh, with HTTP range requests (single and multiple ranges).
import http.server, os, sys
ROOT, PORT = sys.argv[1], int(sys.argv[2])
class H(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def path_(self):
        p = os.path.join(ROOT, os.path.basename(self.path.split("?")[0]))
        return p if os.path.isfile(p) else None
    def do_HEAD(self): self.serve(False)
    def do_GET(self): self.serve(True)
    def serve(self, body):
        p = self.path_()
        if not p:
            self.send_response(404); self.send_header("Content-Length", "0"); self.end_headers(); return
        size = os.path.getsize(p)
        rng = self.headers.get("Range")
        data = open(p, "rb").read()
        if not rng or not rng.startswith("bytes="):
            self.send_response(200); self.send_header("Accept-Ranges", "bytes")
            self.send_header("Content-Length", str(size)); self.end_headers()
            if body: self.wfile.write(data)
            return
        parts = []
        for r in rng[6:].split(","):
            first, last = r.strip().split("-")
            if first:
                a, b = int(first), int(last) if last else size - 1
            else:  # the last N bytes
                a, b = size - int(last), size - 1
            parts.append((max(a, 0), min(b, size - 1)))
        if len(parts) == 1:
            a, b = parts[0]
            self.send_response(206); self.send_header("Content-Range", f"bytes {a}-{b}/{size}")
            self.send_header("Content-Length", str(b - a + 1)); self.end_headers()
            if body: self.wfile.write(data[a:b + 1])
            return
        out = b""
        for a, b in parts:
            out += (f"--SEP\r\nContent-Type: application/octet-stream\r\nContent-Range: bytes {a}-{b}/{size}\r\n\r\n").encode() + data[a:b + 1] + b"\r\n"
        out += b"--SEP--\r\n"
        self.send_response(206); self.send_header("Content-Type", "multipart/byteranges; boundary=SEP")
        self.send_header("Content-Length", str(len(out))); self.end_headers()
        if body: self.wfile.write(out)
    def log_message(self, f, *a): sys.stderr.write((f % a) + "\n")
http.server.ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
