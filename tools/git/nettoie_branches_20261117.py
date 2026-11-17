#!/usr/bin/env python3
"""Grand nettoyage des branches distantes du 17/11/2026 -- verifie AVANT d'agir.

Inventaire etabli le 17/11 sur 57 branches distantes (main compris) :

  ancetre  14  la tete est un ancetre de `main` : tout y est deja ;
  squash   33  PR fusionnee par squash ; CHAQUE fichier touche par la branche a
               le MEME blob dans le commit de squash nomme ;
  archive   7  travail jamais fusionne (WIP, sauvegardes, PR fermees) : une
               etiquette annotee `archive/prune-20261117/<branche>` le garde,
               poussee et RELUE sur le depot distant avant toute suppression ;
  gardees   3  main, la branche de travail de la session, celle de #333 tant
               qu'elle est ouverte (elle n'est pas dans cette liste).

Rien n'est cru sur parole : chaque condition est re-verifiee au moment de
l'execution, sur le depot distant tel qu'il est. Une branche qui a bouge
depuis l'inventaire (SHA different) n'est PAS touchee. La suppression se fait
avec un bail sur le SHA attendu (`--force-with-lease`) : une branche mise a
jour entre la verification et la suppression n'est pas supprimee.

Vos branches LOCALES et votre stash ne sont jamais touches : seul le depot
distant l'est. Un `git fetch --prune` ulterieur retirera seulement les
references `origin/...` correspondantes.

Usage :
  python3 tools/git/nettoie_branches_20261117.py            # verification seule
  python3 tools/git/nettoie_branches_20261117.py --execute  # etiquettes, puis suppressions

Restaurer une branche archivee :
  git push origin archive/prune-20261117/<branche>^{commit}:refs/heads/<branche>
Restaurer une branche fusionnee : son contenu est dans `main` (commit de squash
nomme ci-dessous), et GitHub garde `refs/pull/<n>/head`.
"""
import subprocess
import sys

DISTANT = "origin"
PREFIXE = "archive/prune-20261117/"

# (branche, SHA attendu, categorie, preuve)
INVENTAIRE = [
    ("work/atelier-ladybird", "6bdcb36f3432d202d37b4562f43dee7363d0852f", "ancetre", ""),
    ("feat/p10-mainline-gpu-20261109", "100af4040ec42fae156f5aa4d6b8a745d7e1a164", "archive", "PR #281 fermee sans fusion ; P10 fusionne par #287"),
    ("feat/p10-gpu-preparation-20261109", "c188155af37a6648603c7b394391cbed27a2e82a", "archive", "PR #280 fermee sans fusion ; instantane WIP, historique distinct"),
    ("docs/p10-gpu-qualification-20261109", "35fdf7c891ce7dba6f2257229fbae657cd6fa3d7", "archive", "PR #279 fermee sans fusion"),
    ("claude/ci-selection-artefact-ladybird-20261110", "ca73b2fbe9938da37edb0656d71d94fa77a50611", "squash", "#284 77dacdb556ab982f8dbf67c03bce05af045fa8ba"),
    ("claude/c22-p13-p10-gpu-kernel-0y2ucg", "21e5730a8d188b35d52aba39476bcc10e1334c06", "archive", "sans PR ; session du 10/11 (compositord #286, P10 #287)"),
    ("claude/c22-p13-stabilisation-20261110", "b3e474d1d91074b503594ab1508a2df6d85f040e", "squash", "#285 376ed884b9f0662d8d0ce4f4fca9c6ffa66aee84"),
    ("claude/ci-ladybird-preuve-20261110", "33e81c7f68bedf19d11af42924c1874e3eab963d", "archive", "sans PR ; branche jetable de preuve CI (#282)"),
    ("claude/ci-ladybird-artefact-20261110", "d21b987dc596b617f6183f329e8c29dc5617071c", "squash", "#282 29316a6fe0bbc147b95669768cf9af5488e55aea"),
    ("claude/ci-fast-delai-kernel", "a918e684f48a43a4d7f104ef1670c939efde87e9", "squash", "#296 d8dfd9000fc9bf787efcf52ceffd4e09f492eabf"),
    ("sauvegarde/cle-usb-test", "6d445d4a067832d4900b3a621b3363a6d19c1164", "archive", "sauvegarde WIP non branchee (#299)"),
    ("claude/releve-fils-sysrq-283", "80dc75ee2c813b7e27bc4bb2035d8aa310f122c6", "squash", "#292 5b754bf0459a11e06eaca8038f09742067e96040"),
    ("claude/reveil-silence-observe", "e66f66d1d59bbdd4d66e955d9ff1edc44694fa85", "squash", "#294 bdc8973bcf1011922c504c0218191fd237d5671a"),
    ("sauvegarde/bureau-ring3-2b", "ab03170ad3d109e5af0c42415b05bf8f3f0d3203", "archive", "sauvegarde (#295, #298)"),
    ("claude/fix-banc-coupure-288", "40c21150a7a0752dab728b5051b1b1355ff93f4e", "squash", "#290 25a02b87fb539a60c2626b4b8bef2508f4015504"),
    ("claude/diag-ladybird-demarrage-283", "bf07973ad8a8fd5e97cfb2829f195e9316debaa8", "squash", "#291 63bb005ecf8c6de2226907617e47ed9207f2fb5e"),
    ("claude/docs-session-20261111", "8490c424fb2ab8d069f06c6d6b0137555ade6388", "squash", "#289 03475200675f1a6d220aaa887ecc469911b9f2f3"),
    ("claude/usb-test-disques-internes", "9b59a6f32524ac7f44cbb455d6404e8f84e76d86", "squash", "#299 3ac3b6356ed155f98b3a22b066c6dc5d5506cc4e"),
    ("claude/ci-apt-repli-miroir", "2934ac9b0b4b28e8a1bb5d564748eafc8410f0df", "squash", "#297 d2d47991882e305a915b3ae9dd02f417be964337"),
    ("claude/bureau-ring3-2a", "7c95282b93959062ffa401255c91b8ccf6d61ae6", "squash", "#295 ccd46014a458ac02896fabd8d36fa5261b69704d"),
    ("claude/bureau-ring3-2b", "f778e165a44a2f7f3e008e895919451abaacbbc4", "squash", "#298 f018244f5f24b9b820e63c760af2aa263ee4b219"),
    ("claude/compositord-tranche1-20261110", "e27c8256455bea6101c005e9a434164229b11eaf", "squash", "#286 d37ca9e04d40ae246b213400646d869949c979b4"),
    ("claude/p10-composition-gpu-20261110", "a56cfddaf6cda8a4a8a8f2cbb23a67ef31d4549b", "squash", "#287 063a98bec6652926db5329244e9adb49b4cdec57"),
    ("claude/unix-arborescence", "8d85b911755faba76f467c27b828473765df7e57", "ancetre", ""),
    ("claude/unix-audit-commandes", "3eb46d8c71959fa6ef84ed9a7dd7f086a32dbed6", "ancetre", ""),
    ("claude/unix-find-verrou", "4c3a0ad7623f979f725b545ada6d94ec99826af0", "ancetre", ""),
    ("claude/trigkey-livraison", "85cd1affaf63bcfa3867d616a03b8360a09f4c84", "ancetre", ""),
    ("claude/trigkey-ota", "6f44125a7027fab5b77da972910d94fd46d3ecc1", "ancetre", ""),
    ("claude/trigkey-state", "62f2b1213793b642ae335e60e4c6e329373a69ba", "ancetre", ""),
    ("claude/trigkey-release", "2f2aa8fe6977785771c75373cb712daef8142895", "ancetre", ""),
    ("claude/trigkey-anti-retour", "a6e7f1f75f4780170b7cab572b5f47da2d4e3c98", "ancetre", ""),
    ("claude/unix-persistance", "3a5abaa4e03e003a3b2b8daafed88783ed9f9596", "ancetre", ""),
    ("claude/ladybird-perf", "b0a354df1d20af875d817c5c97a0f1d12fb7b8b0", "ancetre", ""),
    ("claude/trigkey-ota-ring3", "2ef787d5a7ca1b7740d5e740903af14cbfba7a93", "ancetre", ""),
    ("claude/trigkey-ota-reseau", "31d95892a09d3f9753db34dfa308a502d1e56c49", "ancetre", ""),
    ("claude/passation-completion-if-coupe", "c2ca99568ab1f62b653e7b26d6625dee985949d0", "ancetre", ""),
    ("claude/doc-intermittents-317", "73c4179ef3813e8c4f85424ff4a057790d3b6232", "squash", "#318 c44a2e77f346669df76004409f573ed08c831c78"),
    ("claude/whpx-cpu", "3778452fd11437cf055309f176e88901123f410e", "squash", "#315 7a87dd4e0b254560e86101685e82b752c6a0a2b5"),
    ("claude/whpx-mesure", "68b7f88830c95a534feadf853eb7c760f24efa69", "squash", "#316 8f246db923df0d09b9c7f432d4cd165858476231"),
    ("claude/whpx-banc", "ffbac53841f0728e5f58b36fc5de5b567f77d9e7", "squash", "#317 4ad26dae6dae135ef53150e75e8b6e5b0bde2763"),
    ("claude/e2e-ladybird", "8aed50d6a9a096c4610720467170d151ea6f4f55", "squash", "#320 65a67d6b7483a1b6748b2e93f85e35f0706eb79b"),
    ("claude/souffrance-proprio", "b2597354bee03fb14fc37c0e037e763a1b6e998e", "squash", "#324 eb6f1155ca59d87001ebdd7473fa23b10752e39d"),
    ("claude/limites-reseau-hote", "49b0d5792f870ea6d18b4252db14cf43d4107ee8", "squash", "#323 01dc7fd107b1829e83a0952f01a31333d80db488"),
    ("claude/passation-325", "f027c35e120b187169a0b70eb642055e5f9edc0c", "squash", "#327 65f1e6582e4aaa3e7dcaaf6e938adb0bed8a0912"),
    ("claude/souffrance-site-detenteur", "adfdf8891b502f2a75ce6f5f2a87617ecfbcda8a", "squash", "#325 2eabd6525bdf3eeb3e548c69c7edd75aa9985a10"),
    ("claude/passation-14-soir", "cb6f74ef5eaf6de080b3c1cc8692e48ced4dbf2e", "squash", "#326 fab0c4e12a48e199ad8228e7dd023ca1c4c10695"),
    ("claude/python-hors-noyau", "efcf54ff121b8bbabd9313866865512c36a24c39", "squash", "#321 c6c3eea53df9498fdb93dcaaa9882e3005309a6b"),
    ("claude/passation-2026-11-14", "98846d4d37bcbde3bf7f590e86e7e39d04781b47", "squash", "#322 438b60db14c7053f1c43114bf4983fc6ba8d71e1"),
    ("claude/nightly-provenance", "dbbe3bc0517abbcca548834cb2d707ab1782fdfb", "squash", "#329 6d94bcf343e664769ba5546af9c6d1da6546def1"),
    ("claude/sigchld-boucle", "e9e0ce52f886abcf40e097a917bd74adaa93477b", "squash", "#319 02436431461f0a225c5fa26eb40fdf87a6a0ad19"),
    ("claude/souffrance-qemu-echantillons", "48972d5dab5921c0562f186780f657ddece26478", "squash", "#332 54713684159e4c26c431090b7c7926e973409c33"),
    ("claude/fuite-processus", "13a83b3ddbeffc6a96d26f63bd048f1cd7f62875", "squash", "#330 1f97b4b3e7780386609994ed00ef771fb96e493e"),
    ("claude/gel-283-nmi", "4dda0748a5bc522645f8a8cbe164e8bd2dc17d3d", "squash", "#328 504fdf1257967abd53b6e4362d6c52b896a7a580"),
    ("claude/fenetre-migration-283", "619da3c0c2c93462eeabd2f2bce9704e1af16214", "squash", "#331 427fa157f6bbf8feba1d5cb034eb6e5e7f726625"),
]


def git(*args, check=True):
    r = subprocess.run(["git", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} : {r.stderr.strip()}")
    return r.stdout.strip()


def tetes_distantes():
    sortie = git("ls-remote", "--heads", "--tags", DISTANT)
    refs = {}
    for ligne in sortie.splitlines():
        sha, ref = ligne.split("\t", 1)
        refs[ref] = sha
    return refs


def verifie(branche, sha, categorie, preuve, refs):
    """(ok, message) : la suppression est-elle prouvee sans perte ?"""
    distant = refs.get(f"refs/heads/{branche}")
    if distant is None:
        return None, "deja absente"
    if distant != sha:
        return False, f"a bouge ({distant[:8]} au lieu de {sha[:8]}) : non touchee"
    if categorie == "ancetre":
        r = subprocess.run(["git", "merge-base", "--is-ancestor", sha, f"{DISTANT}/main"])
        return (r.returncode == 0), "ancetre de main" if r.returncode == 0 else "N'EST PLUS ancetre de main"
    if categorie == "squash":
        pr, squash = preuve.split()
        r = subprocess.run(["git", "merge-base", "--is-ancestor", squash, f"{DISTANT}/main"])
        if r.returncode != 0:
            return False, f"commit de squash {squash[:8]} absent de main"
        base = git("merge-base", f"{DISTANT}/main", sha)
        fichiers = [f for f in git("diff", "--name-only", base, sha).splitlines() if f]
        for f in fichiers:
            a = git("rev-parse", f"{sha}:{f}", check=False)
            b = git("rev-parse", f"{squash}:{f}", check=False)
            if a != b:
                return False, f"{f} differe du squash {squash[:8]} ({pr})"
        return True, f"{len(fichiers)} fichiers identiques au squash {squash[:8]} ({pr})"
    if categorie == "archive":
        tag = refs.get(f"refs/tags/{PREFIXE}{branche}^{{}}")
        if tag == sha:
            return True, f"archivee ({PREFIXE}{branche})"
        return False, "etiquette d'archive absente du depot distant"
    return False, f"categorie inconnue {categorie}"


def archive(branche, sha, preuve):
    tag = PREFIXE + branche
    if git("tag", "-l", tag) == "":
        git("tag", "-a", tag, sha, "-m",
            f"Archive de la branche {branche} ({sha}), supprimee du depot distant (nettoyage du 17/11/2026).\n"
            f"Raison : {preuve}.\n"
            f"Restauration : git push origin {tag}^{{commit}}:refs/heads/{branche}")
    git("push", DISTANT, f"refs/tags/{tag}")


def main():
    executer = "--execute" in sys.argv[1:]
    git("fetch", "--quiet", DISTANT, "main")
    for b, sha, _c, _p in INVENTAIRE:
        git("fetch", "--quiet", DISTANT, sha, check=False)
    refs = tetes_distantes()

    if executer:
        for b, sha, c, p in INVENTAIRE:
            if c == "archive" and refs.get(f"refs/heads/{b}") == sha:
                archive(b, sha, p)
        refs = tetes_distantes()  # RELIRE : l'archive doit exister la-bas

    supprimees = refusees = absentes = 0
    for b, sha, c, p in INVENTAIRE:
        ok, message = verifie(b, sha, c, p, refs)
        if ok is None:
            absentes += 1
            print(f"ABSENTE   {b}")
            continue
        if not ok and not executer and c == "archive" and refs.get(f"refs/heads/{b}") == sha:
            print(f"A ARCHIVER {b} : etiquette {PREFIXE}{b} creee et relue avant suppression par --execute")
            continue
        if not ok:
            refusees += 1
            print(f"REFUSEE   {b} : {message}")
            continue
        if not executer:
            print(f"PROUVEE   {b} [{c}] {message}")
            continue
        r = subprocess.run(["git", "push", f"--force-with-lease=refs/heads/{b}:{sha}", DISTANT,
                            f":refs/heads/{b}"], capture_output=True, text=True)
        if r.returncode == 0:
            supprimees += 1
            print(f"SUPPRIMEE {b} [{c}] {message}")
        else:
            refusees += 1
            print(f"REFUSEE   {b} : push refuse ({r.stderr.strip().splitlines()[-1:]})")
    restantes = sorted(r[len("refs/heads/"):] for r in tetes_distantes() if r.startswith("refs/heads/"))
    print(f"\nBILAN supprimees={supprimees} refusees={refusees} absentes={absentes} "
          f"branches_distantes={len(restantes)} : {', '.join(restantes)}")
    if not executer:
        print("Verification seule. Relancer avec --execute pour archiver puis supprimer.")
    return 1 if refusees else 0


if __name__ == "__main__":
    sys.exit(main())
