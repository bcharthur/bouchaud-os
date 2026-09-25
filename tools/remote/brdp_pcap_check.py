#!/usr/bin/env python3
"""Verifie la capture QEMU/TAP du banc BRDP sans scapy ni tcpdump."""

from __future__ import annotations

import argparse
import ipaddress
import json
import struct
from pathlib import Path


def ip4(raw: bytes) -> str:
    return str(ipaddress.IPv4Address(raw))


def charge_pcap(path: Path):
    data = path.read_bytes()
    if len(data) < 24:
        raise ValueError("pcap trop court")
    magic = data[:4]
    if magic == b"\xd4\xc3\xb2\xa1":
        endian, nano = "<", False
    elif magic == b"\xa1\xb2\xc3\xd4":
        endian, nano = ">", False
    elif magic == b"\x4d\x3c\xb2\xa1":
        endian, nano = "<", True
    elif magic == b"\xa1\xb2\x3c\x4d":
        endian, nano = ">", True
    else:
        raise ValueError("format pcap inconnu (pcapng non accepte)")
    _, _, _, _, _, snaplen, network = struct.unpack(endian + "IHHIIII", data[:24])
    if network != 1:
        raise ValueError(f"DLT inattendu: {network}, Ethernet attendu")
    pos = 24
    while pos + 16 <= len(data):
        ts_sec, ts_frac, caplen, origlen = struct.unpack(endian + "IIII", data[pos:pos+16])
        pos += 16
        frame = data[pos:pos+caplen]
        pos += caplen
        if len(frame) != caplen:
            break
        frac_div = 1_000_000_000 if nano else 1_000_000
        yield ts_sec + ts_frac / frac_div, frame


def analyse(path: Path, guest_ip: str, guest_mac: str, port: int = 2222):
    guest_mac_b = bytes.fromhex(guest_mac.replace(":", ""))
    syns = []
    synacks = []
    dhcp = []
    guest_rsts = 0
    rst_recyclage = 0
    arp_req = 0
    arp_rep = 0

    for ts, f in charge_pcap(path):
        if len(f) < 14:
            continue
        dst, src, ethertype = f[:6], f[6:12], struct.unpack("!H", f[12:14])[0]
        if ethertype == 0x0806 and len(f) >= 42:
            arp = f[14:42]
            htype, ptype, hlen, plen, op = struct.unpack("!HHBBH", arp[:8])
            if htype == 1 and ptype == 0x0800 and hlen == 6 and plen == 4:
                spa = ip4(arp[14:18])
                tpa = ip4(arp[24:28])
                sha = arp[8:14]
                if op == 1 and tpa == guest_ip:
                    arp_req += 1
                if op == 2 and spa == guest_ip and sha == guest_mac_b:
                    arp_rep += 1
            continue
        if ethertype != 0x0800 or len(f) < 34:
            continue
        ip = f[14:]
        ihl = (ip[0] & 0x0F) * 4
        if len(ip) < ihl + 8 or ihl < 20:
            continue
        proto = ip[9]
        src_ip, dst_ip = ip4(ip[12:16]), ip4(ip[16:20])
        l4 = ip[ihl:]
        if proto == 6 and len(l4) >= 20:
            sport, dport = struct.unpack("!HH", l4[:4])
            seq, ack_num = struct.unpack("!II", l4[4:12])
            flags = l4[13]
            syn = bool(flags & 0x02)
            ack = bool(flags & 0x10)
            rst = bool(flags & 0x04)
            if dst_ip == guest_ip and dport == port and syn and not ack:
                syns.append({
                    "ts": ts,
                    "src_ip": src_ip,
                    "sport": sport,
                    "seq": seq,
                    "terminee_par_rst": False,
                })
            if src_ip == guest_ip and sport == port and syn and ack:
                synacks.append({
                    "ts": ts,
                    "dst_ip": dst_ip,
                    "dport": dport,
                    "ack": ack_num,
                })
            if src_ip == guest_ip and sport == port and rst:
                guest_rsts += 1
                for tentative in reversed(syns):
                    if tentative["terminee_par_rst"]:
                        continue
                    if tentative["src_ip"] != dst_ip or tentative["sport"] != dport:
                        continue
                    if ack and ack_num != ((tentative["seq"] + 1) & 0xFFFFFFFF):
                        continue
                    tentative["terminee_par_rst"] = True
                    rst_recyclage += 1
                    break
        elif proto == 17 and len(l4) >= 8:
            sport, dport = struct.unpack("!HH", l4[:4])
            if {sport, dport} & {67, 68}:
                dhcp.append(ts)

    # Apparier SYN et SYN/ACK par la semantique TCP (ACK == ISN+1).
    latences = []
    paires_sans_dhcp = 0
    utilises = set()
    for syn in syns:
        if syn["terminee_par_rst"]:
            continue
        attendu = (syn["seq"] + 1) & 0xFFFFFFFF
        choisi = None
        for j, sa in enumerate(synacks):
            if j in utilises:
                continue
            if sa["ts"] < syn["ts"]:
                continue
            if sa["dst_ip"] != syn["src_ip"] or sa["dport"] != syn["sport"]:
                continue
            if sa["ack"] != attendu:
                continue
            choisi = (j, sa)
            break
        if choisi is None:
            continue
        j, sa = choisi
        utilises.add(j)
        ack_ts = sa["ts"]
        latences.append((ack_ts - syn["ts"]) * 1000.0)
        if not any(syn["ts"] <= d <= ack_ts for d in dhcp):
            paires_sans_dhcp += 1

    return {
        "syn_2222": len(syns),
        "synack_2222": len(synacks),
        "paires": len(latences),
        "latence_syn_synack_max_ms": round(max(latences), 3) if latences else None,
        "paires_sans_dhcp_entre_syn_synack": paires_sans_dhcp,
        "rst_guest_port_2222": guest_rsts,
        "rst_recyclage_brdp": rst_recyclage,
        "rst_guest_inattendus": guest_rsts - rst_recyclage,
        "dhcp_trames": len(dhcp),
        "arp_requests_guest": arp_req,
        "arp_replies_guest": arp_rep,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("pcap")
    ap.add_argument("--guest-ip", required=True)
    ap.add_argument("--guest-mac", default="52:54:00:12:34:56")
    ap.add_argument("--port", type=int, default=2222)
    ap.add_argument("--max-synack-ms", type=float, default=1500.0)
    ap.add_argument("--out")
    args = ap.parse_args()

    r = analyse(Path(args.pcap), args.guest_ip, args.guest_mac, args.port)
    erreurs = []
    if r["paires"] < 3:
        erreurs.append(f"seulement {r['paires']} paire(s) SYN/SYNACK")
    if r["latence_syn_synack_max_ms"] is None or r["latence_syn_synack_max_ms"] > args.max_synack_ms:
        erreurs.append(
            f"latence SYN/SYNACK {r['latence_syn_synack_max_ms']} ms > {args.max_synack_ms} ms"
        )
    if r["paires_sans_dhcp_entre_syn_synack"] < 1:
        erreurs.append("aucune connexion prouvee independante d'un reveil DHCP")
    if r["rst_guest_inattendus"] != 0:
        erreurs.append(
            f"{r['rst_guest_inattendus']} RST BRDP inattendu(s) "
            f"({r['rst_guest_port_2222']} total, "
            f"{r['rst_recyclage_brdp']} rattache(s) a des SYN refuses)"
        )
    if r["arp_requests_guest"] and r["arp_replies_guest"] == 0:
        erreurs.append("requete ARP vers le guest vue, aucune reponse ARP du guest")

    r["ok"] = not erreurs
    r["erreurs"] = erreurs
    texte = json.dumps(r, indent=2, sort_keys=True)
    print(texte)
    if args.out:
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(texte + "\n", encoding="utf-8")
    if erreurs:
        print("BOUCHAUD_BRDP_PCAP_ECHEC")
        return 1
    print("BOUCHAUD_BRDP_PCAP_OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
