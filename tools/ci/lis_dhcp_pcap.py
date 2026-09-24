#!/usr/bin/env python3
"""Decode les echanges DHCP d'une capture QEMU `filter-dump`.

BOUCHAUD_C76_L_OFFRE_EXISTE_T_ELLE_SUR_LE_FIL

Ce lecteur ne fait qu'une chose : dire ce qui est REELLEMENT passe sur le
fil, champ par champ. Il n'interroge aucun compteur du noyau -- c'est tout
l'interet. `offer_seen=0` dit que le client n'a rien vu ; cette capture dit
si quelque chose est arrive.

Format `filter-dump` : pcap classique, link type Ethernet (1).
"""
import struct
import sys
from pathlib import Path

TYPES = {1: "DISCOVER", 2: "OFFER", 3: "REQUEST", 4: "DECLINE",
         5: "ACK", 6: "NAK", 7: "RELEASE", 8: "INFORM"}


def trames(donnees: bytes):
    if len(donnees) < 24:
        return
    magie = struct.unpack("<I", donnees[:4])[0]
    if magie == 0xa1b2c3d4:
        ordre, nano = "<", False
    elif magie == 0xd4c3b2a1:
        ordre, nano = ">", False
    elif magie == 0xa1b23c4d:
        ordre, nano = "<", True
    elif magie == 0x4d3cb2a1:
        ordre, nano = ">", True
    else:
        sys.exit(f"magie pcap inconnue: {magie:#x}")
    lien = struct.unpack(ordre + "I", donnees[20:24])[0]
    if lien != 1:
        sys.exit(f"link type {lien} : ce lecteur attend de l'Ethernet")
    pos = 24
    while pos + 16 <= len(donnees):
        s, us, taille, _orig = struct.unpack(ordre + "IIII", donnees[pos:pos + 16])
        pos += 16
        corps = donnees[pos:pos + taille]
        pos += taille
        yield (s + us / (1e9 if nano else 1e6)), corps


def mac(octets: bytes) -> str:
    return ":".join(f"{o:02x}" for o in octets)


def ip(octets: bytes) -> str:
    return ".".join(str(o) for o in octets)


def somme_ipv4(entete: bytes) -> int:
    total = 0
    for i in range(0, len(entete), 2):
        total += struct.unpack("!H", entete[i:i + 2])[0]
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return (~total) & 0xFFFF


def decode(trame: bytes):
    if len(trame) < 14:
        return None
    ethertype = struct.unpack("!H", trame[12:14])[0]
    if ethertype != 0x0800:
        return None
    ihl = (trame[14] & 0x0F) * 4
    if len(trame) < 14 + ihl + 8:
        return None
    entete_ip = trame[14:14 + ihl]
    protocole = entete_ip[9]
    if protocole != 17:
        return None
    udp = trame[14 + ihl:]
    sport, dport, ulen, ucks = struct.unpack("!HHHH", udp[:8])
    if {sport, dport} != {67, 68}:
        return None
    bootp = udp[8:]
    if len(bootp) < 240 or bootp[236:240] != b"\x63\x82\x53\x63":
        return None
    op = bootp[0]
    xid = struct.unpack("!I", bootp[4:8])[0]
    type_msg = None
    options = bootp[240:]
    i = 0
    vus = []
    while i < len(options):
        code = options[i]
        if code == 255:
            vus.append(255)
            break
        if code == 0:
            i += 1
            continue
        if i + 1 >= len(options):
            break
        longueur = options[i + 1]
        valeur = options[i + 2:i + 2 + longueur]
        if code == 53 and longueur == 1:
            type_msg = valeur[0]
        vus.append(code)
        i += 2 + longueur
    # La somme d'en-tete IPv4 doit etre nulle une fois recalculee sur place.
    cks_ip_ok = somme_ipv4(entete_ip) == 0
    return dict(
        op=op, xid=xid, type=type_msg,
        src_mac=mac(trame[6:12]), dst_mac=mac(trame[0:6]),
        src_ip=ip(entete_ip[12:16]), dst_ip=ip(entete_ip[16:20]),
        sport=sport, dport=dport, udp_len=ulen, udp_cks=ucks,
        ip_cks_ok=cks_ip_ok, ip_len=struct.unpack("!H", entete_ip[2:4])[0],
        chaddr=mac(bootp[28:34]), flags=struct.unpack("!H", bootp[10:12])[0],
        ciaddr=ip(bootp[12:16]), yiaddr=ip(bootp[16:20]),
        giaddr=ip(bootp[24:28]), options=vus, taille=len(trame),
    )


def main() -> int:
    chemin = Path(sys.argv[1])
    if not chemin.exists():
        print("DHCP_WIRE_FAIL capture absente")
        return 1
    paquets = [(t, decode(c)) for t, c in trames(chemin.read_bytes())]
    dhcp = [(t, d) for t, d in paquets if d]
    if not dhcp:
        print("DHCP_WIRE aucun paquet DHCP sur le fil")
        print("DHCP_WIRE_VERDICT cas=B_aucune_offre discover=0 offre=0")
        return 0

    t0 = dhcp[0][0]
    for t, d in dhcp:
        nom = TYPES.get(d["type"], f"type-{d['type']}")
        sens = "OS->reseau" if d["op"] == 1 else "reseau->OS"
        print(f"DHCP_WIRE t={t - t0:+.3f}s {sens} {nom} xid={d['xid']:#010x} "
              f"src={d['src_ip']}:{d['sport']} dst={d['dst_ip']}:{d['dport']} "
              f"chaddr={d['chaddr']} flags={d['flags']:#06x} "
              f"yiaddr={d['yiaddr']} ip_cks_ok={int(d['ip_cks_ok'])} "
              f"udp_cks={d['udp_cks']:#06x} ip_len={d['ip_len']} "
              f"taille={d['taille']} options={d['options']}")

    discover = [d for _, d in dhcp if d["type"] == 1]
    offre = [d for _, d in dhcp if d["type"] == 2]
    request = [d for _, d in dhcp if d["type"] == 3]
    ack = [d for _, d in dhcp if d["type"] == 5]

    if offre:
        cas = "A_offre_sur_le_fil"
    elif discover:
        cas = "B_aucune_offre"
    else:
        cas = "B_aucun_discover"
    print(f"DHCP_WIRE_VERDICT cas={cas} discover={len(discover)} "
          f"offre={len(offre)} request={len(request)} ack={len(ack)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
