#!/usr/bin/env python3
"""Exercise generated host -> platform and M11 -> ownership -> diagnostics."""
import ast
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / 'tools/ladybird'
producer = ast.parse((SCRIPTS / 'prepare-full-browser-host.py').read_text())
console = ast.parse((SCRIPTS / 'prepare-console.py').read_text())
owner = ast.parse((SCRIPTS / 'prepare-m11-input-ownership.py').read_text())
network = ast.parse((SCRIPTS / 'prepare-network-live.py').read_text())


def assigned_literal(tree, name):
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(
            isinstance(target, ast.Name) and target.id == name for target in node.targets
        ):
            return ast.literal_eval(node.value)
    raise AssertionError(f'Production literal {name} not located')


# Contract check: the output produced by prepare-console.py must be accepted by
# prepare-full-browser-host.py. This runs before fetch/vcpkg/the Ladybird build,
# so an anchor drift fails in seconds instead of after the expensive setup.
console_remplacement = assigned_literal(console, 'remplacement')
console_prepare = assigned_literal(producer, 'console_prepare')
console_legacy = assigned_literal(producer, 'console_legacy')
console_new = assigned_literal(producer, 'console_new')
patched_console = console_remplacement
if console_new not in patched_console:
    for candidate in (console_prepare, console_legacy):
        if candidate in patched_console:
            patched_console = patched_console.replace(candidate, console_new, 1)
            break
assert console_new in patched_console, 'prepare-console -> BrowserHost console contract drifted'

with tempfile.TemporaryDirectory() as temporary:
    root = Path(temporary)
    host = root / 'Services/BouchaudBrowserHost/main.cpp'
    connection = root / 'Services/WebContent/ConnectionFromClient.cpp'
    host.parent.mkdir(parents=True)
    connection.parent.mkdir(parents=True)
    # Use production literals, not copies that can drift from their producer.
    for node in producer.body:
        if isinstance(node, ast.Expr) and isinstance(node.value, ast.Call):
            call = node.value
            if isinstance(call.func, ast.Attribute) and call.func.attr == 'write_text':
                if 'main.cpp' in ast.unparse(call.func.value):
                    host.write_text(ast.literal_eval(call.args[0]))
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == 'new_init' for t in node.targets):
            connection.write_text(ast.literal_eval(node.value))
    assert host.exists() and connection.exists(), 'Production generators not located'
    scope = {'root': root}
    helper = next(n for n in owner.body if isinstance(n, ast.FunctionDef) and n.name == 'patch')
    exec(compile(ast.Module(body=[helper], type_ignores=[]), '<production helper>', 'exec'), scope)
    calls = [n for n in owner.body if isinstance(n, ast.Expr) and isinstance(n.value, ast.Call)
             and isinstance(n.value.func, ast.Name) and n.value.func.id == 'patch'
             and any(isinstance(a, ast.Constant) and a.value == 'propriete M11' for a in n.value.args)]
    assert len(calls) == 1
    code = compile(ast.Module(body=calls, type_ignores=[]), '<production ownership>', 'exec')
    exec(code, scope)
    once = connection.read_text()
    exec(code, scope)
    assert connection.read_text() == once, 'Ownership is not idempotent'
    for _ in range(2):
        subprocess.run([sys.executable, str(SCRIPTS / 'prepare-platform-complete.py'), str(root)], check=True, capture_output=True)
    router = next(n for n in network.body if isinstance(n, ast.FunctionDef) and n.name == 'route_diagnostics')
    exec(compile(ast.Module(body=[router], type_ignores=[]), '<production routing>', 'exec'), scope)
    scope['route_diagnostics'](root)
    assert 'warnln("[ladybird-bouchaud] BROWSER_HOST_M11_ATTACHED' in connection.read_text()
    assert 'warnln("[ladybird-bouchaud] BROWSER_HOST_PLATFORM' in host.read_text()
print('Ladybird generated host / platform / M11 ownership / diagnostics: OK')
