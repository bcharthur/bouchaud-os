#!/usr/bin/env python3
"""Verifie que les appels systeme declares « sans BKL » le meritent vraiment.

Le retrait du gros verrou noyau se fait appel par appel. Chaque retrait repose
sur une preuve, et une preuve fausse ne se voit ni a la compilation ni au boot :
elle se voit un jour, sous charge, a quatre coeurs, sous la forme d'une
corruption qu'on ne saura pas relier a sa cause.

Ce script est la barriere externe. Il relit deux fichiers qui ne se parlent pas
autrement :

  * `src/compat/linux/bkl.rs` -- la table `SANS_BKL` : qui est libere, et pourquoi ;
  * `src/compat/linux/mod.rs` -- l'aiguillage : ce que l'appel fait REELLEMENT.

Et il refuse :

  1. un numero libere qui n'existe pas dans `nr.rs` ;
  2. un numero libere deux fois ;
  3. une ligne sans justification ;
  4. un appel libere dont le bras d'aiguillage fait autre chose que rendre une
     constante -- sauf s'il figure dans `AUDITS_NOMMES` ci-dessous, c'est-a-dire
     s'il a recu un audit ecrit, nomme, qu'on peut relire.

Le point (4) est celui qui compte. Il fait que declarer « sans verrou » un appel
qui touche la table des taches, la memoire utilisateur ou le systeme de fichiers
casse la CI, et non la machine de l'utilisateur. Il fait aussi qu'un appel
aujourd'hui trivial qui cesserait de l'etre -- une constante remplacee par un
vrai calcul -- ramene la question sur la table au lieu de passer inapercu.

Ce que ce script ne peut PAS faire : lire une fonction et decider si elle est
sure. Pour tout ce qui n'est pas une constante, la preuve est humaine, et
`AUDITS_NOMMES` est la liste de celles qui ont ete faites.

Code de retour : 0 si la table et l'aiguillage sont d'accord, 1 sinon.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
# La compatibilite Linux a quitte le coeur du noyau avec la fondation
# multiplateforme (0b3eb17) : ces trois fichiers vivent desormais sous
# `src/compat/linux/`. Le chemin est resolu, et non devine, pour que le
# prochain deplacement echoue en le disant plutot qu'en ne verifiant plus rien.
ABI = RACINE / "src" / "compat" / "linux"
NR = ABI / "nr.rs"
BKL = ABI / "bkl.rs"
DISPATCH = ABI / "mod.rs"

for _chemin in (NR, BKL, DISPATCH):
    if not _chemin.exists():
        raise SystemExit(
            f"verifie-verrouillage : {_chemin} est introuvable.\n"
            "La table des appels systeme a ete deplacee : mets ce chemin a jour,\n"
            "sinon cette barriere ne verifie plus rien."
        )

# Les appels liberes dont le bras d'aiguillage n'est PAS une constante, et dont
# l'audit est donc humain. Chaque entree nomme ou lire cet audit ; sans cela, la
# ligne n'a pas sa place ici.
AUDITS_NOMMES = {
    # c5 -- audit ecrit au-dessus de la ligne FUTEX dans SANS_BKL. Le point qui
    # tranche : `futex_wait`/`futex_wake` appelaient DEJA
    # `smp_lock::suspend_for_schedule()` pour rendre le verrou que l'aiguilleur
    # venait de prendre. Le coeur `wait_word` est a seaux verrouilles, sans
    # parcours de la table des taches. Le compteur `[BKL-FUTEX] herites=` doit
    # rester nul : c'est la falsification runtime de cet audit.
    "FUTEX": "c5 -- wait-word a seaux verrouilles ; le chemin suspendait deja le verrou",
    # c6 -- audit ecrit au-dessus du lot dans SANS_BKL. Meme domaine que POLL :
    # table des descripteurs + verrou par objet. Le point qui a change :
    # `current_process()` ne reprend plus le gros verrou.
    "EVENTFD": "c6 -- table des descripteurs + objet cree sur place",
    "EVENTFD2": "c6 -- table des descripteurs + objet cree sur place",
    "TIMERFD_CREATE": "c6 -- table des descripteurs + objet cree sur place",
    "TIMERFD_SETTIME": "c6 -- verrou de l'objet minuterie + horloges atomiques + Mm",
    "TIMERFD_GETTIME": "c6 -- verrou de l'objet minuterie + horloges atomiques + Mm",
    "PIPE": "c6 -- table des descripteurs + etat de tube + Mm",
    "PIPE2": "c6 -- table des descripteurs + etat de tube + Mm",
    "EPOLL_CTL": "c6 -- table des descripteurs + verrou de la liste epoll + Mm",
    # c7 -- le contrat de `sleep_ticks` a ete change a la source : il n'exige
    # plus le gros verrou de son appelant, parce qu'il ne s'en servait pas.
    "NANOSLEEP": "c7 -- stores atomiques + Mm ; la boucle tournait deja sans verrou",
    "CLOCK_NANOSLEEP": "c7 -- identique a NANOSLEEP, avec l'echeance absolue",
    # c8/v2 -- objets locaux + receive-side ; pump inet interne.
    "SOCKET": "c8v2 -- creation locale + table FD",
    "SOCKETPAIR": "c8v2 -- Canaux + table FD + Mm",
    "BIND": "c8v2 -- SocketState + AtomicU16",
    "GETPEERNAME": "c8v2 -- SocketState + Mm",
    "SETSOCKOPT": "c8v2 -- no-op",
    "GETSOCKOPT": "c8v2 -- SocketState/Canal + Mm",
    "RECVFROM": "c8v2 -- pump inet interne + attente hors BKL",
    "RECVMSG": "c8v2 -- canaux/FD + RECVFROM",
    "RECVMMSG": "c8v2 -- RECVMSG + safe point",
    "MPROTECT": "jalon SMP4 -- domaine Arc<Process>::Mm + protocole TLB sur IRQ",
    "BRK": "jalon SMP4 -- domaine Arc<Process>::Mm + protocole TLB sur IRQ",
    # A1 lot 2 -- voir l'en-tete de bkl.rs et la preuve de duree de vie sur
    # `task::identite_courante`. Chacun de ces appels a son audit ecrit en
    # commentaire au-dessus de sa ligne dans SANS_BKL.
    # A1 lot 3 -- voir l'en-tete de bkl.rs et l'audit ecrit au-dessus de la
    # ligne POLL dans SANS_BKL : domaine table des descripteurs + verrou par
    # objet, `current_process_local` a la place de `current_process`, et les
    # trois branches a etat global qui prennent le verrou elles-memes.
    "POLL": "A1 lot 3 -- table des descripteurs + verrou par objet",
    "PPOLL": "A1 lot 3 -- table des descripteurs + verrou par objet",
    "WRITE": "V14 -- copyin sans BKL + domaines locaux; sinks legacy verrouillent en interne",
    "WRITEV": "V14 -- iovec/copyin sans BKL + chemin WRITE audite",
    "READ": "c1 -- audit branche par branche au-dessus de la ligne READ dans SANS_BKL ; "
            "decodeur clavier et generateur d'alea verrouillees par ce meme lot, "
            "branche socket bornee en interne",
    "READV": "c1 -- boucle sur `sys_read`, meme audit",
    "GETRANDOM": "c13 -- audit au-dessus de la ligne GETRANDOM dans SANS_BKL ; "
                 "le generateur porte son SpinLock depuis c1, `user_write` passe "
                 "par `current_process_local`, aucune lecture de TASKS",
    "MUNMAP": "V14 -- Mm + TLB + caches SMP-safe, aucun writeback sous verrou externe",
    "MADVISE": "V14 -- Mm + TLB + clean-cache SMP-safe",
    "GETPID": "A1 lot 2 -- domaine CPU-local, aucune lecture de TASKS",
    "GETTID": "A1 lot 2 -- domaine CPU-local, aucune lecture de TASKS",
    "GETUID": "A1 lot 2 -- domaine CPU-local + verrou metadata du Process",
    "GETEUID": "A1 lot 2 -- domaine CPU-local + verrou metadata du Process",
    "GETGID": "A1 lot 2 -- domaine CPU-local + verrou metadata du Process",
    "GETEGID": "A1 lot 2 -- domaine CPU-local + verrou metadata du Process",
    "CLOCK_GETTIME": "A1 lot 2 -- horloges atomiques + Mm ; verrou local a la branche CPUTIME",
    "CLOCK_GETRES": "A1 lot 2 -- constante calculee + Mm",
    "GETTIMEOFDAY": "A1 lot 2 -- ancre d'epoque atomique + Mm",
    "TIME": "A1 lot 2 -- ancre d'epoque atomique + Mm",
    # c2 -- audit ecrit au-dessus du lot dans SANS_BKL : registre du CPU
    # courant, champ de la tache courante par garde d'emplacement, et
    # `user_write` qui prend le verrou `mm` du processus.
    "ARCH_PRCTL": "c2 -- FS_BASE du CPU courant + champ de la tache courante + verrou mm",
    "SET_TID_ADDRESS": "c2 -- champ de la tache courante, par garde d'emplacement",
    "SCHED_GETAFFINITY": "c2 -- masque constant + verrou mm ; aucune table parcourue",
    "GETPRIORITY": "c2 -- atomique par tache ; aucune table parcourue",
    # c3 -- audit ecrit au-dessus des deux lots dans SANS_BKL : table des
    # descripteurs pour les cinq premiers, ordonnanceur pour les deux derniers.
    "FSTAT": "c3 -- table des descripteurs + domaines Fs/Vfs declares sortis",
    "LSEEK": "c3 -- table des descripteurs + domaine Fs declare sorti",
    "DUP": "c3 -- table des descripteurs seule",
    "DUP2": "c3 -- table des descripteurs seule",
    "DUP3": "c3 -- table des descripteurs seule",
    "SCHED_YIELD": "c3 -- `schedule()` relache deja le verrou pour commuter",
    "SETPRIORITY": "c3 -- lecture du registre (domaine sorti) + atomique par tache",
    # c4 -- audit ecrit au-dessus du lot dans SANS_BKL. Le point qui tranche
    # pour `mmap` : `peuple_a_la_demande` est le gestionnaire de faute de page,
    # qui n'a jamais pu dependre du gros verrou.
    "MMAP": "c4 -- mm + descripteurs + metadata + Fs, et la faute de page sans verrou",
    "CLOSE": "c4 -- descripteurs, verrous d'enregistrement et readiness : trois domaines sortis",
    # B1 -- retrait complet du gros verrou, lot 1. Audit ecrit au-dessus du
    # lot dans SANS_BKL.
    "UNAME": "B1 -- tampon sur pile + chaines constantes + verrou mm",
    "SYSINFO": "B1 -- FRAMES (SpinLockIrq) + ticks atomiques + verrou mm",
    "GETRLIMIT": "B1 -- limite_as sous verrou mm",
    "SETRLIMIT": "B1 -- limite_as sous verrou mm",
    "PRLIMIT64": "B1 -- limite_as sous verrou mm",
    # B2 -- audit ecrit au-dessus de la ligne FCNTL dans SANS_BKL.
    "FCNTL": "B2 -- descripteurs + VERROUS (PosixRecord) + FS/EXTENTS + mm",
    # B3 -- audit ecrit au-dessus du lot dans SANS_BKL.
    "FSYNC": "B3 -- descripteurs + FS + TRANSACTION",
    "FDATASYNC": "B3 -- meme chemin que fsync",
    "SYNC": "B3 -- TRANSACTION (SleepMutex) + FS",
    "MSYNC": "B3 -- mm + CACHE partage + FS",
    "MREMAP": "B3 -- composition d'appels deja hors BKL",
    # B4 -- audit ecrit au-dessus du lot dans SANS_BKL.
    "EPOLL_CREATE": "B4 -- table des descripteurs seule",
    "EPOLL_CREATE1": "B4 -- table des descripteurs seule",
    "EPOLL_WAIT": "B4 -- liste copiee sous son verrou ; readiness comme poll",
    "EPOLL_PWAIT": "B4 -- meme chemin qu'epoll_wait",
    "SELECT": "B4 -- readiness comme poll",
    "PSELECT6": "B4 -- meme chemin que select",
    # B5 -- audit ecrit au-dessus du lot dans SANS_BKL (systeme de fichiers).
    "OPEN": "B5 -- VFS : FS + descripteurs + metadata ; creation revue sous la prise qui cree",
    "OPENAT": "B5 -- meme chemin qu'open ; affichage sous BASCULE, souris sous ARMEMENT",
    "ACCESS": "B5 -- resolution sous FS",
    "FACCESSAT": "B5 -- resolution sous FS",
    "STAT": "B5 -- resolution sous FS + mm",
    "LSTAT": "B5 -- resolution sous FS + mm",
    "NEWFSTATAT": "B5 -- fstat ou stat, deja audites + mm",
    "STATX": "B5 -- descripteurs + FS + EXTENTS + mm",
    "READLINK": "B5 -- metadata du processus + mm",
    "READLINKAT": "B5 -- metadata du processus + mm",
    "GETDENTS64": "B5 -- descripteurs + FS ; copie sous FS, ecriture utilisateur apres",
    "GETCWD": "B5 -- metadata + FS + mm",
    "CHDIR": "B5 -- metadata + FS",
    "MKDIR": "B5 -- une seule prise de FS",
    "MKDIRAT": "B5 -- une seule prise de FS",
    "UNLINK": "B5 -- resolution ET retrait sous une seule prise de FS",
    "UNLINKAT": "B5 -- resolution ET retrait sous une seule prise de FS",
    "RENAME": "B5 -- source, cible et deplacement sous une seule prise de FS",
    "STATFS": "B5 -- resolution sous FS + mm",
    "FSTATFS": "B5 -- descripteurs + mm",
    "FTRUNCATE": "B5 -- descripteurs + EXTENTS + FS",
    "PREAD64": "B5 -- lecture positionnee, sans toucher au decalage partage",
    "PWRITE64": "B5 -- ecriture positionnee sous une seule prise de FS",
    "SENDFILE": "B5 -- descripteurs + backing + chemin d'ecriture deja audite",
    "MEMFD_CREATE": "B5 -- FS + descripteurs + mm",
    # B6 -- audit ecrit au-dessus du lot dans SANS_BKL (signaux).
    "RT_SIGACTION": "B6 -- signals du processus, lecture/remplacement sous une prise ; mm hors verrou",
    "RT_SIGPROCMASK": "B6 -- masque calcule et ecrit sous une seule prise de signals ; mm hors verrou",
    "RT_SIGPENDING": "B6 -- signals + mm",
    "RT_SIGRETURN": "B6 -- trame restauree depuis la pile utilisateur (mm) + signals",
    "RT_SIGSUSPEND": "B6 -- signals + attente d'interruption (registre en lecture, pas de BKL)",
    "PAUSE": "B6 -- meme chemin que sigsuspend",
    "KILL": "B6 -- PROCESSES (SpinLock) + signals + reveil par CAS Blocked->Ready",
    "TKILL": "B6 -- registre des taches en lecture + meme chemin que kill",
    "TGKILL": "B6 -- meme chemin que kill",
    "ALARM": "B6 -- ALARMES (SpinLock) + ticks atomiques",
    "GETITIMER": "B6 -- ALARMES + mm",
    "SETITIMER": "B6 -- ALARMES + mm",
}

# Une constante rendue directement : `0`, `1`, `0o022`, `-errno::ENOSYS`.
#
# Les bases OCTALE et HEXADECIMALE comptent : `umask` rend `0o022`, qui est
# aussi litteral que `0`. Ne pas les reconnaitre obligeait a inscrire un audit
# HUMAIN pour une constante -- et un audit humain pour ce qui n'en demande pas
# use la liste des audits, qui ne vaut que par ce qu'elle contient vraiment.
CONSTANTE = re.compile(
    r"^-?(?:0[oObBxX][0-9a-fA-F_]+|\d[\d_]*|errno::[A-Z0-9_]+)$"
)

# BOUCHAUD_P3_POLL_SANS_BKL_V1
#
# Les sondes de readiness de `poll` ne doivent JAMAIS passer par
# `task::current_process()`, qui reprend le gros verrou. Remettre cet appel les
# ferait acquerir le verrou depuis zero une fois par descripteur et par sonde --
# 70 000 fois par seconde sur un vrai Ladybird -- et la liberation de `poll`
# couterait plus qu'elle ne rapporte, sans que rien ne le signale.
SONDES_READINESS = ("readable", "writable", "etat_pair", "readiness_deadline_ns")
FICHIER_SONDES = "src/compat/linux/file.rs"
FICHIER_NET = "src/compat/linux/net.rs"

erreurs = []


def echec(message):
    erreurs.append(message)


def verifie_sondes_readiness():
    """Aucune sonde de readiness ne doit reprendre le gros verrou."""
    chemin = RACINE / FICHIER_SONDES
    if not chemin.exists():
        echec(f"{FICHIER_SONDES} introuvable : impossible de verifier les sondes")
        return
    lignes = chemin.read_text(encoding="utf-8").splitlines()

    courante = None
    profondeur = 0
    for numero, brute in enumerate(lignes, 1):
        ligne = brute.split("//")[0]
        for sonde in SONDES_READINESS:
            if re.search(rf"\bfn\s+{sonde}\s*\(", ligne):
                courante = sonde
                profondeur = 0
        if courante is None:
            continue
        if "task::current_process()" in ligne:
            echec(
                f"{FICHIER_SONDES}:{numero} `{courante}` appelle "
                f"`task::current_process()`, qui REPREND le gros verrou.\n"
                f"           {brute.strip()}\n"
                "           Utilise `current_process_local()` : sans cela, "
                "liberer `poll` coute\n"
                "           une acquisition par descripteur et par sonde, soit "
                "plus que le gain."
            )
        profondeur += ligne.count("{") - ligne.count("}")
        if profondeur <= 0 and "}" in ligne:
            courante = None


def verifie_c8_receive_side():
    net_path = RACINE / FICHIER_NET
    file_path = RACINE / FICHIER_SONDES
    if not net_path.exists() or not file_path.exists():
        echec("C8/V2 : net.rs ou file.rs introuvable")
        return

    net = net_path.read_text(encoding="utf-8")
    file = file_path.read_text(encoding="utf-8")

    # L'audit de BIND repose sur un port ephemere ATOMIQUE. Le compteur a
    # quitte net.rs pour `net::port_ephemere` (BOUCHAUD_PORT_EPHEMERE_MONOTONE_V1),
    # partage avec TCP : la garde suit la garantie ou elle vit desormais.
    for marqueur in [
        "BOUCHAUD_C8_RECV_SANS_BKL_V2",
        "crate::net::port_ephemere()",
        "Domaine::Reseau",
        "crate::kernel::scheduler::preempt::safe_point()",
    ]:
        if marqueur not in net:
            echec(f"C8/V2 net.rs : marqueur absent `{marqueur}`")
    pile = RACINE / "src" / "net" / "mod.rs"
    if not pile.exists() or "static PROCHAIN_PORT_EPHEMERE: AtomicU16" not in pile.read_text(encoding="utf-8"):
        echec("C8/V2 net/mod.rs : compteur atomique de port ephemere absent")

    # C8 : trois pumps partagent UNE frontiere legacy.
    # Le nombre de sites BKL est lui-meme un budget d'architecture.
    if "BOUCHAUD_C8_RESEAU_BKL_BORNE_V1" not in net:
        echec("C8/CI net.rs : frontiere reseau bornee absente")
    if net.count("fn avec_domaine_reseau") != 1:
        echec("C8/CI net.rs : la frontiere reseau doit avoir une seule definition")
    if net.count("avec_domaine_reseau(||") != 3:
        echec(
            "C8/CI net.rs : attendu 3 usages de la frontiere "
            "(TCP + deux pumps UDP)"
        )

    if "BOUCHAUD_C8_READ_SOCKET_SANS_BKL_EXTERNE_V2" not in file:
        echec("C8/V2 file.rs : marqueur read socket absent")
    debut = file.find("FdKind::Socket(_) =>")
    fin = file.find("FdKind::SocketPair", debut)
    branche = file[debut:fin if fin > debut else len(file)]
    if "smp_lock::enter()" in branche:
        echec("C8/V2 file.rs : read(socket) reprend encore un BKL externe")


def numeros_syscalls():
    valeurs = {}
    for nom, valeur in re.findall(
        r"pub const (\w+): u64 = (\d+);", NR.read_text(encoding="utf-8")
    ):
        valeurs[nom] = int(valeur)
    return valeurs


def table_sans_bkl():
    """Lit `SANS_BKL` : la liste des (nr::NOM, justification)."""
    source = BKL.read_text(encoding="utf-8")
    debut = source.find("pub const SANS_BKL")
    if debut < 0:
        echec("bkl.rs : table SANS_BKL introuvable")
        return []
    fin = source.find("];", debut)
    corps = source[debut:fin]
    # Les commentaires portent la justification longue ; la ligne, la courte.
    corps = re.sub(r"//[^\n]*", "", corps)
    return re.findall(r"\(\s*nr::(\w+)\s*,\s*\"([^\"]*)\"\s*\)", corps)


def bras_constants():
    """Les noms d'appels dont le bras d'aiguillage rend une constante."""
    source = DISPATCH.read_text(encoding="utf-8")
    debut = source.find("fn dispatch(")
    if debut < 0:
        echec("mod.rs : fonction dispatch introuvable")
        return {}
    corps = source[debut:]
    constants = {}
    for ligne in corps.splitlines():
        m = re.match(r"^ {8}([A-Z][A-Z0-9_]*(?:\s*\|\s*[A-Z][A-Z0-9_]*)*)\s*=>\s*(.+?),?$", ligne)
        if not m:
            continue
        motifs, valeur = m.group(1), m.group(2).strip().rstrip(",").strip()
        if not CONSTANTE.match(valeur):
            continue
        for nom in re.split(r"\s*\|\s*", motifs):
            constants[nom] = valeur
    return constants


def main():
    numeros = numeros_syscalls()
    if not numeros:
        echec("nr.rs : aucun numero d'appel systeme lu")
    table = table_sans_bkl()
    constants = bras_constants()

    vus = {}
    for nom, justification in table:
        if nom not in numeros:
            echec(f"SANS_BKL : `nr::{nom}` n'existe pas dans nr.rs")
            continue
        numero = numeros[nom]
        if numero in vus:
            echec(f"SANS_BKL : le numero {numero} est libere deux fois "
                  f"({vus[numero]} et {nom})")
        vus[numero] = nom
        if not justification.strip():
            echec(f"SANS_BKL : `nr::{nom}` est libere sans justification")
        if nom in AUDITS_NOMMES:
            continue
        if nom not in constants:
            echec(
                f"SANS_BKL : `nr::{nom}` est libere, mais son bras d'aiguillage "
                f"ne rend pas une constante et il n'a pas d'audit nomme. "
                f"Soit on ecrit l'audit dans AUDITS_NOMMES, soit on le remet "
                f"sous le gros verrou."
            )

    verifie_sondes_readiness()
    verifie_c8_receive_side()

    for nom in AUDITS_NOMMES:
        if nom not in vus.values():
            echec(f"AUDITS_NOMMES : `{nom}` n'est plus libere ; retirer son audit")

    print(f"nr.rs           : {len(numeros)} numeros d'appels systeme")
    print(f"SANS_BKL        : {len(table)} appels liberes")
    print(f"  dont constants: {sum(1 for n, _ in table if n in constants)}")
    print(f"  dont audites  : {sum(1 for n, _ in table if n in AUDITS_NOMMES)}")

    if erreurs:
        print()
        for message in erreurs:
            print(f"  ECHEC {message}")
        print(f"\n{len(erreurs)} probleme(s)")
        return 1
    print("\ntable et aiguillage d'accord")
    return 0


if __name__ == "__main__":
    sys.exit(main())
