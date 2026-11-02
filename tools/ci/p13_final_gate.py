#!/usr/bin/env python3
"""Strict P13 same-SHA gate: real 6/6 C22 and independent Nightly product."""
import json
import os
import subprocess

sha = os.environ['GITHUB_SHA']
branch = os.environ['GITHUB_REF_NAME']
repo = os.environ['GITHUB_REPOSITORY']

def api(path):
    return json.loads(subprocess.check_output(['gh', 'api', path], text=True))

runs = api(f'repos/{repo}/actions/runs?head_sha={sha}&per_page=100')['workflow_runs']
checks = {
    'ladybird-native-browser': (
        'ladybird / C22 memoire 6x6 candidat (KVM strict)',
        'ladybird / verdict de convergence',
        "ladybird / provenance de l'artefact",
    ),
    'Nightly Full': (
        'nightly-gate',
        'nightly / Ladybird product',
        'nightly / kernel once',
    ),
}
for workflow, mandatory_jobs in checks.items():
    acceptable = [r for r in runs if r.get('name') == workflow
                  and r.get('head_sha') == sha
                  and r.get('head_branch') == branch
                  and r.get('event') == 'workflow_dispatch'
                  and r.get('status') == 'completed'
                  and r.get('conclusion') == 'success']
    qualified = False
    for r in acceptable:
        jobs = api(f"repos/{repo}/actions/runs/{r['id']}/jobs?per_page=100")['jobs']
        valid = all(sum(1 for j in jobs if j.get('name') == job
                        and j.get('status') == 'completed'
                        and j.get('conclusion') == 'success') == 1
                    for job in mandatory_jobs)
        if workflow == 'ladybird-native-browser':
            primitives = [j for j in jobs if j.get('name', '').startswith('primitives / ')]
            valid = valid and len(primitives) >= 4 and all(
                j.get('status') == 'completed' and j.get('conclusion') == 'success'
                for j in primitives)
        if valid:
            print(f'P13_EXTERNAL_GATE_OK workflow={workflow} run={r["id"]} sha={sha}')
            qualified = True
            break
    if not qualified:
        raise SystemExit(f'P13_NON_QUALIFIE: {workflow} no complete strict evidence on {sha}')
print(f'P13_FINAL_STRICT_OK sha={sha}')
