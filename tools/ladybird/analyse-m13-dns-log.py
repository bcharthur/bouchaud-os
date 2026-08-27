#!/usr/bin/env python3
# Summarize Bouchaud/Ladybird M13 DNS diagnostic markers from a serial log.

from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: analyse-m13-dns-log.py <serial-log>")

path = Path(sys.argv[1])
if not path.is_file():
    raise SystemExit(f"log absent: {path}")

text = path.read_text(errors="replace")
lines = text.splitlines()

interesting = [
    line for line in lines
    if "M13_DNS_" in line or "M9_RS_DNS_" in line or "M9_RS_STATE" in line
]

print("=== Marqueurs DNS ===")
if not interesting:
    print("Aucun marqueur M13/M9 DNS trouve.")
    print("Le binaire Ladybird utilise probablement encore un build sans ce patch.")
    raise SystemExit(2)

for line in interesting[-80:]:
    print(line)

def has(marker: str) -> bool:
    return marker in text

def count(marker: str) -> int:
    return text.count(marker)

print()
print("=== Diagnostic ===")

if not has("M9_RS_DNS_QUERY"):
    print("La requete n'atteint pas RequestServer::DNSLookup.")
elif has("M9_RS_DNS_RESOLU"):
    print("DNS RESOLU: LibDNS a rendu une adresse a RequestServer.")
    print("Le prochain blocage est apres DNS (cookie, connect/TLS, HTTP ou rendu).")
elif has("M13_DNS_PROMISE_RESOLVE"):
    print("LibDNS a resolu sa Promise, mais RequestServer n'a pas imprime M9_RS_DNS_RESOLU.")
    print("Piste: callback Promise / transition RequestServer apres DNS.")
elif has("M13_DNS_UNMATCHED"):
    print("Une reponse DNS a ete parsee mais son ID ne correspond a aucune requete en attente.")
    print("Piste: m_pending_lookups / retransmission / routage de datagrammes.")
elif has("M13_DNS_MESSAGE"):
    print("Une reponse DNS a ete lue et parsee, mais la Promise n'a pas ete resolue.")
    print("Piste: matching ID, DNSSEC ou traitement des answers.")
elif has("M13_DNS_NOTIFIER_FIRED"):
    if has("M13_DNS_READY value=true") or has("M13_DNS_READY value=1"):
        print("Le notifier et la readiness fonctionnent, mais aucun message DNS complet n'est parse.")
        print("Piste: recv/FIONREAD/BufferedUDPSocket/parse_one_message.")
    else:
        print("Le notifier LibCore se declenche mais can_read_without_blocking ne voit pas de donnees.")
        print("Piste forte: coherence poll/FIONREAD/socket_readable dans l'ABI Bouchaud.")
elif has("M13_DNS_TIMER_FIRED"):
    print(f"Le timer DNS fonctionne ({count('M13_DNS_TIMER_FIRED')} declenchement(s)), mais aucune reponse n'atteint le notifier.")
    print("Piste: reception/routage UDP, poll du fd DNS, ou reponse absente du DNS QEMU.")
elif has("M13_DNS_TIMER_ARMED"):
    print("Le paquet DNS a ete ecrit et le timer a ete arme, mais ni notifier ni timer ne se declenche.")
    print("Piste forte: EventLoop/poll/timers de RequestServer bloques.")
elif has("M13_DNS_WRITE_OK"):
    print("write_until_depleted a termine, mais le marqueur TIMER_ARMED manque.")
    print("Piste: chemin immediat apres l'ecriture / objet PendingLookup.")
elif has("M13_DNS_WRITE_BEGIN"):
    print("BLOCAGE TRES PROBABLE dans write_until_depleted() / Core::UDPSocket::write.")
    print("WRITE_BEGIN est present mais WRITE_OK ne l'est pas.")
elif has("M13_DNS_SOCKET_OK"):
    print("Le socket DNS est cree, mais on n'atteint pas l'ecriture de la requete.")
    print("Piste: construction/serialisation de la requete DNS dans LibDNS::lookup.")
elif has("M13_DNS_SOCKET_BEGIN"):
    print("Le resolver tente de creer son socket mais la creation ne rend pas la main.")
    print("Piste: Core::UDPSocket::connect / BufferedUDPSocket::create.")
elif has("M13_DNS_LOOKUP_ASYNC"):
    print("LibDNS est entre dans le chemin async, mais n'a pas termine la creation du socket.")
else:
    print("RequestServer a annonce DNS_QUERY mais aucun marqueur interne LibDNS n'apparait.")
    print("Verifier que Ladybird a bien ete reconstruit avec prepare-m13-dns-diagnostics.py.")

if has("M13_DNS_TIMEOUT"):
    print()
    print("Note: LibDNS a atteint son timeout final; l'EventLoop et les timers sont donc vivants.")
