"""Mutation negative: pointer-only screenshots are not window proof."""
from pathlib import Path
import runpy
import struct
import tempfile
import unittest
import zlib

ROOT=Path(__file__).resolve().parents[3]
TARGET=runpy.run_path(str(ROOT/"tools/ci/verifie_capture_atelier_fenetres.py"))
DIFF=TARGET["pixels_differents"]

def ppm(p:Path,w=200,h=140,changed=None):
    data=bytearray([230]*(w*h*3))
    for x,y in changed or ():
        j=(y*w+x)*3
        data[j:j+3]=b"\x00\x00\x00"
    p.write_bytes(f"P6\n{w} {h}\n255\n".encode()+data)

class TestAtelierScreenshotProof(unittest.TestCase):
    def test_identical_frame_is_not_proof(self):
        with tempfile.TemporaryDirectory() as tmp:
            a,b=Path(tmp)/"a.ppm",Path(tmp)/"b.ppm"
            ppm(a); ppm(b)
            self.assertEqual(DIFF(a,b),0)

    def test_pointer_only_change_is_not_proof(self):
        with tempfile.TemporaryDirectory() as tmp:
            a,b=Path(tmp)/"a.ppm",Path(tmp)/"b.ppm"
            ppm(a); ppm(b,changed=[(x,y) for x in range(15) for y in range(15)])
            self.assertLess(DIFF(a,b),TARGET["MIN_PIXELS"])

    def test_real_large_window_region_is_detected(self):
        with tempfile.TemporaryDirectory() as tmp:
            a,b=Path(tmp)/"a.ppm",Path(tmp)/"b.ppm"
            ppm(a); ppm(b,changed=[(x,y) for x in range(180) for y in range(40,120)])
            self.assertGreaterEqual(DIFF(a,b),TARGET["MIN_PIXELS"])

if __name__=="__main__":
    unittest.main()
