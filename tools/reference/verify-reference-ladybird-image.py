#!/usr/bin/env python3
from pathlib import Path
import sys
import tarfile

PERSIST_BYTES = 128 * 1024 * 1024

REQUIRED_FILES = {
    "bo-navigateur",
    "usr/libexec/ladybird/BouchaudBrowserHost",
    "usr/libexec/ladybird/WebContent",
    "usr/libexec/ladybird/RequestServer",
    "usr/libexec/ladybird/ImageDecoder",
    "usr/libexec/ladybird/Compositor",
    "usr/libexec/ladybird/WebWorker",
    "usr/libexec/ladybird/WebDriver",
    "usr/libexec/ladybird/webcontent-bootstrap",
    "etc/ssl/certs/ca-certificates.crt",
    "usr/share/ladybird/fontconfig/fonts.conf",
}

EXECUTABLES = {
    "bo-navigateur",
    "usr/libexec/ladybird/BouchaudBrowserHost",
    "usr/libexec/ladybird/WebContent",
    "usr/libexec/ladybird/RequestServer",
    "usr/libexec/ladybird/ImageDecoder",
    "usr/libexec/ladybird/Compositor",
    "usr/libexec/ladybird/WebWorker",
    "usr/libexec/ladybird/WebDriver",
    "usr/libexec/ladybird/webcontent-bootstrap",
}

RAMONLY_MARKERS = (
    b"BROWSER_HOST_INITIALIZED",
    b"/tmp/ladybird-profile",
    b"--disable-http-disk-cache",
    b"--disable-sql-database",
)

def normalize(name: str) -> str:
    while name.startswith("./"):
        name = name[2:]
    return name.rstrip("/")

def main() -> int:
    if len(sys.argv) != 2:
        print("usage: verify-reference-ladybird-image.py <image>", file=sys.stderr)
        return 2

    image = Path(sys.argv[1]).resolve()
    if not image.is_file():
        raise SystemExit(f"image absente: {image}")

    if image.stat().st_size <= PERSIST_BYTES:
        raise SystemExit("image trop petite pour contenir archive + persistance")

    members = {}
    marker_missing = set(RAMONLY_MARKERS)
    marker_tail = b""
    marker_max = max(map(len, RAMONLY_MARKERS))

    with tarfile.open(image, mode="r:*") as archive:
        for member in archive:
            name = normalize(member.name)
            members[name] = member

            if name == "usr/libexec/ladybird/BouchaudBrowserHost":
                stream = archive.extractfile(member)
                if stream is not None:
                    while marker_missing:
                        chunk = stream.read(1024 * 1024)
                        if not chunk:
                            break
                        blob = marker_tail + chunk
                        for marker in tuple(marker_missing):
                            if marker in blob:
                                marker_missing.remove(marker)
                        marker_tail = blob[-(marker_max - 1):]

    missing = sorted(REQUIRED_FILES - set(members))
    if missing:
        raise SystemExit("image Ladybird incomplete: " + ", ".join(missing))

    if marker_missing:
        names = ", ".join(
            marker.decode("ascii", errors="replace")
            for marker in sorted(marker_missing)
        )
        raise SystemExit(
            "image Ladybird perimee: BrowserHost non RAM-only; marqueurs absents: "
            + names
        )

    for name in EXECUTABLES:
        member = members[name]
        if member.mode & 0o111 == 0:
            raise SystemExit(f"bit executable absent: {name}")

    archive_size = image.stat().st_size - PERSIST_BYTES
    if archive_size > 2048 * 1024 * 1024:
        raise SystemExit("archive Ladybird depasse 2048 Mio")

    print(f"LADYBIRD_IMAGE={image}")
    print(f"LADYBIRD_ARCHIVE_APPROX_BYTES={archive_size}")
    print("BOUCHAUD_REFERENCE_LADYBIRD_IMAGE_OK")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
