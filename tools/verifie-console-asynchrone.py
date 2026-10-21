#!/usr/bin/env python3
"""BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : la ligne attend dans un anneau, pas le port.

Une ligne s'emettait interruptions masquees, jeton tenu, apres la vidange des
lignes differees de tous les coeurs : `serie_pire_us` 15 a 20 ms par bras de
l'A/B KVM SMP4 (run 39503091750). Desormais l'appelant range sa ligne dans un
anneau (copie memoire, interruptions masquees le temps de la copie) et le fil
`console` l'emet sous le jeton, interruptions OUVERTES, par morceaux de 16
octets preemption coupee, un point cooperatif entre deux morceaux.

Le garde exige :
  1. `emet` : l'anneau seulement si la console est active, sinon (ou en
     reentrance) `sous_jeton` -- jamais d'ecriture directe sur le port ;
  2. `range_console` : interruptions masquees AVANT le verrou, verrou rendu
     AVANT de les rouvrir, reentrance du meme coeur refusee sans attendre,
     aucun acces au port (ni jeton, ni `ecris_octets`), publication par
     `range_morceaux` (entiere ou absente) ; BOUCHAUD_CONSOLE_VERROU_BORNE_V1 :
     l'attente du verrou, interruptions masquees, est BORNEE
     (`CONSOLE_VERROU_BORNE_NS` <= 5 ms, puis `RefusConsole::Occupe`) -- un
     detenteur suspendu (vCPU retire, coeur arrete) ne fait plus tourner les
     autres coeurs sans fin -- et la ligne refusee va dans l'anneau de son
     coeur (`differe`), jamais sur le port ; un ecrivain qui PEUT CEDER
     attend interruptions OUVERTES en cedant (`yield_now`, borne
     `CONSOLE_VERROU_BORNE_CEDEE_NS` <= 120 s : finie, et au-dela d'une
     suspension de 12 s) -- seul l'essai du verrou est
     masque ;
  3. `Anneau::range_morceaux` ne publie que si exactement `total` octets ont
     ete poses ;
  4. `vide_un_morceau` : preemption coupee avant le jeton, jeton essaye un
     nombre BORNE de tours, jamais d'interruptions masquees,
     `VIDANGE_EN_COURS` publie (SeqCst) avant de relire `EN_PANIQUE`, jeton
     rendu avant la preemption ; UN morceau de `CONSOLE_MORCEAU` <= 64 octets
     par passage. L'enregistrement de tete est REPRENABLE (`CONSOLE_ENTAME`) :
     le jeton n'est jamais tenu par une tache sortie du processeur (SMP1 : le
     contexte de demarrage, qui ne peut pas ceder, restait bloque derriere le
     fil preempte), et tout detenteur synchrone -- `sous_jeton`, le banc du
     detenteur suspendu -- termine l'enregistrement entame AVANT tout autre
     octet : la ligne reste entiere ;
  5. le fil : point cooperatif entre deux lignes, sommeil BORNE
     (`CONSOLE_SOMME_NS` <= 100 ms), barriere SeqCst entre « je dors » et la
     relecture de l'anneau ; le reveil : barriere SeqCst avant de lire « il
     dort », `wake_one` seulement si `cession_volontaire_sure()` ;
  6. anneau plein : le PRODUCTEUR AIDE A VIDER -- les lignes les plus
     anciennes sous le jeton, une a la fois (par morceaux avec points
     cooperatifs s'il peut ceder), puis la sienne dans l'anneau -- et ne se
     replie dans l'anneau de son coeur que si le jeton est tenu par son propre
     coeur sans pouvoir ceder, ou introuvable a l'echeance (banc
     run_console_panique.sh : 190 lignes sur 600 perdues sans cela) ; une
     ligne differee porte son repere (BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V1 :
     position d'ecriture de l'anneau) et ne sort que lorsque la lecture de
     l'anneau l'a atteint -- ni avant, ni apres --, jamais au milieu d'un
     enregistrement entame, le chemin synchrone et la panique fusionnant de
     meme (rafale de 2 000 lignes, TCG SMP4 : la 1 615 avant la 1 282 quand
     elle passait « une fois tous les 32 morceaux ») ; une attente
     de l'anneau vide d'un contexte qui ne peut pas ceder emet elle-meme
     (SMP1 : 5 s d'attente a vide et repli) ;
  7. fil muet : repli synchrone au-dela de `CONSOLE_FIL_MUET_NS` <= 5 s ;
  8. panique : `EN_PANIQUE` (SeqCst) PUIS lecture de `VIDANGE_EN_COURS`,
     attente bornee, puis le jeton PRIS ET GARDE (20 ms au plus) avant de
     vider l'anneau -- aucun autre coeur n'ecrit plus (SMP4 : `NET_BOOT` mele
     a la vidange sans cela) ; double faute : la meme
     bascule avant d'arreter les autres coeurs ; arret definitif hors panique :
     les lignes en attente sortent sous le jeton ;
  9. extinction et redemarrage : console arretee (anneau vide, synchrone) en
     PREMIER ;
 10. le detenteur synchrone ne vide l'anneau que console inactive, et
     jamais pendant une panique (seul le coeur qui panique le lit : SMP4,
     22 doublons et 285 lignes abimees sans cela) -- mais un detenteur
     legitime du jeton finit TOUJOURS l'enregistrement entame, panique ou non
     (sa ligne s'ecrivait au milieu sinon) ;
 11. le fil demarre (main.rs, stage2) ; l'autorun attend l'anneau vide avant
     le bilan et publie `BOUCHAUD_CONSOLE_BILAN` ; `console-async` existe.
 12. BOUCHAUD_CONSOLE_CONTRE_PRESSION_V1 -- anneau plein : un producteur qui
     PEUT CEDER fait la queue (`attend_place`, ticket de `file_place`) au
     lieu d'aider puis de renoncer a une echeance (KVM SMP4 : 135 lignes
     perdues ; TCG, port a 5 us/octet : 323) : aucune echeance dans
     l'attente -- elle ne sort que si la console devient synchrone ou sur
     verrou suspendu --, toute sortie rend son ticket et reveille les
     suivants, les non-tetes dorment en attente NON interruptible (une tache
     condamnee meurt au parking sans derouler sa pile : son ticket bloquerait
     la file), une tete figee est sautee ; un producteur qui peut ceder ne
     double pas ceux qui attendent et laisse la RESERVE d'urgence
     (`range_console(.., CONSOLE_RESERVE_URGENCE)`, verifiee dans
     `range_console` par `file_place::tient`) aux contextes qui ne peuvent pas
     attendre, seuls a passer par `aide_a_vider` (reserve comprise).

Fail-closed ; chaque regle a son test negatif (mutation qui doit rougir).
"""
import re
import sys
import tempfile
from pathlib import Path

RACINE = Path(__file__).resolve().parent.parent
UART = "src/drivers/serial/uart16550.rs"
ANNEAU = "src/drivers/serial/differees.rs"
FILE_PLACE = "src/drivers/serial/file_place.rs"
EXC = "src/arch/x86_64/idt/exceptions.rs"
POWER = "src/kernel/power.rs"
MAIN = "src/main.rs"
STAGE2 = "src/platform/pc/stage2.rs"
AUTORUN = "src/kernel/autorun.rs"
SHELL = "src/shell/mod.rs"
FICHIERS = (UART, ANNEAU, FILE_PLACE, EXC, POWER, MAIN, STAGE2, AUTORUN, SHELL)


def sans_commentaires(t: str) -> str:
    return "\n".join(l.split("//", 1)[0] for l in t.splitlines())


def corps(texte: str, nom: str) -> str:
    m = re.search(r"\bfn\s+" + re.escape(nom) + r"\s*(?:<[^>{]*>)?\s*\(", texte)
    if not m:
        return ""
    o = texte.find("{", m.end())
    p = 0
    for i in range(o, len(texte)):
        p += texte[i] == "{"
        p -= texte[i] == "}"
        if p == 0:
            return texte[o:i + 1]
    return ""


def pos(texte: str, motif: str, rang: int = 1) -> int:
    i = -1
    for _ in range(rang):
        i = texte.find(motif, i + 1)
        if i < 0:
            return 10**9
    return i


def constante(texte: str, nom: str):
    m = re.search(r"const " + nom + r": (?:u64|usize) = ([0-9_]+);", texte)
    return int(m.group(1).replace("_", "")) if m else None


def verifie(racine: Path) -> list[str]:
    try:
        src = {f: sans_commentaires((racine / f).read_text(encoding="utf-8")) for f in FICHIERS}
    except OSError as e:
        return [f"lecture impossible : {e}"]
    f = []
    u = src[UART]
    # 1. emet
    e = corps(u, "emet")
    if not e or pos(e, "console_active()") > pos(e, "range_console(octets, ") \
            or not re.search(r"\n    sous_jeton\(octets\);\n\}$", e) \
            or re.search(r"ecris_octets|write_lot|EMISSION\b", e):
        f.append(f"{UART} : `emet` contourne la console ou ecrit sur le port")
    # 2. range_console
    r = corps(u, "range_console")
    cli = pos(r, "interrupts::disable();")
    cas = pos(r, "CONSOLE_VERROU.compare_exchange_weak(0, moi,")
    rend = pos(r, "CONSOLE_VERROU.store(0, Ordering::Release);")
    sti = r.rfind("interrupts::enable();")
    if not (cli < cas < rend < (sti if sti >= 0 else 10**9) < 10**9):
        f.append(f"{UART} : `range_console` prend le verrou interruptions ouvertes, ou les rouvre verrou tenu")
    if not re.search(r"Err\(tenu\) if tenu == moi => \{.{0,200}?return Err\(RefusConsole::Reentree\);", r, re.S):
        f.append(f"{UART} : la reentrance sur le verrou de la console attend au lieu d'etre refusee")
    if re.search(r"ecris_octets|write_lot|EMISSION\b|sous_jeton", r) or "CONSOLE.range_morceaux(total," not in r:
        f.append(f"{UART} : `range_console` touche le port, ou ne publie plus en un enregistrement")
    borne = re.search(r"const CONSOLE_VERROU_BORNE_NS: u64 = ([0-9_]+);", u)
    attente = r[pos(r, "loop {"):cas + 2000] if cas >= 0 else ""
    if not borne or int(borne.group(1).replace("_", "")) > 5_000_000 \
            or "monotonic_ns().saturating_sub(debut) > CONSOLE_VERROU_BORNE_NS" not in attente \
            or "return Err(RefusConsole::Occupe);" not in attente:
        f.append(f"{UART} : l'attente du verrou de la console, interruptions masquees, n'est plus bornee (<= 5 ms)")
    cede = re.search(r"const CONSOLE_VERROU_BORNE_CEDEE_NS: u64 = ([0-9_]+);", u)
    if not cede or int(cede.group(1).replace("_", "")) > 120_000_000_000 \
            or not re.search(r"if peut_ceder && ouvertes \{.{0,900}?x86_64::instructions::interrupts::enable\(\);\s*"
                             r"crate::kernel::task::yield_now\(\);\s*x86_64::instructions::interrupts::disable\(\);", r, re.S) \
            or "attente > CONSOLE_VERROU_BORNE_CEDEE_NS" not in r:
        f.append(f"{UART} : l'attente cedee du verrou de la console n'est plus interruptions ouvertes, ou plus bornee")
    # BOUCHAUD_CONSOLE_REPLI_COOPERATIF_V1 : en repli, l'anneau passe par
    # morceaux interruptions ouvertes avant le chemin synchrone, si l'on peut ceder.
    vc = corps(u, "vide_anneau_en_cedant")
    if pos(e, "cession_volontaire_sure()") > pos(e, "vide_anneau_en_cedant();") \
            or pos(e, "vide_anneau_en_cedant();") > e.rfind("sous_jeton(octets);") \
            or "vide_un_morceau()" not in vc or "echeance" not in vc or "interrupts::disable" in vc:
        f.append(f"{UART} : en repli, l'anneau n'est plus vide par morceaux interruptions ouvertes avant le chemin synchrone")
    # Chaque bras `Occupe` d'`emet` (ajout direct, attente de place) : la
    # ligne va dans l'anneau de son coeur, jamais sur le port.
    bras = [m.group(2) for m in re.finditer(r"(\n +)Err\(RefusConsole::Occupe\) => \{(.*?)\1\}", e, re.S)]
    if len(bras) < 2 or any("differe(octets)" not in b or "return;" not in b
                            or re.search(r"ecris_octets|sous_jeton|EMISSION\b", b) for b in bras):
        f.append(f"{UART} : une ligne refusee par le verrou (detenteur suspendu) ne va plus dans l'anneau de son coeur")
    # 3. range_morceaux
    rm = corps(src[ANNEAU], "range_morceaux")
    if not re.search(r"if pose != total \{\s*return false;\s*\}\s*self\.queue\.store\(", rm):
        f.append(f"{ANNEAU} : `range_morceaux` publie un enregistrement incomplet")
    # 4. vide_un_morceau
    v = corps(u, "vide_un_morceau")
    if not (pos(v, "preempt::disable();") < pos(v, "EMISSION.compare_exchange_weak(0, moi,") < 10**9):
        f.append(f"{UART} : le fil prend le jeton preemptible")
    if not re.search(r"if tours > 64 \{\s*crate::kernel::scheduler::preempt::enable\(\);\s*return Vidange::JetonOccupe;", v):
        f.append(f"{UART} : le fil attend le jeton sans borne (ou preemption coupee)")
    if "interrupts::disable" in v or "without_interrupts" in v:
        f.append(f"{UART} : le fil emet interruptions masquees")
    if not (pos(v, "VIDANGE_EN_COURS.store(moi, Ordering::SeqCst);") < pos(v, "EN_PANIQUE.load(Ordering::SeqCst)") < 10**9):
        f.append(f"{UART} : le fil relit la panique avant de se declarer lecteur (Dekker rompu)")
    fin = v.rfind("EMISSION.store(0, Ordering::Release);")
    if fin < 0 or v.rfind("preempt::enable();") < fin:
        f.append(f"{UART} : le fil rend la preemption avant le jeton")
    morceau = constante(u, "CONSOLE_MORCEAU")
    if "vide_console_entiere" in v or "vide_differees()" in v or "finis_entame" in v \
            or v.count("avance_tete(CONSOLE_MORCEAU, debut)") != 1 or "avance_tete(usize::MAX" in v \
            or len(re.findall(r"\bwhile\b", v)) != 1 or re.search(r"\bloop\b|\bfor\b", v) \
            or morceau is None or morceau > 64:
        f.append(f"{UART} : plus d'un morceau (<= 64 octets) par passage, preemption coupee")
    at = corps(u, "avance_tete")
    if not re.search(r"if a < n \{\s*CONSOLE_ENTAME\.store\(a, Ordering::Relaxed\);\s*return Some\(false\);", at) \
            or not (pos(at, "CONSOLE.retire(t, n);") < pos(at, "CONSOLE_ENTAME.store(0, Ordering::Relaxed);") < 10**9) \
            or "let de = deja.max(CONSOLE_HORODATAGE).min(n);" not in at:
        f.append(f"{UART} : l'enregistrement de tete n'est plus repris la ou il en etait (ligne coupee ou repetee)")
    sj = corps(u, "sous_jeton")
    banc = corps(u, "banc_detenteur_suspendu")
    if not re.search(r"Some\(true\) => \{.{0,900}?finis_entame\(\);\s*vide_differees\(\);", sj, re.S) \
            or not re.search(r"finis_entame\(\);\s*vide_differees\(\);\s*ecris_octets\(b\"SERIE_DETENTEUR_SUSPENDU debut", banc):
        f.append(f"{UART} : un detenteur synchrone ecrit au milieu d'une ligne entamee par la console")
    # 5. fil et reveil
    fil = corps(u, "fil_console")
    somme = constante(u, "CONSOLE_SOMME_NS")
    if "point_cooperatif(&mut depuis)" not in fil or "wait_until_noyau(ticket, borne)" not in fil \
            or somme is None or somme > 100_000_000:
        f.append(f"{UART} : le fil ne cede plus entre deux lignes, ou dort sans borne")
    if not (pos(fil, "CONSOLE_FIL_ENDORMI.store(true, Ordering::SeqCst);") < pos(fil, "fence(Ordering::SeqCst);")
            < pos(fil, "CONSOLE.est_vide()", 1 + fil[:pos(fil, "fence(Ordering::SeqCst);")].count("CONSOLE.est_vide()")) < 10**9):
        f.append(f"{UART} : le fil relit l'anneau sans barriere apres « je dors » (reveil perdu)")
    rv = corps(u, "reveille_vidange")
    if not (pos(rv, "fence(Ordering::SeqCst);") < pos(rv, "CONSOLE_FIL_ENDORMI.load(") < pos(rv, "cession_volontaire_sure()")
            < pos(rv, "CONSOLE_ATTENTE.wake_one();") < 10**9) or "DANS_REVEIL_CONSOLE" not in rv:
        f.append(f"{UART} : le reveil du fil part d'un contexte qui ne peut pas publier, ou sans barriere")
    # 6. anneau plein
    ac = corps(u, "attend_console_vide")
    if "vide_un_morceau()" not in ac or re.search(r"else \{\s*core::hint::spin_loop\(\);\s*\}\s*\}\s*\}$", ac):
        f.append(f"{UART} : l'attente de l'anneau vide tourne a vide dans un contexte qui ne peut pas ceder (famine du fil)")
    urgence = re.search(r"Err\(RefusConsole::Plein\) => \{\s*CONSOLE_PLEINES\.fetch_add\(1, Ordering::Relaxed\);\s*"
                        r"CONSOLE_URGENCES\.fetch_add\(1, Ordering::Relaxed\);(.*?)\n                \}", e, re.S)
    if not urgence or not (pos(urgence.group(1), "if aide_a_vider(octets) {") < pos(urgence.group(1), "differe(octets)") < 10**9):
        f.append(f"{UART} : anneau plein, l'appelant qui ne peut pas ceder differe au lieu d'aider a vider (lignes perdues)")
    av = corps(u, "aide_a_vider")
    if "vide_un_morceau()" not in av or "if emis && peut_ceder {\n            crate::kernel::task::point_cooperatif(&mut depuis);" not in av \
            or "if ouvertes { 1_000_000_000 } else { 20_000_000 }" not in av or "range_console(octets, 0)" not in av \
            or pos(av, "Vidange::JetonOccupe =>") > pos(av, "match range_console(octets, 0)") or re.search(r"\bcontinue\b", av) \
            or "EN_PANIQUE.load(Ordering::Acquire)" not in av \
            or not re.search(r"\} else if EMISSION\.load\(Ordering::Relaxed\) == crate::arch::x86_64::smp::cpu_index\(\) \+ 1 \{\s*(?:phase\(0\);\s*)?return false;", av):
        f.append(f"{UART} : l'aide a la vidange n'est plus bornee, ou attend un jeton tenu par son propre coeur")
    # BOUCHAUD_CONSOLE_ORDRE_DIFFEREES_V1 : une ligne differee porte son
    # repere (position d'ecriture de l'anneau), ne sort que lorsque la lecture
    # l'a atteint, la plus ancienne d'abord, avant la tete et jamais au milieu
    # d'un enregistrement entame ; le chemin synchrone fusionne de meme.
    dif = corps(u, "differe")
    mure = corps(u, "vide_differee_mure")
    entiere = corps(u, "vide_console_entiere")
    if not re.search(r"if !entame \{\s*fait = vide_differee_mure\(\);\s*\}\s*if !fait \{\s*fait = avance_tete\(CONSOLE_MORCEAU, debut\)\.is_some\(\);\s*\}", v) \
            or "let repere = (CONSOLE.ecrits() as u64).to_le_bytes();" not in dif \
            or not re.search(r"range_morceaux\(DIFFEREE_REPERE \+ octets\.len\(\), \|pose\| \{\s*pose\(&repere\);\s*pose\(octets\);", dif) \
            or "let lus = CONSOLE.lus();" not in mure \
            or "if repere <= lus && choix.map_or(true, |(_, _, _, r)| repere < r) {" not in mure \
            or "DIFFEREES[cpu].lis(t, DIFFEREE_REPERE.min(n), n, ecris_octets);" not in mure \
            or not re.search(r"if CONSOLE_ENTAME\.load\(Ordering::Relaxed\) == 0 && vide_differee_mure\(\) \{\s*continue;\s*\}", entiere) \
            or not re.search(r"fn vide_differees\(\) \{\s*while vide_differee_mure\(\) \{\}\s*\}", u) \
            or re.search(r"\.vide(_un)?\(ecris_octets\)", u):
        f.append(f"{UART} : les lignes differees passent devant l'anneau (ordre d'un ecrivain rompu)")
    # 7. fil muet
    muet = constante(u, "CONSOLE_FIL_MUET_NS")
    if muet is None or muet > 5_000_000_000 or not re.search(r"CONSOLE_REPLI_SECURITE\.(store\(true|swap\(true)", corps(u, "surveille_fil_console")) \
            or "surveille_fil_console();" not in e or "!CONSOLE_REPLI_SECURITE.load(" not in corps(u, "console_active"):
        f.append(f"{UART} : un fil muet n'est plus remplace par le chemin synchrone")
    # 8. panique, double faute, arret
    pp = corps(u, "passe_en_panique")
    if not (pos(pp, "EN_PANIQUE.store(true, Ordering::SeqCst);") < pos(pp, "VIDANGE_EN_COURS.load(Ordering::SeqCst)")
            < pos(pp, "vide_console_entiere();") < 10**9) or "saturating_add(20_000_000)" not in pp \
            or "monotonic_ns() > echeance" not in pp or "tours >= 50_000_000" not in pp \
            or "saturating_add(200_000_000)" not in pp \
            or not (pos(pp, "VIDANGE_EN_COURS.load(Ordering::SeqCst)") < pos(pp, "EMISSION.compare_exchange(0, moi,")
                    < pos(pp, "vide_console_entiere();") < 10**9) \
            or pp[pos(pp, "EMISSION.compare_exchange(0, moi,"):].count("EMISSION.store(0") != 0:
        f.append(f"{UART} : la panique vide l'anneau sans poignee de main bornee avec le fil")
    df = corps(src[EXC], "double_fault_handler")
    if not (pos(df, "prends_la_panique(cpu)") < pos(df, "crate::drivers::serial::passe_en_panique();")
            < pos(df, "smp::arrete_les_autres_cpu();") < 10**9):
        f.append(f"{EXC} : une double faute arrete les coeurs avant de vider la console")
    ad = corps(src[EXC], "arret_definitif")
    if pos(ad, "crate::drivers::serial::vide_console_avant_arret();") > pos(ad, "loop {"):
        f.append(f"{EXC} : un arret definitif laisse des lignes dans l'anneau")
    va = corps(u, "vide_console_avant_arret")
    if not re.search(r"^\{\s*if EN_PANIQUE\.load\(Ordering::SeqCst\) \|\| !is_ready\(\) \{\s*return;", va) \
            or "vide_un_morceau()" not in va:
        f.append(f"{UART} : l'arret definitif vide l'anneau pendant une panique (deux lecteurs) ou hors jeton")
    # 9. extinction
    for nom in ("shutdown_interne", "reboot"):
        c = corps(src[POWER], nom)
        if not re.search(r"^\{\s*crate::drivers::serial::arrete_console_asynchrone\(\);", c):
            f.append(f"{POWER} : `{nom}` ne vide pas la console en premier")
    ar = corps(u, "arrete_console_asynchrone")
    if not (pos(ar, "attend_console_vide(") < pos(ar, "CONSOLE_VOULUE.store(false") < pos(ar, "sous_jeton(&[]);") < 10**9):
        f.append(f"{UART} : l'arret de la console ne vide pas l'anneau sous le jeton")
    # 10. detenteur synchrone
    sj = corps(u, "sous_jeton")
    if not re.search(r"if !console_active\(\) && !EN_PANIQUE\.load\(Ordering::Acquire\) \{\s*vide_console_entiere\(\);\s*\}", sj) \
            or sj.count("vide_console_entiere") != 1 \
            or "EN_PANIQUE" in corps(u, "finis_entame"):
        f.append(f"{UART} : le detenteur synchrone vide l'anneau console active, ou en panique (second lecteur)")
    # 11. demarrage, autorun, shell
    for fic, appel in ((MAIN, "drivers::serial::demarre_console_asynchrone();"),
                       (STAGE2, "crate::drivers::serial::demarre_console_asynchrone();")):
        if appel not in src[fic]:
            f.append(f"{fic} : le fil `console` ne demarre pas")
    if not (pos(src[AUTORUN], '"BOUCHAUD_SERIE_BILAN') < pos(src[AUTORUN], "crate::drivers::serial::journalise_bilan_console();") < 10**9):
        f.append(f"{AUTORUN} : le bilan de la console n'est plus publie")
    vd = corps(u, "vidange_differees")
    if not (pos(vd, "attend_console_vide(") < pos(vd, "sous_jeton(&[]);") < 10**9):
        f.append(f"{UART} : l'autorun publie son bilan avant que le fil ait tout emis")
    if '"console-async" => console_async(' not in src[SHELL]:
        f.append(f"{SHELL} : `console-async` absent (A/B impossible)")
    # 12. contre-pression
    if not re.search(r"let cedable = x86_64::instructions::interrupts::are_enabled\(\)\s*"
                     r"&& crate::kernel::scheduler::preempt::cession_volontaire_sure\(\);\s*"
                     r"let essai = if !cedable \{\s*range_console\(octets, 0\)\s*"
                     r"\} else if CONSOLE_FILE_PLACE\.vide\(\) \{\s*range_console\(octets, CONSOLE_RESERVE_URGENCE\)\s*"
                     r"\} else \{\s*Err\(RefusConsole::Plein\)\s*\};", e):
        f.append(f"{UART} : un producteur qui peut ceder double la file, ou consomme la reserve d'urgence")
    cede = re.search(r"Err\(RefusConsole::Plein\) if cedable => \{(.*?)\n                \}\n                Err\(RefusConsole::Plein\) => \{", e, re.S)
    if not cede or "match attend_place(octets) {" not in cede.group(1) or "aide_a_vider" in cede.group(1) \
            or not re.search(r"Err\(RefusConsole::Plein\) => \{\}", cede.group(1)):
        f.append(f"{UART} : anneau plein, un producteur qui peut ceder n'attend pas sa place (echeance, puis ligne perdue)")
    ap = corps(u, "attend_place")
    corps_boucle = ap[pos(ap, "let issue = loop {"):pos(ap, "CONSOLE_FILE_PLACE.sort(ticket);")]
    if not ap or re.search(r"echeance|BORNE|> debut|elapsed", corps_boucle) \
            or len(re.findall(r"\bbreak\b", corps_boucle)) != 2 \
            or not re.search(r"if !console_active\(\) \{\s*CONSOLE_PLACE_SYNCHRONES\.fetch_add\(1, Ordering::Relaxed\);\s*break Err\(RefusConsole::Plein\);", corps_boucle) \
            or not re.search(r"match range_console\(octets, CONSOLE_RESERVE_URGENCE\) \{\s*Err\(RefusConsole::Plein\) => \{\}\s*autre => break autre,", corps_boucle):
        f.append(f"{UART} : l'attente de place a une echeance (ligne perdue sous saturation) ou une sortie sans contrepartie")
    if not (pos(ap, "let ticket = CONSOLE_FILE_PLACE.prend();") < pos(ap, "let issue = loop {")
            < pos(ap, "CONSOLE_FILE_PLACE.sort(ticket);") < pos(ap, "CONSOLE_PLACE_ATTENTE.wake_all();") < 10**9):
        f.append(f"{UART} : une sortie de la file de place ne rend pas son ticket (file bloquee) ou ne reveille pas les suivants")
    if "CONSOLE_PLACE_ATTENTE.wait_until_noyau(" not in corps_boucle or re.search(r"\.wait_until\(|\.wait\(", corps_boucle):
        f.append(f"{UART} : un attendant de la file dort en attente interruptible (tache condamnee : ticket jamais rendu)")
    if not re.search(r"CONSOLE_FILE_PLACE\.saute\(vu, ticket\)", corps_boucle):
        f.append(f"{UART} : une tete de file figee bloque les suivants sans fin")
    rg = corps(u, "range_console")
    if not re.search(r"let fait = super::file_place::tient\(CONSOLE_OCTETS - CONSOLE\.occupation\(\), total, reserve\)\s*&& CONSOLE\.range_morceaux\(", rg):
        f.append(f"{UART} : `range_console` ignore la reserve d'urgence")
    fp = src[FILE_PLACE]
    if "self.servi.fetch_max(ticket.saturating_add(1), Ordering::AcqRel);" not in fp \
            or "self.servi() >= ticket" not in fp or not re.search(r"vu < moi\s*&& self", fp):
        f.append(f"{FILE_PLACE} : la file de place peut reculer, bloquer un ticket depasse ou sauter au-dela d'elle-meme")
    return f


def mutation(fichier: str, a: str, b: str) -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        for fic in FICHIERS:
            d = Path(tmp) / fic
            d.parent.mkdir(parents=True, exist_ok=True)
            t = (RACINE / fic).read_text(encoding="utf-8")
            if fic == fichier:
                if a not in t:
                    print(f"  mutation introuvable dans {fic} : {a[:70]!r}")
                    return False
                t = t.replace(a, b, 1)
            d.write_text(t, encoding="utf-8")
        return bool(verifie(Path(tmp)))


NEGATIFS = [
    # 2/12 : verrou suspendu pendant l'attente de place : ligne ecrite sur le port
    (UART, "                        Err(RefusConsole::Occupe) => {\n                            x86_64::instructions::interrupts::without_interrupts(|| differe(octets));",
     "                        Err(RefusConsole::Occupe) => {\n                            sous_jeton(octets);"),
    # 12 : le producteur cedable aide puis renonce a l'echeance (avant)
    (UART, "                    match attend_place(octets) {", "                    match aide_a_vider(octets) {"),
    # 12 : il double la file
    (UART, "            } else if CONSOLE_FILE_PLACE.vide() {", "            } else if true {"),
    # 12 : il consomme la reserve d'urgence
    (UART, "            } else if CONSOLE_FILE_PLACE.vide() {\n                range_console(octets, CONSOLE_RESERVE_URGENCE)",
     "            } else if CONSOLE_FILE_PLACE.vide() {\n                range_console(octets, 0)"),
    # 12 : echeance dans l'attente de place
    (UART, "        surveille_fil_console();\n        if CONSOLE_FILE_PLACE.a_son_tour(ticket) {",
     "        surveille_fil_console();\n        if crate::kernel::timer::monotonic_ns() > debut + 1_000_000_000 {\n            break Err(RefusConsole::Plein);\n        }\n        if CONSOLE_FILE_PLACE.a_son_tour(ticket) {"),
    # 12 : ticket jamais rendu
    (UART, "    phase(0);\n    CONSOLE_FILE_PLACE.sort(ticket);", "    phase(0);"),
    # 12 : attente interruptible (tache condamnee : pile jamais deroulee)
    (UART, "CONSOLE_PLACE_ATTENTE.wait_until_noyau(t,", "CONSOLE_PLACE_ATTENTE.wait_until(t,"),
    # 12 : tete figee jamais sautee
    (UART, "            && CONSOLE_FILE_PLACE.saute(vu, ticket)", "            && false"),
    # 12 : reserve ignoree
    (UART, "    let fait = super::file_place::tient(CONSOLE_OCTETS - CONSOLE.occupation(), total, reserve)\n        && CONSOLE.range_morceaux(",
     "    let fait = CONSOLE.range_morceaux("),
    # 12 : file qui recule
    (FILE_PLACE, "self.servi.fetch_max(ticket.saturating_add(1), Ordering::AcqRel);", "self.servi.store(ticket.saturating_add(1), Ordering::Release);"),
    # 12 : l'urgence (non cedable) differe sans aider
    (UART, "                    CONSOLE_URGENCES.fetch_add(1, Ordering::Relaxed);\n                    if aide_a_vider(octets) {",
     "                    CONSOLE_URGENCES.fetch_add(1, Ordering::Relaxed);\n                    if false {"),
    # 1 : emet ecrit directement
    (UART, "    sous_jeton(octets);\n}\n\n/// Range `octets`", "    ecris_octets(octets);\n}\n\n/// Range `octets`"),
    # 2 : verrou pris interruptions ouvertes
    (UART, "    x86_64::instructions::interrupts::disable();\n    let debut = crate::kernel::timer::monotonic_ns();\n    let mut moi",
     "    let debut = crate::kernel::timer::monotonic_ns();\n    let mut moi"),
    # 2 : reentrance attendue
    (UART, "            Err(tenu) if tenu == moi => {", "            Err(tenu) if tenu == usize::MAX => {"),
    # 2 : le verrou rendu apres les interruptions
    (UART, "    CONSOLE_VERROU.store(0, Ordering::Release);\n    let duree", "    let duree"),
    # 2 : attente du verrou sans borne (detenteur suspendu : coeurs figes IF=0)
    (UART, "            || (tours % 64 == 63\n                && crate::kernel::timer::monotonic_ns().saturating_sub(debut) > CONSOLE_VERROU_BORNE_NS)",
     "            || (tours % 64 == 63 && false)"),
    # 2 : ceder interruptions masquees
    (UART, "            x86_64::instructions::interrupts::enable();\n            crate::kernel::task::yield_now();\n            x86_64::instructions::interrupts::disable();\n            // Elle a pu migrer",
     "            crate::kernel::task::yield_now();\n            x86_64::instructions::interrupts::disable();\n            // Elle a pu migrer"),
    # 2 : attente cedee sans borne
    (UART, "                || attente > CONSOLE_VERROU_BORNE_CEDEE_NS\n", "                || false\n"),
    # 2 : attente cedee bornee si loin qu'elle ne l'est plus (10 min)
    (UART, "const CONSOLE_VERROU_BORNE_CEDEE_NS: u64 = 60_000_000_000;", "const CONSOLE_VERROU_BORNE_CEDEE_NS: u64 = 600_000_000_000;"),
    # repli : l'anneau entier vide interruptions masquees par le chemin synchrone
    (UART, "    {\n        vide_anneau_en_cedant();\n    }\n    sous_jeton(octets);", "    {\n    }\n    sous_jeton(octets);"),
    # 2 : borne relevee a 50 ms
    (UART, "const CONSOLE_VERROU_BORNE_NS: u64 = 1_000_000;", "const CONSOLE_VERROU_BORNE_NS: u64 = 50_000_000;"),
    # 2 : ligne refusee par le verrou ecrite au lieu d'etre differee
    (UART, "                    x86_64::instructions::interrupts::without_interrupts(|| differe(octets));\n                    CONSOLE_DIFFEREES_OCCUPE.fetch_add(1, Ordering::Relaxed);\n                    CONSOLE_REPLIS_DIFFERES.fetch_add(1, Ordering::Relaxed);\n                    reveille_vidange();\n                    return;\n                }\n            }",
     "                    sous_jeton(octets);\n                    return;\n                }\n            }"),
    # 3 : enregistrement incomplet publie
    (ANNEAU, "        if pose != total {\n            return false;\n        }\n", ""),
    # 4 : l'enregistrement de tete n'est plus repris (repete depuis le debut)
    (UART, "    let de = deja.max(CONSOLE_HORODATAGE).min(n);", "    let de = CONSOLE_HORODATAGE.min(n);"),
    # 4 : le chemin synchrone ecrit au milieu d'une ligne entamee
    (UART, "            finis_entame();\n            vide_differees();", "            vide_differees();"),
    # 4 : une ligne differee au milieu d'un enregistrement entame
    (UART, "        if !entame {\n            fait = vide_differee_mure();\n        }", "        fait = vide_differee_mure();"),
    # 6 : une ligne differee sort avant que l'anneau ait atteint son repere
    (UART, "            if repere <= lus && choix.map_or(true, |(_, _, _, r)| repere < r) {",
     "            if choix.map_or(true, |(_, _, _, r)| repere < r) {"),
    # 6 : la plus recente d'abord
    (UART, "            if repere <= lus && choix.map_or(true, |(_, _, _, r)| repere < r) {",
     "            if repere <= lus && choix.map_or(true, |(_, _, _, r)| repere > r) {"),
    # 6 : ligne differee sans repere
    (UART, "    let repere = (CONSOLE.ecrits() as u64).to_le_bytes();", "    let repere = 0u64.to_le_bytes();"),
    # 6 : le repere emis avec la ligne
    (UART, "    DIFFEREES[cpu].lis(t, DIFFEREE_REPERE.min(n), n, ecris_octets);", "    DIFFEREES[cpu].lis(t, 0, n, ecris_octets);"),
    # 6 : chemin synchrone : l'anneau entier avant les differees
    (UART, "        if CONSOLE_ENTAME.load(Ordering::Relaxed) == 0 && vide_differee_mure() {\n            continue;\n        }\n", ""),
    # 6 : toutes les differees d'un coup, mures ou non
    (UART, "fn vide_differees() {\n    while vide_differee_mure() {}\n}", "fn vide_differees() {\n    for anneau in DIFFEREES.iter() {\n        anneau.vide(ecris_octets);\n    }\n}"),
    # 6 : la differee passe apres la tete (des lignes plus recentes la doublent)
    (UART, "        if !entame {\n            fait = vide_differee_mure();\n        }\n        if !fait {\n            fait = avance_tete(CONSOLE_MORCEAU, debut).is_some();\n        }",
     "        fait = avance_tete(CONSOLE_MORCEAU, debut).is_some();\n        if !fait && !entame {\n            fait = vide_differee_mure();\n        }"),
    # 4 : jeton attendu sans borne
    (UART, "        if tours > 64 {\n            crate::kernel::scheduler::preempt::enable();\n            return Vidange::JetonOccupe;\n        }\n", ""),
    # 4 : emission interruptions masquees
    (UART, "    VIDANGE_EN_COURS.store(moi, Ordering::SeqCst);\n    let mut fait = false;",
     "    VIDANGE_EN_COURS.store(moi, Ordering::SeqCst);\n    x86_64::instructions::interrupts::disable();\n    let mut fait = false;"),
    # 4 : Dekker rompu
    (UART, "    VIDANGE_EN_COURS.store(moi, Ordering::SeqCst);\n    let mut fait = false;\n    if !EN_PANIQUE.load(Ordering::SeqCst) {",
     "    let mut fait = false;\n    if !EN_PANIQUE.load(Ordering::SeqCst) {\n        VIDANGE_EN_COURS.store(moi, Ordering::SeqCst);"),
    # 4 : une ligne entiere preemption coupee
    (UART, "const CONSOLE_MORCEAU: usize = 16;", "const CONSOLE_MORCEAU: usize = 4096;"),
    # 4 : un enregistrement entier par passage, preemption coupee
    (UART, "            fait = avance_tete(CONSOLE_MORCEAU, debut).is_some();", "            fait = avance_tete(usize::MAX, debut).is_some();"),
    # 5 : sommeil sans borne
    (UART, "const CONSOLE_SOMME_NS: u64 = 50_000_000;", "const CONSOLE_SOMME_NS: u64 = 5_000_000_000;"),
    # 5 : barriere retiree (reveil perdu)
    (UART, "        // Pendant de `reveille_vidange` : se declarer endormi, PUIS relire.\n        core::sync::atomic::fence(Ordering::SeqCst);\n", ""),
    # 5 : reveil depuis n'importe quel contexte
    (UART, "    if !crate::kernel::scheduler::preempt::cession_volontaire_sure()\n        || DANS_REVEIL_CONSOLE",
     "    if DANS_REVEIL_CONSOLE"),
    # 6 : attente a vide de l'anneau d'un contexte qui ne peut pas ceder
    (UART, "        match vide_un_morceau() {\n            Vidange::Emise | Vidange::Rien => {}",
     "        match Vidange::Rien {\n            Vidange::Emise | Vidange::Rien => {}"),
    # 6 : anneau plein, plus d'aide a la vidange
    (UART, "                    if aide_a_vider(octets) {\n", "                    if false {\n"),
    # 6 : aide qui attend un jeton tenu par son propre coeur
    (UART, "                } else if EMISSION.load(Ordering::Relaxed) == crate::arch::x86_64::smp::cpu_index() + 1 {\n                    phase(0);\n                    return false;",
     "                } else if false {\n                    phase(0);\n                    return false;"),
    # 6 : l'aide ne retente plus sa place quand le jeton est pris (repli, ordre rompu)
    (UART, "                core::hint::spin_loop();\n                }\n                false\n            }\n        };",
     "                core::hint::spin_loop();\n                }\n                continue;\n            }\n        };"),
    # 6 : retour a l'alternance anneau / differees (toute differee mure)
    (UART, "    let lus = CONSOLE.lus();", "    let lus = usize::MAX;"),
    # 7 : plus de repli si le fil est muet
    (UART, "        if !CONSOLE_REPLI_SECURITE.swap(true, Ordering::AcqRel) {", "        if false {"),
    # 8 : panique sans poignee de main
    (UART, "    EN_PANIQUE.store(true, Ordering::SeqCst);", "    EN_PANIQUE.store(true, Ordering::Relaxed);"),
    # 8 : la panique vide l'anneau sans prendre le jeton (un autre coeur ecrit en meme temps)
    (UART, "        match EMISSION.compare_exchange(0, moi, Ordering::Acquire, Ordering::Relaxed) {\n            Ok(_) => break,\n            Err(tenu) if tenu == moi => break,",
     "        match Ok::<usize, usize>(0) {\n            Ok(_) => break,\n            Err(tenu) if tenu == moi => break,"),
    # 8 : double faute sans vidange
    (EXC, "    crate::drivers::serial::passe_en_panique();\n    releve_faute_fatale(\"DOUBLE FAULT\"", "    releve_faute_fatale(\"DOUBLE FAULT\""),
    # 8 : arret definitif sans vidange
    (EXC, "    crate::drivers::serial::vide_console_avant_arret();\n", ""),
    # 8 : arret pendant une panique (deux lecteurs)
    (UART, "    if EN_PANIQUE.load(Ordering::SeqCst) || !is_ready() {\n        return;\n    }\n    let echeance",
     "    if !is_ready() {\n        return;\n    }\n    let echeance"),
    # 9 : extinction sans vidange
    (POWER, "fn shutdown_interne(code: u8) -> ! {\n    // BOUCHAUD_CONSOLE_ASYNCHRONE_V1 : les dernieres lignes avant de couper\n    // le courant ne dependent d'aucun fil.\n    crate::drivers::serial::arrete_console_asynchrone();\n",
     "fn shutdown_interne(code: u8) -> ! {\n"),
    (POWER, "pub fn reboot() -> ! {\n    crate::drivers::serial::arrete_console_asynchrone();\n", "pub fn reboot() -> ! {\n"),
    # 10 : detenteur synchrone qui vide l'anneau console active
    (UART, "            if !console_active() && !EN_PANIQUE.load(Ordering::Acquire) {\n                vide_console_entiere();\n            }\n", "            vide_console_entiere();\n"),
    # 10 : second lecteur de l'anneau pendant la panique
    (UART, "            if !console_active() && !EN_PANIQUE.load(Ordering::Acquire) {\n", "            if !console_active() {\n"),
    # 10 : un detenteur legitime abandonne l'enregistrement entame en panique (sa ligne au milieu)
    (UART, "    if CONSOLE_ENTAME.load(Ordering::Relaxed) != 0 {\n        let _ = avance_tete", "    if CONSOLE_ENTAME.load(Ordering::Relaxed) != 0 && !EN_PANIQUE.load(Ordering::Acquire) {\n        let _ = avance_tete"),
    # 11 : fil jamais demarre, bilan absent
    (MAIN, "    drivers::serial::demarre_console_asynchrone();\n", ""),
    (AUTORUN, "    crate::drivers::serial::journalise_bilan_console();\n", ""),
    (UART, "        attend_console_vide(5_000_000_000);\n        sous_jeton(&[]);", "        sous_jeton(&[]);"),
]


def main() -> int:
    fautes = verifie(RACINE)
    if fautes:
        print("console asynchrone : regle violee")
        print("\n".join("  " + x for x in fautes))
        return 1
    for n, (fic, a, b) in enumerate(NEGATIFS, 1):
        if not mutation(fic, a, b):
            print(f"console asynchrone : le test negatif {n} ne rougit pas -- garde inoperant")
            return 1
    print(f"CONSOLE_ASYNCHRONE_OK negatifs={len(NEGATIFS)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
