#!/usr/bin/env python3
"""Serve only a prepared update.json and its content-addressed blobs for a pinned LAN test."""
import argparse
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--bind", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8840)
    args = parser.parse_args()
    directory = args.directory.resolve()
    if not (directory / "update.json").is_file():
        parser.error("No prepared update.json")

    class Handler(SimpleHTTPRequestHandler):
        def __init__(self, *a, **kw):
            super().__init__(*a, directory=str(directory), **kw)

        def send_head(self):
            path = urlsplit(self.path).path
            if not re.fullmatch(r"/(?:update\.json|[0-9a-f]{64}\.bin)", path) or (directory / path[1:]).is_symlink():
                self.send_error(404)
                return None
            return super().send_head()

        def end_headers(self):
            self.send_header("Cache-Control", "no-store")
            super().end_headers()

    print(f"Serving the prepared update only at http://{args.bind}:{args.port}/update.json", flush=True)
    ThreadingHTTPServer((args.bind, args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
