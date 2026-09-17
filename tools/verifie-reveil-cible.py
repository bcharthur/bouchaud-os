#!/usr/bin/env python3
"""Garde-fou : une tache prete ne doit pas rester sans coeur.

# Le defaut, mesure sur TRIGKEY le 17 septembre 2026

    hid_poll_gap_max_ms    = 6783
    hid_wake_to_run_max_us = 6782927
    hid_run_to_lock_max_us = 2
    hid_poll_body_max_us   = 162

Le fil `usb-hid` n'etait ni lent ni bloque : il n'etait pas ELU. Cinq
mecanismes se refermaient sur le meme predicat -- « la tache courante est-elle
une tache noyau ? » -- et le sixieme, le vol de travail, ne regardait pas la
bande ou il attendait :

1. `publish_ready` n'envoyait l'IPI que si le coeur cible DORMAIT ;
2. `reschedule_interrupt_handler` ne faisait RIEN quand l'IPI interrompait une
   tache noyau -- ni commutation, ni meme demande differee ;
3. `running_user_cpu_mask` excluait du balayage de quantum les coeurs occupes
   par une tache noyau ;
4. `preempt::safe_point` refuse de commuter une tache noyau, et aucun fil noyau
   n'atteint de point sur de toute facon ;
5. la pression volable ne comptait que la bande NORMALE : une interactive seule
   en attente ne rendait jamais son coeur candidat au delestage.

# Ce qui est verifie ici

1. `reveil.rs` reste PUR : un test hote doit pouvoir contredire la politique
   sans demarrer le systeme.
2. Le scheduler ne connait AUCUN nom de tache. Le privilege se demande par la
   propriete `latency_sensitive`, jamais par une comparaison de chaine.
3. `publish_ready` decide APRES la barriere. Le choix du coeur peut se faire
   sur un etat perime -- il coute au pire une election ; la decision d'envoyer
   l'IPI, non : c'est la moitie du motif croise qui ferme le reveil perdu.
4. Le budget est LU ET REMIS A ZERO au meme endroit. Le lire sans le rendre
   ferait grandir la somme pour toujours et desarmerait le privilege apres
   quelques secondes de fonctionnement normal.
5. Les deux gestionnaires d'IRQ servent la demande ciblee quand la tache
   interrompue est une tache noyau. Sans eux la demande n'a aucun destinataire.
6. La demande ciblee n'est PAS rendue quand la preemption est refusee. La
   rendre rendrait la tache invisible jusqu'a son prochain reveil -- qui
   n'arrivera pas : elle attend d'etre elue pour se rendormir.
7. `preemption_noyau_sure` verifie les quatre conditions de contexte. En
   retirer une ouvrirait la commutation depuis une IRQ sous verrou.
8. Le balayage de quantum reprend les coeurs a demande pendante, faute de quoi
   un refus n'aurait jamais de seconde chance.
9. Le secours par vol regarde les DEUX bandes.
"""

import re
import sys
from pathlib import Path

RACINE = Path(__file__).resolve().parents[1]
REVEIL = RACINE / "src/kernel/scheduler/reveil.rs"
PREEMPT = RACINE / "src/kernel/scheduler/preempt.rs"
CREATION = RACINE / "src/kernel/process/thread/creation.rs"
COURANT = RACINE / "src/kernel/process/thread/courant.rs"
COMMUTATION = RACINE / "src/kernel/process/thread/commutation.rs"
COMPTABILITE = RACINE / "src/kernel/process/thread/comptabilite.rs"
RESCHEDULE = RACINE / "src/arch/x86_64/idt/reschedule.rs"
TIMER = RACINE / "src/arch/x86_64/idt/timer.rs"
PREEMPTION_IDT = RACINE / "src/arch/x86_64/idt/preemption.rs"
XHCI = RACINE / "src/drivers/usb/xhci_active.rs"


def code_seul(source):
    """Le code sans les commentaires.

    Une garde qui lit les commentaires se satisfait de sa propre prose : le
    nom qu'elle cherche figure dans l'explication du defaut qu'elle defend.
    """
    sans = re.sub(r"/\*.*?\*/", "", source, flags=re.S)
    return "\n".join(re.sub(r"//.*$", "", l) for l in sans.splitlines())


def corps(source, signature):
    """Le corps d'une fonction, jusqu'a la declaration suivante -- INDENTEE
    COMPRISE, sans quoi une regle sur une methode se satisfait d'un jeton
    trouve trois methodes plus bas."""
    d = source.find(signature)
    if d < 0:
        return None
    reste = source[d + len(signature):]
    fin = re.search(r"\n\s*(?:pub )?(?:const |static |fn |struct |impl |enum )", reste)
    return reste[: fin.start()] if fin else reste


def bloc(source, entete):
    """Le bloc COMPLET d'un `impl`, accolades comptees.

    `corps` coupe a la declaration suivante, ce qui est juste pour une
    fonction et faux pour un `impl` : la premiere methode indentee EST la
    declaration suivante, et la fenetre serait vide. Une regle posee sur un
    `impl` serait alors toujours violee, ou -- pire selon le sens du test --
    toujours satisfaite.
    """
    d = source.find(entete)
    if d < 0:
        return None
    ouvre = source.find("{", d)
    if ouvre < 0:
        return None
    profondeur = 0
    for i in range(ouvre, len(source)):
        if source[i] == "{":
            profondeur += 1
        elif source[i] == "}":
            profondeur -= 1
            if profondeur == 0:
                return source[ouvre : i + 1]
    return source[ouvre:]


def lit(chemin, fautes):
    try:
        return chemin.read_text(encoding="utf-8", errors="replace")
    except OSError:
        fautes.append("%s est illisible." % chemin.name)
        return None


def main():
    fautes = []

    # ------------------------------------------------------------------ 1
    reveil = lit(REVEIL, fautes)
    if reveil is not None:
        pur = code_seul(reveil)
        if "crate::" in pur:
            fautes.append(
                "reveil.rs n'est plus pur : il touche au reste du noyau et ne "
                "peut plus etre compile seul par un test hote. La politique "
                "redeviendrait inverifiable autrement qu'en demarrant la "
                "machine -- c'est ce qui a laisse le defaut vivre."
            )
        if "unsafe" in pur:
            fautes.append("reveil.rs contient de l'unsafe : une politique n'en a pas besoin.")
        privilege = corps(pur, "pub fn privilegiee(&self) -> bool {")
        if privilege is None:
            fautes.append("reveil.rs : `privilegiee` a disparu.")
        else:
            if "self.sensible" not in privilege:
                fautes.append(
                    "reveil.rs : le privilege ne depend plus de la propriete "
                    "declaree."
                )
            if "BUDGET_ACTIVATION_NS" not in privilege:
                fautes.append(
                    "reveil.rs : le privilege n'est plus borne par un budget. "
                    "Il deviendrait une priorite permanente, et une tache "
                    "sensible qui se met a calculer couperait le systeme a "
                    "chaque reveil."
                )
        decide = corps(pur, "pub fn decide(cible: &Coeur, reveille: &Reveille) -> Decision {")
        if decide is None:
            fautes.append("reveil.rs : `decide` a disparu.")
        elif "RESIDENCE_MINIMALE_NS" not in decide:
            fautes.append(
                "reveil.rs : un pair sensible n'a plus de tranche minimale. "
                "Deux fils sensibles se couperaient mutuellement a chaque "
                "publication et passeraient leur budget en commutations."
            )

    # ------------------------------------------------------------------ 2
    for chemin in (REVEIL, PREEMPT, CREATION):
        source = lit(chemin, fautes)
        if source is not None and "usb-hid" in code_seul(source):
            fautes.append(
                "%s nomme une tache. L'ordonnanceur doit servir une PROPRIETE "
                "-- `latency_sensitive` --, pas un nom : l'audio, le "
                "compositeur et l'entree la demanderont de la meme facon."
                % chemin.name
            )

    xhci = lit(XHCI, fautes)
    if xhci is not None:
        demarre = corps(code_seul(xhci), "pub fn demarre_le_fil_hid() -> bool {")
        if demarre is None or "spawn_noyau_sensible" not in demarre:
            fautes.append(
                "xhci_active.rs : le fil HID ne declare plus `latency_sensitive`. "
                "Il etait DEJA `Interactive` quand il a attendu 6,78 s : la "
                "classe ordonne la file, la propriete ordonne le REVEIL."
            )

    # ------------------------------------------------------------- 3, 4, 9
    creation = lit(CREATION, fautes)
    if creation is not None:
        pur = code_seul(creation)
        publie = corps(pur, "fn publish_ready(index: usize) {")
        if publie is None:
            fautes.append("creation.rs : `publish_ready` est introuvable.")
        else:
            barriere = publie.find("fence(Ordering::SeqCst)")
            # L'ANCRE EST LA LECTURE, PAS L'APPEL QUI LA CONSOMME.
            #
            # Deplacer `decide` ne prouve rien : ce qui doit suivre la
            # barriere, c'est la LECTURE de l'etat du coeur cible -- `is_idle`
            # en fait partie --, et non la fonction qui en tire une
            # conclusion. Une premiere version de cette garde s'ancrait sur
            # l'appel, et une mutation qui remontait la seule lecture passait.
            decision = publie.find("etat_coeur_reveil(target")
            if barriere < 0:
                fautes.append(
                    "creation.rs : la barriere du motif croise a disparu de "
                    "`publish_ready`. Sans elle le reveilleur lit « pas idle » "
                    "pendant que le coeur s'endort, et personne n'envoie l'IPI."
                )
            elif decision < 0 or "reveil::decide" not in publie:
                fautes.append(
                    "creation.rs : `publish_ready` ne consulte plus la politique "
                    "de reveil."
                )
            elif decision < barriere:
                fautes.append(
                    "creation.rs : l'etat du coeur cible est lu AVANT la "
                    "barriere. Le choix du coeur, lui, peut lire un etat perime "
                    "-- il coute au pire une election ; cette lecture-ci, non : "
                    "elle est la moitie du motif croise, et la faire trop tot "
                    "perd le reveil."
                )
            if "budget_reveil_ns.echange(0)" not in publie:
                fautes.append(
                    "creation.rs : le budget d'activation n'est plus lu ET "
                    "remis a zero au meme endroit. Une somme jamais rendue "
                    "depasse la borne apres quelques secondes et desarme le "
                    "privilege d'un fil parfaitement sage."
                )
            if "demande_ciblee" not in publie:
                fautes.append(
                    "creation.rs : `publish_ready` ne pose plus de demande "
                    "ciblee. C'est la seule chose qui ouvre la preemption d'un "
                    "fil noyau, et donc la seule qui sert une tache posee sur "
                    "un coeur occupe."
                )
        pression = corps(pur, "fn pression_volable(cpu_id: usize) -> usize {")
        if pression is None or "pression_de_secours" not in pression:
            fautes.append(
                "creation.rs : le secours par vol ne compte plus que le fond de "
                "file. Une interactive seule en attente derriere un fil noyau "
                "ne rendrait plus son coeur candidat, et aucun coeur au repos "
                "ne viendrait la chercher."
            )

    # ------------------------------------------------------------------ 5
    for chemin, signature, nom in (
        (RESCHEDULE, "extern \"x86-interrupt\" fn reschedule_interrupt_handler(", "l'IPI de replanification"),
        (TIMER, "extern \"x86-interrupt\" fn timer_interrupt_handler(", "le balayage de quantum"),
    ):
        source = lit(chemin, fautes)
        if source is None:
            continue
        handler = corps(code_seul(source), signature)
        if handler is None:
            fautes.append("%s : le gestionnaire est introuvable." % chemin.name)
            continue
        if "accorde_preemption_noyau()" not in handler:
            fautes.append(
                "%s : %s ne sert plus la demande ciblee quand la tache "
                "interrompue est une tache noyau. La demande n'aurait alors "
                "aucun destinataire -- un fil noyau n'atteint aucun point sur."
                % (chemin.name, nom)
            )

    idt = lit(PREEMPTION_IDT, fautes)
    if idt is not None:
        dispatch = corps(code_seul(idt), "fn dispatch_irq_preempt(source: u8, force: bool) {")
        if dispatch is None:
            fautes.append("preemption.rs : `dispatch_irq_preempt` a change de signature.")
        elif "&& !force" not in dispatch:
            fautes.append(
                "preemption.rs : le report diagnostic du BSP reprend les "
                "demandes ciblees. Un fil noyau sur le coeur zero n'atteint "
                "aucun point sur : il ne cederait jamais."
            )

    # --------------------------------------------------------------- 6, 7, 8
    preempt = lit(PREEMPT, fautes)
    if preempt is not None:
        pur = code_seul(preempt)
        accorde = corps(pur, "pub fn accorde_preemption_noyau() -> bool {")
        if accorde is None:
            fautes.append("preempt.rs : `accorde_preemption_noyau` est introuvable.")
        else:
            if "CIBLEE[" in accorde:
                fautes.append(
                    "preempt.rs : `accorde_preemption_noyau` touche au drapeau. "
                    "Le rendre au REFUS rendrait la tache invisible jusqu'a son "
                    "prochain reveil -- qui n'arrivera pas : elle attend d'etre "
                    "elue pour se rendormir."
                )
            if "preemption_noyau_sure()" not in accorde:
                fautes.append(
                    "preempt.rs : la preemption noyau est accordee sans "
                    "verifier le contexte."
                )
        sure = corps(pur, "pub fn preemption_noyau_sure() -> bool {")
        if sure is None:
            fautes.append("preempt.rs : `preemption_noyau_sure` est introuvable.")
        else:
            for jeton, pourquoi in (
                ("preempt_count()", "quelqu'un a demande a ne pas etre commute"),
                ("verrous_simples()", "un porteur de verrou tournant simple serait coupe, et la tache entrante tournerait sur ce verrou sur ce meme coeur"),
                ("lockdep::depth()", "une section critique rangee resterait ouverte sur un autre coeur"),
                ("profondeur_locale()", "le gros verrou serait tenu par une pile suspendue"),
                ("held_by_current_cpu()", "le gros verrou appartient a ce coeur"),
            ):
                if jeton not in sure:
                    fautes.append(
                        "preempt.rs : `preemption_noyau_sure` ne verifie plus "
                        "`%s` -- %s." % (jeton, pourquoi)
                    )
        masque = corps(pur, "pub fn masque_cible() -> u64 {")
        if masque is None:
            fautes.append("preempt.rs : `masque_cible` est introuvable.")

    spinlock = lit(RACINE / "src/kernel/sync/spinlock.rs", fautes)
    if spinlock is not None:
        pur = code_seul(spinlock)
        prise = corps(pur, "pub fn lock(&self) -> SpinLockGuard<'_, T> {")
        if prise is None:
            fautes.append("spinlock.rs : `SpinLock::lock` est introuvable.")
        else:
            marque = prise.find("marque_verrou_simple_pris(cpu)")
            acquisition = prise.find("compare_exchange_weak")
            if marque < 0:
                fautes.append(
                    "spinlock.rs : un porteur de verrou tournant simple ne se "
                    "declare plus. La preemption ciblee pourrait le couper, et "
                    "la tache entrante tournerait sur ce verrou, sur ce coeur, "
                    "pendant que la sortante attend un coeur pour le rendre."
                )
            elif acquisition >= 0 and marque > acquisition:
                fautes.append(
                    "spinlock.rs : la marque est posee APRES la prise. La "
                    "fenetre entre les deux est d'une instruction -- assez "
                    "pour l'interruption contre laquelle elle protege."
                )
        rendu = bloc(pur, "impl<T: ?Sized> Drop for SpinLockGuard<'_, T>")
        if rendu is None or "marque_verrou_simple_rendu" not in rendu:
            fautes.append(
                "spinlock.rs : la marque n'est plus rendue au relachement. Le "
                "coeur resterait impreemptible pour toujours."
            )
        elif "owner_cpu.swap(" not in rendu:
            fautes.append(
                "spinlock.rs : la marque est rendue au coeur COURANT et non a "
                "celui qui l'a posee. Une tache qui aurait migre entre les "
                "deux laisserait un coeur marque pour toujours."
            )

    courant = lit(COURANT, fautes)
    if courant is not None:
        mask = corps(code_seul(courant), "pub fn running_user_cpu_mask() -> u64 {")
        if mask is None:
            fautes.append("courant.rs : `running_user_cpu_mask` est introuvable.")
        elif "masque_cible()" not in mask:
            fautes.append(
                "courant.rs : le balayage de quantum exclut de nouveau TOUS les "
                "coeurs occupes par un fil noyau. Une preemption refusee une "
                "fois -- verrou tenu au moment de l'IPI -- n'aurait plus jamais "
                "de seconde chance."
            )

    commutation = lit(COMMUTATION, fautes)
    if commutation is not None:
        final = corps(code_seul(commutation), "fn finalise_task_running(task: &mut Task, cpu_id: usize) {")
        if final is None:
            fautes.append("commutation.rs : `finalise_task_running` est introuvable.")
        elif "rend_demande_ciblee()" not in final:
            fautes.append(
                "commutation.rs : la demande ciblee n'est plus rendue quand la "
                "commutation a eu lieu. Elle resterait pendante et ferait "
                "couper un occupant a chaque quantum, pour rien."
            )
        if "set_current_profil(" not in code_seul(commutation):
            fautes.append(
                "commutation.rs : le profil de l'occupant n'est plus publie. "
                "`publish_ready` devrait alors dereferencer la tache d'un autre "
                "coeur pendant qu'elle commute."
            )

    compta = lit(COMPTABILITE, fautes)
    if compta is not None and "budget_reveil_ns" not in code_seul(compta):
        fautes.append(
            "comptabilite.rs : le budget d'activation n'est plus accumule. Il "
            "resterait nul, et le privilege ne se desarmerait jamais."
        )

    if fautes:
        for f in fautes:
            print("FAUTE: %s" % f)
        return 1
    print(
        "reveil cible : politique pure et sans nom de tache, decision apres la "
        "barriere, budget lu et rendu, demande ciblee servie par les deux IRQ "
        "et jamais perdue au refus, secours par vol sur les deux bandes."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
