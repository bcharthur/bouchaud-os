#!/usr/bin/env python3
"""Garde statique du correctif page_id <-> WebContentClient M11."""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
runner = root / "tools/ladybird/browser-upstream.sh"
prep = root / "tools/ladybird/prepare-m11-page-registry.py"

errors = []
if not prep.exists():
    errors.append("prepare-m11-page-registry.py absent")
else:
    data = prep.read_text()
    for required in (
        "M11_TAB_HOST_REGISTERED",
        "DidRequestNewWebView",
        'getenv("BOUCHAUD_BROWSER_HOST") != nullptr',
        "M11_TAB_POPUP_HOST_REGISTERED",
        "WebContentClient::m_views",
    ):
        if required not in data:
            errors.append(f"marqueur absent du prepare script: {required}")

if not runner.exists():
    errors.append("browser-upstream.sh absent")
else:
    data = runner.read_text()
    hook = 'python3 tools/ladybird/prepare-m11-page-registry.py "$SRC"'
    if hook not in data:
        errors.append("browser-upstream.sh n'appelle pas prepare-m11-page-registry.py")
    else:
        pos_v19 = data.find('python3 tools/ladybird/prepare-v19-navigateur.py "$SRC"')
        pos_hook = data.find(hook)
        pos_runtime = data.find('python3 tools/ladybird/prepare-browser-runtime-link.py "$SRC"')
        if pos_v19 < 0 or not (pos_v19 < pos_hook):
            errors.append("le correctif doit s'appliquer APRES prepare-v19-navigateur.py")
        if pos_runtime >= 0 and not (pos_hook < pos_runtime):
            errors.append("le correctif doit s'appliquer AVANT prepare-browser-runtime-link.py")

if errors:
    for error in errors:
        print("ERREUR:", error)
    sys.exit(1)

print("BOUCHAUD_PAGE_LIFECYCLE_GUARD_OK")
