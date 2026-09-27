#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
LAB = ROOT / "tools" / "remote" / "bouchaud-lab.py"

spec = importlib.util.spec_from_file_location("bouchaud_lab_verify_source", LAB)
if spec is None or spec.loader is None:
    raise SystemExit("import bouchaud-lab impossible")
lab = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = lab
spec.loader.exec_module(lab)


class FakeSock:
    def settimeout(self, _):
        pass

    def close(self):
        pass


# 1. Un --source-ip explicite doit etre passe tel quel au socket.
client = lab.ClientBrdp(
    "169.254.178.21",
    "x" * 64,
    2222,
    1.0,
    "169.254.6.185",
)
client._poignee_de_main = lambda: None
with patch.object(lab.socket, "create_connection", return_value=FakeSock()) as cc:
    client.ouvre()
    _, kwargs = cc.call_args
    assert kwargs.get("source_address") == ("169.254.6.185", 0), kwargs
client.abandonne()

# 2. La source LAB de .env/processus est automatique UNIQUEMENT pour une
# cible IPv4 link-local. Elle ne doit jamais contaminer les tests/faux serveurs
# sur 127.0.0.1 ni une autre destination non link-local.
ancien = os.environ.get("BOUCHAUD_LAB_SOURCE_IP")
os.environ["BOUCHAUD_LAB_SOURCE_IP"] = "169.254.99.9"
try:
    assert lab.resout_source_ip("169.254.178.21", None) == "169.254.99.9"
    assert lab.resout_source_ip("127.0.0.1", None) is None
    assert lab.resout_source_ip("192.168.137.1", None) is None
    # L'explicite reste prioritaire, quelle que soit la cible.
    assert lab.resout_source_ip("127.0.0.1", "127.0.0.1") == "127.0.0.1"
finally:
    if ancien is None:
        os.environ.pop("BOUCHAUD_LAB_SOURCE_IP", None)
    else:
        os.environ["BOUCHAUD_LAB_SOURCE_IP"] = ancien

print("LAB_HOST_SOURCE_VERIFY_V5_OK")
