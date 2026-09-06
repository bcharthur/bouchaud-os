#!/usr/bin/env python3
from pathlib import Path
import sys

MARKERS = (
    b"BROWSER_HOST_INITIALIZED",
    b"/tmp/ladybird-profile",
    b"--disable-http-disk-cache",
    b"--disable-sql-database",
)

def scan(path: Path) -> tuple[bool, list[bytes]]:
    missing = set(MARKERS)
    max_len = max(map(len, MARKERS))
    tail = b""
    with path.open("rb") as stream:
        while True:
            chunk = stream.read(1024 * 1024)
            if not chunk:
                break
            data = tail + chunk
            for marker in tuple(missing):
                if marker in data:
                    missing.remove(marker)
            if not missing:
                return True, []
            tail = data[-(max_len - 1):] if max_len > 1 else b""
    return False, sorted(missing)

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: verify-ladybird-ramonly-artifact.py <BouchaudBrowserHost>",
              file=sys.stderr)
        return 2
    path = Path(sys.argv[1]).resolve()
    if not path.is_file():
        print(f"artefact absent: {path}", file=sys.stderr)
        return 1
    ok, missing = scan(path)
    if not ok:
        print("BOUCHAUD_LADYBIRD_RAMONLY_ARTIFACT_STALE", file=sys.stderr)
        for marker in missing:
            print("  marqueur absent: " + marker.decode("ascii", errors="replace"),
                  file=sys.stderr)
        return 1
    print(f"LADYBIRD_BROWSER_HOST={path}")
    print(f"LADYBIRD_BROWSER_HOST_BYTES={path.stat().st_size}")
    print("BOUCHAUD_LADYBIRD_RAMONLY_ARTIFACT_OK")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
