#!/usr/bin/env python3
"""BOUCHAUD_SERVICES_UNIQUES_V1 -- un executable multi-appels pour les services.

    prepare-services-uniques.py SRC

Mesure (smoke Ladybird, main@e18a954) : 321 Mio lus sur le disque par session,
WebContent 101, BrowserHost 98, WebWorker 55, Compositor 43, RequestServer 13,
ImageDecoder 10. Six executables statiques distincts embarquent chacun LibWeb,
LibJS, Skia, LibGfx : le cache de pages ne peut rien partager entre eux.

Ce script ajoute a l'arbre Ladybird jetable une cible `BouchaudServices` :

  * les sources, bibliotheques, definitions, options et inclusions de chaque
    service sont RELUES sur sa cible existante (introspection CMake) : rien
    n'est recopie a la main, une preparation qui change un service change
    aussi l'executable unique ;
  * chaque `main.cpp` est compile dans une enveloppe qui renomme son point
    d'entree (`#define ladybird_main bouchaud_main_<service>`) ;
  * `dispatch.cpp` definit le seul `ladybird_main` et choisit le service sur
    le dernier composant de `argv[0]`, arguments transmis tels quels : chaque
    processus reste un processus distinct, avec son nom, ses arguments, son
    bac a sable et son code de sortie.

Les cibles separees restent construites : l'artefact porte les deux formes et
le packaging choisit (`BO_SMOKE_SERVICES_UNIQUES=1`). C'est ce qui permet un
A/B a artefact constant.
"""
import sys
from pathlib import Path

root = Path(sys.argv[1])
services_cmake = root / "Services/CMakeLists.txt"
cible = root / "Services/BouchaudServices"
cible.mkdir(exist_ok=True)

SERVICES = [
    ("WebContent", "webcontent"),
    ("WebWorker", "webworker"),
    ("Compositor", "compositor"),
    ("ImageDecoder", "imagedecoder"),
    ("RequestServer", "requestserver"),
    ("BouchaudBrowserHost", "bouchaudbrowserhost"),
]

declarations = "\n".join(f"ErrorOr<int> bouchaud_main_{bas}(Main::Arguments);" for _, bas in SERVICES)
branches = "\n".join(
    f'    if (nom == "{nom}"sv)\n        return bouchaud_main_{bas}(arguments);' for nom, bas in SERVICES)
(cible / "dispatch.cpp").write_text(f"""/*
 * BOUCHAUD_SERVICES_UNIQUES_V1 : un seul executable, plusieurs services.
 * Le service est le dernier composant de argv[0] ; les arguments sont
 * transmis tels quels et le code de sortie est celui du service.
 */
#include <AK/Format.h>
#include <AK/StringView.h>
#include <LibMain/Main.h>

{declarations}

ErrorOr<int> ladybird_main(Main::Arguments arguments)
{{
    StringView nom = arguments.strings.is_empty() ? StringView {{}} : arguments.strings[0];
    if (auto barre = nom.find_last('/'); barre.has_value())
        nom = nom.substring_view(*barre + 1);
    // `/bo-navigateur` est le nom que lance le bureau.
    if (nom == "bo-navigateur"sv)
        return bouchaud_main_bouchaudbrowserhost(arguments);
{branches}
    warnln("BOUCHAUD_SERVICES_UNIQUES service_inconnu nom={{}}", nom);
    return 127;
}}
""")

CMAKE = r"""# BOUCHAUD_SERVICES_UNIQUES_V1 -- voir tools/ladybird/prepare-services-uniques.py
# Phase 1 : tout relire et verifier. Une anomalie OMET la cible (avertissement),
# elle ne casse jamais la construction des services separes.
set(BOUCHAUD_SERVICES @LISTE@)
set(bouchaud_extras)
set(bouchaud_enveloppes)
set(bouchaud_bibliotheques)
set(bouchaud_proprietes_INCLUDE_DIRECTORIES)
set(bouchaud_proprietes_COMPILE_DEFINITIONS)
set(bouchaud_proprietes_COMPILE_OPTIONS)
set(bouchaud_options_lien)
foreach(service IN LISTS BOUCHAUD_SERVICES)
    if (NOT TARGET ${service})
        message(WARNING "BOUCHAUD_SERVICES_UNIQUES omis : cible ${service} absente")
        return()
    endif()
    string(TOLOWER "${service}" bas)
    get_target_property(dossier ${service} SOURCE_DIR)
    get_target_property(sources ${service} SOURCES)
    set(principal "")
    foreach(source IN LISTS sources)
        if (source MATCHES "^\\$<")
            message(WARNING "BOUCHAUD_SERVICES_UNIQUES omis : source generee dans ${service} : ${source}")
            return()
        endif()
        if (IS_ABSOLUTE "${source}")
            set(chemin "${source}")
        else()
            set(chemin "${dossier}/${source}")
        endif()
        cmake_path(NORMAL_PATH chemin)
        get_filename_component(nom "${chemin}" NAME)
        if (nom STREQUAL "main.cpp")
            set(principal "${chemin}")
        elseif (nom MATCHES "\\.(cpp|c|cc)$")
            list(APPEND bouchaud_extras "${chemin}")
        endif()
    endforeach()
    if (principal STREQUAL "")
        message(WARNING "BOUCHAUD_SERVICES_UNIQUES omis : main.cpp introuvable pour ${service}")
        return()
    endif()
    set(enveloppe "${CMAKE_CURRENT_BINARY_DIR}/main_${bas}.cpp")
    file(WRITE "${enveloppe}" "#define ladybird_main bouchaud_main_${bas}\n#include \"${principal}\"\n")
    list(APPEND bouchaud_enveloppes "${enveloppe}")
    get_target_property(bibliotheques ${service} LINK_LIBRARIES)
    if (bibliotheques)
        list(APPEND bouchaud_bibliotheques ${bibliotheques})
    endif()
    foreach(propriete INCLUDE_DIRECTORIES COMPILE_DEFINITIONS COMPILE_OPTIONS)
        get_target_property(valeur ${service} ${propriete})
        if (valeur)
            list(APPEND bouchaud_proprietes_${propriete} ${valeur})
        endif()
    endforeach()
    get_target_property(options_lien ${service} LINK_OPTIONS)
    if (options_lien)
        list(APPEND bouchaud_options_lien ${options_lien})
    endif()
endforeach()

# Phase 2 : la cible.
list(REMOVE_DUPLICATES bouchaud_extras)
add_executable(BouchaudServices dispatch.cpp ${bouchaud_enveloppes} ${bouchaud_extras})
target_link_libraries(BouchaudServices PRIVATE ${bouchaud_bibliotheques})
foreach(propriete INCLUDE_DIRECTORIES COMPILE_DEFINITIONS COMPILE_OPTIONS)
    if (bouchaud_proprietes_${propriete})
        list(REMOVE_DUPLICATES bouchaud_proprietes_${propriete})
        set_property(TARGET BouchaudServices PROPERTY ${propriete} ${bouchaud_proprietes_${propriete}})
    endif()
endforeach()
if (bouchaud_options_lien)
    list(REMOVE_DUPLICATES bouchaud_options_lien)
    set_property(TARGET BouchaudServices PROPERTY LINK_OPTIONS ${bouchaud_options_lien})
endif()
set_target_properties(BouchaudServices PROPERTIES
    SKIP_BUILD_RPATH TRUE
    BUILD_WITH_INSTALL_RPATH FALSE
    INSTALL_RPATH ""
)
"""
(cible / "CMakeLists.txt").write_text(CMAKE.replace("@LISTE@", " ".join(nom for nom, _ in SERVICES)))

data = services_cmake.read_text()
bloc = """if (BOUCHAUD_PORT)
    # BOUCHAUD_SERVICES_UNIQUES_V1 : APRES toutes les cibles de services.
    add_subdirectory(BouchaudServices)
endif()
"""
if bloc not in data:
    services_cmake.write_text(data.rstrip() + "\n\n" + bloc)
print("BouchaudServices prepare :", ", ".join(nom for nom, _ in SERVICES))
