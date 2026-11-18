#!/usr/bin/env python3
"""W^X : aucune page n'est inscriptible et executable en meme temps.

Une page qui porte les deux droits est exactement ce dont une injection de code
a besoin, et rien de plus : ecrire des octets, puis les executer, sans avoir a
contourner quoi que ce soit. Un debordement qui atteint un `mmap` suffit.

Ces regles protegent les TROIS portes par lesquelles une telle page peut
naitre : `mmap`, `mprotect`, et le chargement d'un segment ELF. Une seule
laissee ouverte annule les deux autres.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
WX = RACINE / "src/kernel/security/wx.rs"
MEM = RACINE / "src/compat/linux/mem.rs"
ELF = RACINE / "src/kernel/process/elf.rs"
TEST = RACINE / "tools/platform/test_wx.rs"
CAPACITES = RACINE / "src/kernel/security/capability.rs"
PROFIL = RACINE / "src/kernel/security/profile.rs"
POLITIQUE = RACINE / "src/kernel/security/policy.rs"
MEMOIRE = RACINE / "src/kernel/security/memory.rs"


def sans_commentaires(source):
    sortie = []
    dans_chaine = False
    i = 0
    while i < len(source):
        c = source[i]
        if dans_chaine:
            if c == "\\":
                sortie.append("  ")
                i += 2
                continue
            if c == '"':
                dans_chaine = False
            sortie.append(c)
            i += 1
            continue
        if c == '"':
            dans_chaine = True
            sortie.append(c)
            i += 1
            continue
        if c == "/" and i + 1 < len(source) and source[i + 1] == "/":
            while i < len(source) and source[i] != "\n":
                sortie.append(" ")
                i += 1
            continue
        sortie.append(c)
        i += 1
    return "".join(sortie)


def corps(source, signature):
    debut = 0
    while True:
        trouve = source.find(signature, debut)
        if trouve < 0:
            return None
        ouvrante = -1
        declaration = -1
        profondeur = 0
        i = trouve
        while i < len(source):
            c = source[i]
            if c in "([<":
                profondeur += 1
            elif c in ")]>":
                if profondeur > 0:
                    profondeur -= 1
            elif profondeur == 0:
                if c == "{":
                    ouvrante = i
                    break
                if c == ";":
                    declaration = i
                    break
            i += 1
        if ouvrante >= 0:
            break
        if declaration < 0:
            return None
        debut = declaration + 1
    profondeur = 0
    i = ouvrante
    while i < len(source):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvrante : i + 1]
        i += 1
    return None


def regle_la_regle_existe(wx, fautes):
    """La regle porte sur l'ABSENCE du bit de non-execution.

    L'execution est le DEFAUT sur x86-64 : une page est executable des que le
    bit 63 est absent. Une regle qui chercherait un bit d'execution ne
    trouverait jamais rien, et laisserait tout passer.
    """
    bloc = corps(wx, "pub const fn inscriptible_et_executable(")
    if bloc is None:
        fautes.append("wx.rs : la regle des drapeaux a disparu.")
        return
    if "NON_EXECUTABLE == 0" not in bloc:
        fautes.append(
            "wx.rs : la regle ne porte plus sur l'ABSENCE du bit de "
            "non-execution. Sur x86-64 l'execution est le defaut : une regle "
            "qui cherche un bit d'execution ne trouve jamais rien."
        )
    if "ECRITURE != 0" not in bloc:
        fautes.append("wx.rs : la regle ne regarde plus le droit d'ecriture.")
    valeur = re.search(r"NON_EXECUTABLE:\s*u64\s*=\s*1\s*<<\s*(\d+)", wx)
    if valeur is None or valeur.group(1) != "63":
        fautes.append(
            "wx.rs : le bit de non-execution n'est plus le bit 63 ; il ne "
            "designerait plus rien dans une entree de table de pages."
        )


def regle_les_trois_portes(mem, elf, fautes):
    """`mmap`, `mprotect` et le chargement ELF. Une seule ouverte annule tout."""
    mmap = corps(mem, "pub fn sys_mmap(")
    if mmap is None:
        fautes.append("mem.rs : sys_mmap introuvable.")
    elif "prot_to_flags_verifie_jit(prot," not in mmap:
        fautes.append(
            "mem.rs : `mmap` ne verifie plus W^X. Une demande "
            "`PROT_WRITE | PROT_EXEC` donnerait la page dont une injection de "
            "code a besoin."
        )
    elif "EACCES" not in mmap:
        fautes.append("mem.rs : `mmap` ne refuse plus la demande violant W^X.")

    mprotect = corps(mem, "pub fn sys_mprotect(")
    if mprotect is None:
        fautes.append("mem.rs : sys_mprotect introuvable.")
    else:
        if "prot_to_flags_verifie_jit(prot," not in mprotect:
            fautes.append(
                "mem.rs : `mprotect` ne verifie plus W^X. C'est la voie la "
                "plus directe : une page deja ecrite qu'on rend executable "
                "sans retirer l'ecriture donne le meme resultat en une etape "
                "de moins."
            )
        # Le controle doit venir AVANT toute modification.
        controle = mprotect.find("prot_to_flags_verifie")
        for modification in ("vma::protege", "prepare_protect"):
            position = mprotect.find(modification)
            if controle >= 0 and position >= 0 and controle > position:
                fautes.append(
                    "mem.rs : `mprotect` verifie W^X APRES avoir modifie les "
                    "protections (%s) ; la page a deja porte les deux droits."
                    % modification
                )

    charge = corps(elf, "fn page_flags(")
    if charge is None:
        fautes.append("elf.rs : page_flags introuvable.")
    elif "segment_viole_wx" not in charge:
        fautes.append(
            "elf.rs : un segment declare inscriptible ET executable est de "
            "nouveau charge tel quel. Aucune chaine de compilation moderne "
            "n'en emet : un binaire qui en porte un a ete fabrique pour cela."
        )
    # Les deux appelants doivent traiter le refus.
    appels = re.findall(r"page_flags\(ph\.flags\)", elf)
    refus = re.findall(r"let Some\(drapeaux\) = page_flags\(ph\.flags\) else", elf)
    if len(refus) != len(appels):
        fautes.append(
            "elf.rs : %d appel(s) a page_flags et %d refus traites ; un "
            "segment refuse serait charge par le chemin qui ne regarde pas."
            % (len(appels), len(refus))
        )


def regle_le_refus_est_visible(mem, fautes):
    """Un refus se compte et se dit.

    Une machine qui en accumule dit qu'un programme du systeme demande une page
    inscriptible et executable -- et c'est une chose qu'on veut SAVOIR, pas
    seulement empecher.
    """
    if "WX_REFUSES" not in mem:
        fautes.append("mem.rs : les refus W^X ne sont plus comptes.")
    if "BOUCHAUD_WX_REFUSE" not in mem:
        fautes.append("mem.rs : un refus W^X ne laisse plus de trace au journal.")


def regle_pas_de_correction_silencieuse(mem, fautes):
    """On refuse, on ne corrige pas.

    Retirer discretement l'execution rendrait une page qui n'est pas celle
    qu'on a demandee, et l'appelant s'en apercevrait par une faute loin de la
    cause. Retirer l'ecriture ferait echouer la premiere ecriture. Les deux
    mentent.
    """
    bloc = corps(mem, "pub fn prot_to_flags_verifie(")
    if bloc is None:
        fautes.append("mem.rs : prot_to_flags_verifie introuvable.")
        return
    if "return None" not in bloc:
        fautes.append(
            "mem.rs : `prot_to_flags_verifie` ne refuse plus ; s'il corrigeait "
            "les drapeaux, il rendrait une page qui n'est pas celle demandee."
        )
    if "|= vmm::PTE_NO_EXEC" in bloc or "&= !vmm::PTE_WRITE" in bloc:
        fautes.append(
            "mem.rs : `prot_to_flags_verifie` corrige les drapeaux au lieu de "
            "refuser. Une correction silencieuse se decouvre par une faute "
            "loin de sa cause."
        )


def regle_exception_jit_bornee(wx, mem, capacites, profil, politique, memoire, fautes):
    """BOUCHAUD_JIT_RWX_V1 : UNE exception, et rien qu'elle.

    Une JVM (HotSpot) ecrit son code compile dans des pages qu'elle execute.
    L'exception l'autorise pour un lanceur Java installe par root, sur de la
    memoire anonyme et privee. Chaque condition retiree ouvrirait W^X a un
    autre programme : on les verifie une par une.
    """
    bloc = corps(wx, "pub const fn exception_jit_rwx(")
    if bloc is None:
        fautes.append("wx.rs : exception_jit_rwx introuvable.")
    else:
        for condition in ("prot_viole_wx(", "anonyme", "prive", "capacite_jit_rwx"):
            if condition not in bloc:
                fautes.append(
                    "wx.rs : l'exception RWX ne verifie plus « %s »." % condition
                )
        if "||" in bloc:
            fautes.append(
                "wx.rs : l'exception RWX contient une alternative (`||`) ; "
                "ses conditions doivent TOUTES tenir."
            )

    verifie_jit = corps(mem, "pub fn prot_to_flags_verifie_jit(")
    if verifie_jit is None:
        fautes.append("mem.rs : prot_to_flags_verifie_jit introuvable.")
    else:
        if "exception_jit_rwx(" not in verifie_jit:
            fautes.append("mem.rs : la barriere JIT n'applique plus l'exception bornee.")
        if "prot_to_flags_verifie(prot)" not in verifie_jit:
            fautes.append(
                "mem.rs : hors exception, la barriere JIT ne revient plus a la "
                "regle stricte `prot_to_flags_verifie`."
            )

    tout = re.search(r"pub const ALL: Self = Self\((.*?)\);", capacites, re.S)
    if tout is None:
        fautes.append("capability.rs : Capabilities::ALL introuvable.")
    elif "JIT_RWX" in tout.group(1):
        fautes.append(
            "capability.rs : JIT_RWX fait partie de ALL ; tout processus "
            "System pourrait creer des pages RWX."
        )

    table = corps(profil, "pub const fn capabilities(")
    if table is not None and "JIT_RWX" in table:
        fautes.append(
            "profile.rs : un profil porte JIT_RWX dans sa table ; il passerait "
            "par l'intersection des droits sans la verification du proprietaire."
        )
    eligible = corps(profil, "pub const fn eligible_jit_rwx(")
    if eligible is None:
        fautes.append("profile.rs : eligible_jit_rwx introuvable.")
    else:
        if "RuntimeJava" not in eligible or "!no_new_privs" not in eligible:
            fautes.append(
                "profile.rs : eligible_jit_rwx ne se limite plus au profil "
                "RuntimeJava hors no_new_privs."
            )
        profils = re.findall(r"SecurityProfile::(\w+)", eligible)
        if set(profils) != {"RuntimeJava"}:
            fautes.append(
                "profile.rs : eligible_jit_rwx nomme d'autres profils : %s"
                % ", ".join(sorted(set(profils)))
            )

    accorde = corps(politique, "fn accorde_jit_rwx(")
    if accorde is None:
        fautes.append("policy.rs : accorde_jit_rwx introuvable.")
    else:
        if "eligible_jit_rwx(" not in accorde or "image_java_sure" not in accorde:
            fautes.append(
                "policy.rs : accorde_jit_rwx n'exige plus le profil eligible ET "
                "une image verifiee (proprietaire root, non inscriptible)."
            )
        if accorde.count(".union(Capabilities::JIT_RWX)") != 1 or politique.count(
            ".union(Capabilities::JIT_RWX)"
        ) != 1:
            fautes.append(
                "policy.rs : JIT_RWX est accorde ailleurs que dans accorde_jit_rwx."
            )
    lanceur = corps(politique, "pub fn lanceur_java_de_confiance(")
    if lanceur is None:
        fautes.append("policy.rs : lanceur_java_de_confiance introuvable.")
    else:
        for condition in ("runtime_java_image(", "uid == 0", "0o022"):
            if condition not in lanceur:
                fautes.append(
                    "policy.rs : la verification du lanceur Java ne regarde plus "
                    "« %s »." % condition
                )

    controle = corps(memoire, "pub fn check_protection(")
    if controle is None or "exception_jit_rwx(" not in controle:
        fautes.append(
            "security/memory.rs : la premiere barriere n'applique plus "
            "l'exception bornee (elle doit etre la meme que la seconde)."
        )


def regle_preuves(test, fautes):
    for attendu in (
        "l_execution_est_le_defaut_sur_x86_64",
        "toutes_les_combinaisons_de_drapeaux_sont_couvertes",
        "une_demande_refusee_aurait_bien_produit_une_page_w_et_x",
        "un_segment_refuse_aurait_bien_produit_une_page_w_et_x",
        "la_sequence_ecrire_puis_executer_reste_possible",
        "l_exception_jit_exige_toutes_ses_conditions",
        "l_exception_jit_ne_concerne_que_w_et_x",
        "une_plage_avec_un_trou_n_est_pas_couverte",
    ):
        if attendu not in test:
            fautes.append("test_wx.rs : la preuve « %s » a disparu." % attendu)


def main():
    fautes = []
    for chemin in (WX, MEM, ELF, TEST, CAPACITES, PROFIL, POLITIQUE, MEMOIRE):
        if not chemin.exists():
            fautes.append("fichier absent : %s" % chemin.relative_to(RACINE).as_posix())
    if fautes:
        for faute in fautes:
            print("  - %s" % faute)
        return 1

    wx = sans_commentaires(WX.read_text(encoding="utf-8"))
    mem = sans_commentaires(MEM.read_text(encoding="utf-8"))
    elf = sans_commentaires(ELF.read_text(encoding="utf-8"))
    test = TEST.read_text(encoding="utf-8")
    capacites = sans_commentaires(CAPACITES.read_text(encoding="utf-8"))
    profil = sans_commentaires(PROFIL.read_text(encoding="utf-8"))
    politique = sans_commentaires(POLITIQUE.read_text(encoding="utf-8"))
    memoire = sans_commentaires(MEMOIRE.read_text(encoding="utf-8"))

    regle_la_regle_existe(wx, fautes)
    regle_les_trois_portes(mem, elf, fautes)
    regle_le_refus_est_visible(mem, fautes)
    regle_pas_de_correction_silencieuse(mem, fautes)
    regle_exception_jit_bornee(wx, mem, capacites, profil, politique, memoire, fautes)
    regle_preuves(test, fautes)

    if fautes:
        print("W^X : %d regle(s) violee(s)\n" % len(fautes))
        for faute in fautes:
            print("  - %s\n" % faute)
        return 1
    print(
        "W^X : les trois portes -- mmap, mprotect, chargement ELF -- refusent "
        "une page a la fois inscriptible et executable, et le refus se compte ; "
        "seule exception : memoire anonyme privee d'un lanceur Java root (JIT_RWX)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
