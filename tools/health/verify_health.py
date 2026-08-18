#!/usr/bin/env python3
import argparse
import json
from pathlib import Path

def section(text, start, end):
    begin = text.find(start)
    if begin < 0:
        return ""
    begin += len(start)
    finish = text.find(end, begin) if end else len(text)
    if finish < 0:
        finish = len(text)
    return text[begin:finish]

def main():
    p = argparse.ArgumentParser()
    p.add_argument("--log", required=True)
    p.add_argument("--manifest", default="tools/health/manifest.json")
    p.add_argument("--check")
    p.add_argument("--report")
    args = p.parse_args()

    text = Path(args.log).read_text(encoding="utf-8", errors="replace")
    manifest = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    ids = [args.check] if args.check else list(manifest)
    report = {}
    failed = False

    for check_id in ids:
        if check_id not in manifest:
            print(f"health: check inconnu: {check_id}")
            return 2
        spec = manifest[check_id]
        body = section(text, spec["start"], spec.get("end"))
        missing = [x for x in spec.get("requires", []) if x not in body]
        forbidden = [x for x in spec.get("forbids", []) if x.lower() in body.lower()]
        ok = bool(body) and not missing and not forbidden
        report[check_id] = {"title": spec["title"], "ok": ok, "missing": missing, "forbidden": forbidden}
        print(f"[{'PASS' if ok else 'FAIL'}] {check_id}: {spec['title']}")
        if not body:
            print(f"  section absente: {spec['start']}")
        for x in missing:
            print(f"  requis absent: {x!r}")
        for x in forbidden:
            print(f"  motif interdit: {x!r}")
        failed |= not ok

    if args.report:
        Path(args.report).write_text(json.dumps(report, indent=2, ensure_ascii=False)+"\n", encoding="utf-8")
    return 1 if failed else 0

if __name__ == "__main__":
    raise SystemExit(main())
