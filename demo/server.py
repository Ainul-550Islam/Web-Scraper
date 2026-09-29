#!/usr/bin/env python3
"""Tiny mock data source for harvest demos/tests.

Serves a toy product catalog + news feed with a real robots.txt
(/private/ disallowed). Local only — never point harvest configs at
arbitrary public sites without permission.

Usage: python3 demo/server.py [port]   (default 8099)
"""
import http.server
import hashlib
import json
import sys

PRODUCTS = {f"/product/{i}": (n, p) for i, (n, p) in enumerate(
    [("Widget One", "$19.99"), ("Widget Two", "$24.99"), ("Widget Three", "$29.99")], start=1)}

INDEX = """<html><body><h1>Catalog Home</h1><p class="price">$0.00</p>
""" + "".join(
    f'<a class="product" href="{p}">{n}</a><ul><li class="tag">toy</li><li class="tag">demo</li></ul>'
    for p, (n, _) in PRODUCTS.items()
) + '<a href="/private/secret">admin</a></body></html>'

PAGES = {
    "/index.html": INDEX,
    "/news.html": '<html><body><h2>Harvest demo ships</h2><time datetime="2026-09-11">Sep 11</time></body></html>',
    "/robots.txt": "User-agent: *\nDisallow: /private/\nCrawl-delay: 1\n",
}

# Sitemap-driven seeding + JSON API demo targets (Phase 5).
SITEMAP = """<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>/index.html</loc></url>
  <url><loc>http://REPLACE/product/1</loc></url>
  <url><loc>http://REPLACE/product/2</loc></url>
  <url><loc>http://REPLACE/product/3</loc></url>
  <url><loc>http://REPLACE/private/secret</loc></url>
</urlset>"""

CATALOG = json.dumps({"items": [
    {"id": i, "name": name, "price_cents": int(price[1:].replace(".", "")), "tags": ["widget"], "in_stock": i != 2}
    for i, (name, price) in PRODUCTS.items()
]})
for path, (name, price) in PRODUCTS.items():
    PAGES[path] = (f'<html><body><h1>{name}</h1><p class="price">{price}</p>'
                   f'<ul><li class="tag">widget</li></ul><a class="product" href="/index.html">back</a></body></html>')


class H(http.server.BaseHTTPRequestHandler):
    def _serve(self, data: bytes, ctype: str):
        tag = '"' + hashlib.sha1(data).hexdigest()[:16] + '"'
        if self.headers.get("If-None-Match") == tag:
            self.send_response(304)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("content-type", ctype)
        self.send_header("etag", tag)
        self.send_header("cache-control", "no-cache")  # forces revalidation every run
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):  # noqa: N802
        if self.path == "/sitemap.xml":
            self._serve(SITEMAP.replace("REPLACE", self.headers.get("Host", f"127.0.0.1:{PORT}")).encode(), "application/xml")
            return
        if self.path == "/api/catalog.json":
            self._serve(CATALOG.encode(), "application/json")
            return
        body = PAGES.get(self.path)
        if self.path.startswith("/private/"):
            self.send_response(403)
            self.end_headers()
            self.wfile.write(b"nope")
            return
        if body is None:
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"not found")
            return
        data = body.encode()
        ctype = "text/plain" if self.path == "/robots.txt" else "text/html; charset=utf-8"
        self._serve(data, ctype)

    def log_message(self, fmt, *args):
        print("[mock]", fmt % args, flush=True)


PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8099

if __name__ == "__main__":
    http.server.ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
