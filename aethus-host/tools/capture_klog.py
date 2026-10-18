#!/usr/bin/env python3
"""Capture console klog to a file until the deadline or a stop marker appears."""
import argparse
from pathlib import Path
import socket
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="192.168.0.86")
    parser.add_argument("--port", type=int, default=3232)
    parser.add_argument("--seconds", type=int, default=3600)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--stop-file", type=Path, required=True)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + args.seconds
    with socket.create_connection((args.host, args.port), timeout=10) as sock, args.output.open("ab", buffering=0) as log:
        sock.settimeout(1)
        print(f"Capturing {args.host}:{args.port} to {args.output}", flush=True)
        while time.monotonic() < deadline and not args.stop_file.exists():
            try:
                data = sock.recv(65536)
            except socket.timeout:
                continue
            if not data:
                print("Console closed klog connection", flush=True)
                break
            log.write(data)
    print("Klog capture complete", flush=True)


if __name__ == "__main__":
    main()
