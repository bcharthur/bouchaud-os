#!/usr/bin/env python3
'''Diagnostic de l'assertion LibJS `Value.cpp:484` (P19, ouverte).

BOUCHAUD_LIBJS_VALEUR_DIAGNOSTIC_V1

`JS::Value::to_utf16_string` finit par `default: VERIFY_NOT_REACHED();`. Le
cas `default` ne couvre que trois familles de valeurs, et chacune designe un
coupable different :

  * `EMPTY_TAG`    -- la valeur speciale « vide » de LibJS a fui jusqu'a une
                      conversion en chaine : defaut du moteur (amont) ;
  * `ACCESSOR_TAG` -- un accesseur brut a ete lu comme une valeur : defaut du
                      moteur (amont) ;
  * autre tag      -- un motif qu'aucun constructeur de `Value` ne produit :
                      memoire corrompue. Sur Bouchaud, le premier suspect est
                      le noyau (copie sur ecriture, pages partagees, TLB).

Le patch ECRIT une ligne `[LB:JSVALEUR]` (tag, encodage, famille, pid) PUIS
laisse l'assertion se declencher. Il ne retire ni n'affaiblit rien :
`VERIFY_NOT_REACHED()` reste la derniere instruction du cas.

    prepare-libjs-valeur-diagnostic.py <arbre-ladybird>
'''
from pathlib import Path
import sys

MARQUEUR = "BOUCHAUD_LIBJS_VALEUR_DIAGNOSTIC_V1"

ANCRE = (
        "        // 12. Return ? ToString(primValue).\n"
        "        return primitive_value.to_utf16_string(vm);\n"
        "    }\n"
        "    default:\n"
        "        VERIFY_NOT_REACHED();\n"
        "    }\n"
        "}\n"
)

REMPLACEMENT = (
        "        // 12. Return ? ToString(primValue).\n"
        "        return primitive_value.to_utf16_string(vm);\n"
        "    }\n"
        "    default:\n"
        "        // " + MARQUEUR + " : nommer la famille AVANT l'assertion.\n"
        "        dbgln(\"[LB:JSVALEUR] site=to_utf16_string tag={:#x} encodage={:#016x} famille={} pid={}\",\n"
        "            static_cast<u64>(m_value.tag), static_cast<u64>(m_value.encoded),\n"
        "            m_value.tag == EMPTY_TAG ? \"vide\"sv : m_value.tag == ACCESSOR_TAG ? \"accesseur\"sv : \"inconnue\"sv,\n"
        "            getpid());\n"
        "        VERIFY_NOT_REACHED();\n"
        "    }\n"
        "}\n"
)


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-libjs-valeur-diagnostic.py <arbre-ladybird>", file=sys.stderr)
        return 2
    cpp = Path(sys.argv[1]).resolve() / "Libraries/LibJS/Runtime/Value.cpp"
    if not cpp.is_file():
        raise SystemExit(f"libjs-valeur: Value.cpp absent: {cpp}")
    texte = cpp.read_text(encoding="utf-8")
    if MARQUEUR in texte:
        return 0
    n = texte.count(ANCRE)
    if n != 1:
        raise SystemExit(f"libjs-valeur: ancre to_utf16_string attendue 1 fois, trouvee {n}")
    texte = texte.replace(ANCRE, REMPLACEMENT, 1)
    if "#include <unistd.h>" not in texte:
        premier = texte.index("#include ")
        texte = texte[:premier] + "#include <unistd.h>\n" + texte[premier:]
    cpp.write_text(texte, encoding="utf-8")
    print(f"{MARQUEUR}_APPLIQUE")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
