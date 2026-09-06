#!/usr/bin/env python3
from pathlib import Path
import sys
import tarfile

PERSIST_BYTES = 128 * 1024 * 1024

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

def main() -> int:
    if len(sys.argv) != 3:
        print("usage: make-reference-ladybird-image.py <scenario> <image>", file=sys.stderr)
        return 2

    root = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2]).resolve()

    if not root.is_dir():
        raise SystemExit(f"scenario absent: {root}")

    if output.exists():
        output.unlink()

    with tarfile.open(output, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for path in sorted(root.rglob("*"), key=lambda p: p.as_posix()):
            relative = path.relative_to(root).as_posix()
            info = archive.gettarinfo(str(path), arcname=f"./{relative}")
            info.uid = 0
            info.gid = 0
            info.uname = "root"
            info.gname = "root"
            info.mtime = 0

            if path.is_dir():
                info.mode = 0o755
                archive.addfile(info)
                continue

            if path.is_file():
                info.mode = 0o755 if relative in EXECUTABLES else 0o644
                with path.open("rb") as stream:
                    archive.addfile(info, stream)

    size = output.stat().st_size
    padding = (-size) % 512

    with output.open("ab") as stream:
        if padding:
            stream.write(b"\0" * padding)

        remaining = PERSIST_BYTES
        zero = b"\0" * (1024 * 1024)
        while remaining:
            chunk = min(remaining, len(zero))
            stream.write(zero[:chunk])
            remaining -= chunk

    final_size = output.stat().st_size
    if final_size % 512:
        raise SystemExit("image Ladybird non alignee sur 512 octets")

    archive_size = final_size - PERSIST_BYTES
    if archive_size > 2048 * 1024 * 1024:
        raise SystemExit("archive Ladybird > plafond noyau 2048 Mio")

    print(f"LADYBIRD_IMAGE={output}")
    print(f"LADYBIRD_IMAGE_BYTES={final_size}")
    print("BOUCHAUD_REFERENCE_LADYBIRD_IMAGE_BUILT")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
