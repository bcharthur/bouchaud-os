import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "whpx_bench", Path(__file__).with_name("banc-whpx-browserhost.py")
)
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)


class DiskTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def image(self, name, *, payload=b"browser", config=b"root ca", system=b"wasm",
              mode=0o644, uid=0, gid=0, mtime=0, link="config", nonzero=False):
        path = self.root / name
        with tarfile.open(path, "w", format=tarfile.USTAR_FORMAT) as archive:
            for filename, contents in (
                ("bo-navigateur", payload), ("etc/ssl/certs/ca-certificates.crt", config),
                ("usr/lib/python/rustpython.wasm", system),
            ):
                entry = tarfile.TarInfo("./" + filename)
                entry.size = len(contents)
                entry.mtime = mtime
                entry.mode, entry.uid, entry.gid = mode, uid, gid
                archive.addfile(entry, io.BytesIO(contents))
            entry = tarfile.TarInfo("./etc/ssl/current")
            entry.type, entry.linkname = tarfile.SYMTYPE, link
            archive.addfile(entry)
        with path.open("r+b") as image:
            image.truncate(path.stat().st_size + 128 * BENCH.MIB)
            if nonzero:
                image.seek(-1, 2)
                image.write(b"x")
        return path

    def test_disk_contract(self):
        reference = BENCH.inventory(self.image("reference"))
        cases = (
            ("identical", {}, True),
            ("timestamp", {"mtime": 158}, True),
            ("ladybird", {"payload": b"new browser"}, True),
            ("system", {"system": b"modified wasm"}, False),
            ("configuration", {"config": b"other ca"}, False),
            ("metadata", {"mode": 0o755, "uid": 42, "gid": 11, "link": "wrong"}, False),
            ("persistence", {"nonzero": True}, False),
        )
        for name, variants, expected in cases:
            with self.subTest(name=name):
                state = BENCH.inventory(self.image(name, **variants))
                report = BENCH.diff_disks(reference, state)
                self.assertEqual(report["accepted"], expected)
                if name == "metadata":
                    self.assertEqual(set(report["changed"]["usr/lib/python/rustpython.wasm"]),
                                     {"mode", "uid", "gid"})
                    self.assertIn("linkname", report["changed"]["etc/ssl/current"])
                if name == "configuration":
                    self.assertIn("sha256", report["changed"]["etc/ssl/certs/ca-certificates.crt"])
                if name == "persistence":
                    self.assertFalse(report["persistence"]["current"]["zero"])

    def test_lists_exclusive_paths_and_type_changes(self):
        reference = BENCH.inventory(self.image("reference"))
        candidate = BENCH.inventory(self.image("candidate"))
        entries = candidate["entries"]
        entries["etc/new-config"] = entries.pop("etc/ssl/certs/ca-certificates.crt")
        entries["etc/ssl/current"]["type"] = tarfile.LNKTYPE.hex()
        report = BENCH.diff_disks(reference, candidate)
        self.assertFalse(report["accepted"])
        self.assertEqual(set(report["only_reference"]), {"etc/ssl/certs/ca-certificates.crt"})
        self.assertEqual(set(report["only_current"]), {"etc/new-config"})
        self.assertIn("type", report["changed"]["etc/ssl/current"])
        self.assertFalse(BENCH.is_browser_payload("usr/share/ladybird/fontconfig/fonts.conf"))

    def test_rejected_disk_has_inventory_diff_and_archive(self):
        trial = self.root / "trial"
        trial.mkdir()
        logs = self.root / "logs"
        logs.mkdir()
        disk = self.image("rejected", config=b"modified")
        invariants = {"reference_disk": BENCH.inventory(self.image("reference"))}
        with self.assertRaisesRegex(ValueError, "ca-certificates.crt"):
            BENCH.check_disk(disk, trial, invariants)
        (logs / "previous-serie.log").touch()
        (logs / "interrupted-serie.log").write_text("partial serial", encoding="utf-8")
        (logs / "interrupted-qemu.log").write_text("partial qemu", encoding="utf-8")
        BENCH.archive_trial(logs, {logs / "previous-serie.log"}, trial, disk, passed=False)
        self.assertEqual({path.name for path in trial.iterdir()}, {
            "disk-diff.json", "disk-inventory.json", "disk-interrompu.img",
            "interrupted-qemu.log", "serial.log",
        })
        self.assertEqual((trial / "serial.log").read_text(encoding="utf-8"), "partial serial")
        self.assertFalse(disk.exists())
