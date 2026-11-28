"""Conservative Fabric dependency plan. Historical triggers are a floor."""

import fnmatch
import json
from pathlib import Path


CONSUMERS = {
    "cpu": {"p13", "memory", "ipc", "storage", "gpu", "network", "security"},
    "memory": {"p13", "cpu", "gpu", "ipc", "storage", "security"},
    "ipc": {"security", "integration", "ladybird"},
    "gpu": {"integration", "atelier", "ladybird"},
    "network": {"integration", "ladybird", "update", "security"},
    "storage": {"integration", "update", "userland"},
    "security": {"integration", "update"},
    "update": {"reliability"},
    "integration": {"reliability"},
    "userland": {"integration"},
    "atelier": {"integration"},
    "ladybird": {"integration"},
    "reliability": set(),
    "windows": set(),
    "p13": {"memory"},
}

OWNERS = {
    "cpu": ("src/kernel/scheduler/", "src/kernel/process/", "src/arch/", "src/kernel/timer/"),
    "memory": ("src/kernel/memory/",),
    "ipc": ("src/kernel/native/", "src/compat/linux/"),
    "gpu": ("src/drivers/display/", "src/gui/", "tools/doom/"),
    "network": ("src/net/", "src/drivers/network/"),
    "storage": ("src/fs/", "src/drivers/block/", "src/drivers/usb/", "tools/fs/"),
    "security": ("src/kernel/security/", "tools/security/", "tools/securite/"),
    "update": ("system/update-core/", "tools/update/", "tools/release/"),
    "userland": ("tools/userland/", "userland/"),
    "ladybird": ("tools/ladybird/",),
    "windows": ("tools/windows/", "run.ps1"),
}

MIGRATED = frozenset(("gpu", "security", "storage", "p13", "memory", "ipc"))
COMPAT = json.loads((Path(__file__).parent / "fabric_compat_paths.json").read_text())
CONFIG = (".github/", "tools/ci/", "targets/", ".cargo/")


def documentation(path):
    return path.startswith(("docs/", ".github/ISSUE_TEMPLATE/")) or path in {"README.md", "STATUS.md"}


def plan(paths, exhaustive=False):
    selected = set()
    uncategorized = []
    for path in paths:
        if documentation(path):
            continue
        historical = {domain for domain, patterns in COMPAT.items()
                      if any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)}
        selected.update(historical)
        owners = {domain for domain, prefixes in OWNERS.items()
                  if any(path.startswith(prefix) if prefix.endswith("/") else path == prefix
                         for prefix in prefixes)}
        if not owners and not historical:
            uncategorized.append(path)
        selected.update(owners)
    if exhaustive or not paths or uncategorized or any(p.startswith(CONFIG) and not documentation(p) for p in paths):
        selected.update(CONSUMERS)
    frontier = set(selected)
    while frontier:
        next_domains = set().union(*(CONSUMERS[domain] for domain in frontier)) - selected
        selected.update(next_domains)
        frontier = next_domains
    return {"domains": sorted(selected), "uncategorized": sorted(uncategorized)}
