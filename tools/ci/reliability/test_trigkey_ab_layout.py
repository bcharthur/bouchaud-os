"""Image A/B : on ne valide jamais des services absents ou un faux commit."""
import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

path = Path(__file__).resolve().parents[2] / "release" / "verify_trigkey_ab_image.py"
spec = importlib.util.spec_from_file_location("trigkey_verifier", path)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

class TrigkeyAbLayoutTests(unittest.TestCase):
    def make_tar(self, file: Path, members: tuple[str, ...]):
        with tarfile.open(file, "w", format=tarfile.USTAR_FORMAT) as archive:
            for name in members:
                payload = b"ELFTEST"
                info = tarfile.TarInfo("bin/" + name)
                info.size = len(payload)
                info.mode = 0o755
                archive.addfile(info, io.BytesIO(payload))

    def test_quatre_services_exiges(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "ramdisk"
            self.make_tar(path, check.SERVICES)
            self.assertEqual(set(check.check_tar(path)), set(check.SERVICES))
            self.make_tar(path, check.SERVICES[:-1])
            with self.assertRaises(ValueError):
                check.check_tar(path)

    def test_image_absente_refusee(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ValueError):
                check.verify(Path(d), "a" * 40)

    def test_sha_invalide_refuse(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(ValueError):
                check.verify(Path(d), "main")

if __name__ == "__main__":
    unittest.main()
