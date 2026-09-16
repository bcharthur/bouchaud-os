#!/usr/bin/env python3
"""Invariants du rendu professionnel et du demarrage graphique Trigkey."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

interfaces = [
    "src/gui/apps/calculator.rs",
    "src/gui/apps/file_explorer.rs",
    "src/gui/apps/journal.rs",
    "src/gui/apps/rustpad.rs",
    "src/gui/apps/services.rs",
    "src/gui/apps/system_info.rs",
    "src/gui/apps/terminal.rs",
    "src/gui/power_screen.rs",
]
erreurs: list[str] = []
for relatif in interfaces:
    texte = (ROOT / relatif).read_text(encoding="utf-8")
    for ancien in ("fb::draw_text(", "fb::draw_text_rgb("):
        if ancien in texte:
            erreurs.append(f"{relatif}: ancien moteur bitmap encore utilise ({ancien})")
    if "draw_text_prop" not in texte:
        erreurs.append(f"{relatif}: aucun rendu TrueType proportionnel")

bochs = (ROOT / "src/drivers/display/bochs.rs").read_text(encoding="utf-8")
debut_prop = bochs.index("pub fn draw_text_prop")
fin_prop = bochs.index("/// Largeur en pixels", debut_prop)
if "draw_char_bmp" in bochs[debut_prop:fin_prop]:
    erreurs.append("bochs.rs: le texte proportionnel peut encore retomber en bitmap")

faute = (ROOT / "src/platform/pc/ecran_faute.rs").read_text(encoding="utf-8")
if "FAULT_FONT_6" not in faute or 'include_bytes!(concat!(env!("OUT_DIR")' not in faute:
    erreurs.append("ecran_faute.rs: atlas TrueType statique absent")
if "gfx::font::glyph" in faute:
    erreurs.append("ecran_faute.rs: retour a la police bitmap 8x8")
if "etat_ansi" not in faute or "sequence_ansi" in faute:
    erreurs.append("ecran_faute.rs: analyseur CSI/ANSI incomplet")
if "EXCEPTION INITIALE" not in faute or "PISTE PRINCIPALE" not in faute:
    erreurs.append("ecran_faute.rs: diagnostic causal incomplet")
if "point_silencieux" not in faute:
    erreurs.append("ecran_faute.rs: jalons de commutation absents")

build = (ROOT / "build.rs").read_text(encoding="utf-8")
if "DejaVuSans.ttf" not in build or "font.rasterize" not in build:
    erreurs.append("build.rs: atlas de faute non genere depuis la police TTF")

stage2 = (ROOT / "src/platform/pc/stage2.rs").read_text(encoding="utf-8")
checkpoint = stage2[stage2.index("fn point_de_controle"):stage2.index("\n}", stage2.index("fn point_de_controle"))]
if "boot_step" in checkpoint:
    erreurs.append("stage2.rs: un point critique dessine encore dans le framebuffer")

icones = (ROOT / "src/gui/icones.rs").read_text(encoding="utf-8")
if "couverture_services" not in icones or "blend_rgb" not in icones:
    erreurs.append("icones.rs: icone Services sans anticrenelage vectoriel")
explorateur = (ROOT / "src/gui/apps/file_explorer.rs").read_text(encoding="utf-8")
if "couverture_icone" not in explorateur or "blend_rgb" not in explorateur:
    erreurs.append("file_explorer.rs: icones fichier/dossier encore pixelisees")

creation = (ROOT / "src/kernel/process/thread/creation.rs").read_text(encoding="utf-8")
if "adresse de retour fictive" not in creation or "(task.ctx.rsp + 8 * 8) & 0xF" not in creation:
    erreurs.append("creation.rs: pile initiale sans alignement ABI SysV")

exceptions = (ROOT / "src/arch/x86_64/idt/exceptions.rs").read_text(encoding="utf-8")
if exceptions.count("entre_exception") < 5 or "sort_exception_resolue" not in exceptions:
    erreurs.append("exceptions.rs: premiere exception non memorisee")

# BOUCHAUD_ECRAN_AU_LIEU_DU_LOGO_V1
#
# Cette garde EXIGEAIT un logo lisse, dans les deux etages. Elle protegeait
# contre un « B » pixelise, et elle a tenu -- mais l'invariant a change : un
# logo, si beau soit-il, reste FIGE pendant tout l'amorcage, et devant un ecran
# fige on ne sait ni si la machine travaille, ni ou elle est lente. C'est
# exactement ce que l'utilisateur a rapporte deux fois.
#
# Ce qui est exige maintenant est l'inverse, et il faut le verifier des DEUX
# cotes : le prechargeur UEFI peint le premier ecran, le noyau prend la suite,
# et laisser un logo dans l'un des deux suffit a le remettre a l'ecran.
gop = (ROOT / "src/platform/pc/reference_gop.rs").read_text(encoding="utf-8")
preboot = (ROOT / "tools/reference/uefi-preboot-probe/src/main.rs").read_text(encoding="utf-8")
for relatif, source in (("reference_gop.rs", gop), ("uefi-preboot-probe", preboot)):
    if "logo_alpha" in source or "draw_boot_logo" in source or "logo_lisse" in source:
        erreurs.append(
            f"{relatif}: le logo fige est revenu. Un ecran immobile ne dit ni "
            "que la machine travaille ni ou elle est lente ; l'ecran de "
            "demarrage (titre, barre, ligne d'etat) le remplace."
        )

# BOUCHAUD_AUCUNE_POLICE_BITMAP_AU_PREBOOT_V1
#
# Cette garde a exige successivement deux choses opposees, et les deux
# etaient mauvaises : d'abord un logo lisse, puis un ecran de progression
# dessine avec une police BITMAP 8x8 -- la seule rendable avant
# ExitBootServices, faute de rasteriseur TrueType.
#
# Verdict de l'utilisateur, sans appel : « des caracteres de polices en
# pixels [...] non ca c'est non ». Et il a raison au-dela de l'esthetique :
# le noyau a un atlas TrueType pre-rasterise, son ecran de demarrage est
# deja beau, et le faire preceder d'une version pixelisee du MEME ecran
# n'ajoute rien tout en abimant ce qu'on voit en premier.
#
# Ce qui est exige maintenant : le prechargeur ne dessine AUCUN texte. Le
# premier texte de la machine est celui du noyau, dans sa vraie police.
for jeton, pourquoi in (
    ("const POLICE", "une table de police bitmap"),
    ("fn glyphe(", "un rendu de glyphe"),
    ("fn texte(", "un rendu de texte"),
    ("BltRegion", "un transfert de tampon, dont un rendu de texte a besoin"),
):
    if jeton in preboot:
        erreurs.append(
            f"uefi-preboot-probe: {pourquoi} est revenu. Avant "
            "ExitBootServices il n'y a pas de rasteriseur TrueType : tout "
            "texte dessine ici sera pixelise, et precedera le bel ecran du "
            "noyau au lieu de l'annoncer."
        )

# La palette doit rester celle du noyau : un ecart de teinte au passage de
# relais se verrait comme un clignotement au milieu de l'amorcage.
faute = (ROOT / "src/platform/pc/ecran_faute.rs").read_text(encoding="utf-8")
for teinte, nom in (("0D_1117", "fond"), ("0044_A8FF", "accent"), ("002B_323F", "barre")):
    if teinte.replace("_", "") not in faute.replace("_", ""):
        erreurs.append(f"ecran_faute.rs: la teinte de {nom} a change")
if "0x0D, 0x11, 0x17" not in preboot:
    erreurs.append(
        "uefi-preboot-probe: la teinte de fond ne correspond plus a celle du "
        "noyau ; le passage de relais clignoterait au milieu de l'amorcage."
    )

if erreurs:
    raise SystemExit("\n".join(erreurs))
print("rendu professionnel: TrueType, icones AA et checkpoints sans dessin")
