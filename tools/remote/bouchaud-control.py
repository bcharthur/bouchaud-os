#!/usr/bin/env python3
# P0_REMOTE_CONTROL_CLIENT_V1_2
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import socket
import sys
import time

HERE = Path(__file__).resolve().parent
LAB_PATH = HERE / "bouchaud-lab.py"

spec = importlib.util.spec_from_file_location("bouchaud_lab", LAB_PATH)
if spec is None or spec.loader is None:
    raise SystemExit(f"impossible de charger {LAB_PATH}")

lab = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = lab
spec.loader.exec_module(lab)

DEFAULT_HOST = "169.254.178.21"


def get_token(args):
    value = args.token or os.environ.get("BOUCHAUD_DEBUG_TOKEN")
    if not value:
        raise SystemExit("BOUCHAUD_DEBUG_TOKEN absent.")
    return value


def confirm(args, label):
    if args.yes:
        return
    if not sys.stdin.isatty():
        raise SystemExit(f"{label}: ajoute --yes")
    answer = input(f"Confirmer {label} ? [y/N] ").strip().lower()
    if answer not in ("y", "yes", "o", "oui"):
        raise SystemExit("annule")


def open_client(args):
    return lab.ClientBrdp(args.host, get_token(args), args.port, args.timeout)


def dump(obj):
    print(json.dumps(obj, indent=2, sort_keys=True))


def do_action(args, name, value=None, destructive=False, label=None):
    if destructive:
        confirm(args, label or name)
    with open_client(args) as client:
        reply = client.commande(name, value)
    dump(reply)
    return 0


def wait_reboot_ready(args, before_t_ns: int) -> int:
    """Attend un NOUVEAU boot, puis une vraie commande BRDP authentifiee.

    La telemetrie UDP peut revenir avant que la socket TCP 2222 soit prete.
    Une simple presence UDP n'est donc pas un critere de readiness. On attend
    `status`, et on exige une horloge noyau plus petite que celle d'avant le
    reboot pour ne jamais prendre l'ancien boot pour le nouveau.
    """
    deadline = time.monotonic() + args.ready_timeout
    started = time.monotonic()
    attempts = 0
    last_error = "aucune tentative"

    while time.monotonic() < deadline:
        attempts += 1
        try:
            with open_client(args) as client:
                status = client.commande("status")
            current_t_ns = int(status.get("t_ns", 0))
            if status.get("ok") is True and 0 < current_t_ns < before_t_ns:
                dump({
                    "ready": True,
                    "host": args.host,
                    "attempts": attempts,
                    "elapsed_s": round(time.monotonic() - started, 3),
                    "before_t_ns": before_t_ns,
                    "new_t_ns": current_t_ns,
                    "status": status,
                })
                return 0
            last_error = (
                f"BRDP repond mais le nouveau boot n'est pas encore prouve "
                f"(t_ns={current_t_ns}, avant={before_t_ns})"
            )
        except (lab.ErreurLab, OSError, socket.timeout) as exc:
            # Une coupure/timeout est normale pendant UEFI + boot. Ne pas la
            # masquer : on la conserve dans le verdict final si la readiness
            # n'arrive jamais.
            last_error = f"{type(exc).__name__}: {exc}"
        time.sleep(args.ready_interval)

    dump({
        "ready": False,
        "host": args.host,
        "attempts": attempts,
        "elapsed_s": round(time.monotonic() - started, 3),
        "before_t_ns": before_t_ns,
        "last_error": last_error,
    })
    return 6


def system_cmd(args):
    name = "system-reboot" if args.action == "reboot" else "system-shutdown"
    label = "le redemarrage distant" if args.action == "reboot" else "l'extinction distante"
    confirm(args, label)

    # Pour un reboot suivi, prendre la borne AVANT d'armer power. Si status ne
    # marche deja pas, on refuse de rebooter : on ne saurait ensuite distinguer
    # l'ancien boot du nouveau.
    before_t_ns = None
    with open_client(args) as client:
        if args.action == "reboot" and args.wait_ready:
            before = client.commande("status")
            if before.get("ok") is not True or int(before.get("t_ns", 0)) <= 0:
                raise lab.ErreurLab("status pre-reboot invalide; reboot non arme")
            before_t_ns = int(before["t_ns"])
        reply = client.commande(name)

    dump(reply)
    if args.action == "reboot" and args.wait_ready:
        return wait_reboot_ready(args, before_t_ns)
    return 0


def browser_cmd(args):
    name = {
        "start": "browser-start",
        "stop": "browser-stop",
        "restart": "browser-restart",
    }[args.action]
    return do_action(args, name)


def service_cmd(args):
    if args.service != "browser":
        raise SystemExit("V1.2: seul le service 'browser' est actionnable.")
    return browser_cmd(args)


def process_cmd(args):
    name = "process-kill-tree" if args.action == "kill-tree" else "process-kill"
    return do_action(
        args,
        name,
        args.pid,
        destructive=True,
        label=f"{args.action} pid={args.pid}",
    )


def checkpoint_cmd(args):
    return do_action(args, "checkpoint")


def doctor_cmd(args):
    names = ["status", "services", "processes", "memory", "net", "rtl8168", "internet"]
    result = {"schema": 1, "host": args.host, "time": time.time(), "checks": {}}

    with open_client(args) as client:
        for name in names:
            try:
                result["checks"][name] = client.commande(name)
            except Exception as exc:
                result["checks"][name] = {
                    "ok": False,
                    "client_error": f"{type(exc).__name__}: {exc}",
                }

    dump(result)
    return 0 if all(v.get("ok", False) for v in result["checks"].values()) else 1


def capabilities_cmd(_args):
    dump({
        "schema": 1,
        "control_plane": "P0_REMOTE_CONTROL_V1_2",
        "commands": {
            "system": ["reboot", "reboot --wait-ready", "shutdown"],
            "browser": ["start", "stop", "restart (two-phase)"],
            "service": {"browser": ["start", "stop", "restart"]},
            "process": ["kill <pid>", "kill-tree <pid>"],
            "debug": ["checkpoint", "doctor"],
        },
    })
    return 0


def build_parser():
    parser = argparse.ArgumentParser(description="Bouchaud OS P0 Remote Control V1.2")
    parser.add_argument("--host", default=DEFAULT_HOST)
    parser.add_argument("--port", type=int, default=lab.PORT_BRDP)
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--token", default=None)
    parser.add_argument("--yes", action="store_true")

    sub = parser.add_subparsers(dest="group", required=True)

    command = sub.add_parser("system")
    command.add_argument("action", choices=["reboot", "shutdown"])
    command.add_argument(
        "--wait-ready",
        action="store_true",
        help="apres reboot, attendre un status BRDP authentifie sur un nouveau boot",
    )
    command.add_argument(
        "--ready-timeout",
        type=float,
        default=90.0,
        help="delai maximal de readiness apres reboot (defaut: 90 s)",
    )
    command.add_argument(
        "--ready-interval",
        type=float,
        default=1.0,
        help="intervalle entre tentatives BRDP (defaut: 1 s)",
    )
    command.set_defaults(func=system_cmd)

    command = sub.add_parser("browser")
    command.add_argument("action", choices=["start", "stop", "restart"])
    command.set_defaults(func=browser_cmd)

    command = sub.add_parser("service")
    command.add_argument("action", choices=["start", "stop", "restart"])
    command.add_argument("service")
    command.set_defaults(func=service_cmd)

    command = sub.add_parser("process")
    command.add_argument("action", choices=["kill", "kill-tree"])
    command.add_argument("pid", type=int)
    command.set_defaults(func=process_cmd)

    command = sub.add_parser("checkpoint")
    command.set_defaults(func=checkpoint_cmd)

    command = sub.add_parser("doctor")
    command.set_defaults(func=doctor_cmd)

    command = sub.add_parser("capabilities")
    command.set_defaults(func=capabilities_cmd)

    return parser


def main():
    args = build_parser().parse_args()
    try:
        if getattr(args, "ready_timeout", 1.0) <= 0:
            raise SystemExit("--ready-timeout doit etre > 0")
        if getattr(args, "ready_interval", 1.0) <= 0:
            raise SystemExit("--ready-interval doit etre > 0")
        return int(args.func(args) or 0)
    except lab.ErreurLab as exc:
        print(f"BRDP: {exc}", file=sys.stderr)
        return 4
    except OSError as exc:
        print(f"reseau: {exc}", file=sys.stderr)
        return 5


if __name__ == "__main__":
    raise SystemExit(main())
