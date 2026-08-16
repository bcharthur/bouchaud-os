#!/bin/bash
# Produit les sources generees de LibWeb, avec les generateurs d'upstream.
#
#   ./tools/ladybird/build-libweb-gen.sh
#
# Sortie : `third_party/gen-libweb/`, a passer en `-I` a toute compilation de
# LibWeb et de ce qui en depend.
#
# ## Pourquoi un script separe de la compilation
#
# Parce que la generation ne demande que Python, et la compilation demande
# toute la chaine (clang, Skia, ICU, OpenSSL...). Les separer rend la partie
# generation **verifiable seule**, en quelques secondes, sans runner ni cache —
# et c'est la partie ou une erreur d'ordonnancement se paierait le plus cher
# plus tard, au milieu d'un millier d'erreurs de compilation.
#
# ## L'ordre n'est pas negociable
#
# Deux generateurs CSS produisent des **fichiers IDL** en plus de leur paire
# .h/.cpp :
#
#     generate_libweb_css_style_properties.py      -> GeneratedCSSStyleProperties.idl
#     generate_libweb_css_numeric_factory_methods.py -> GeneratedCSSNumericFactoryMethods.idl
#
# Le generateur de liaisons JavaScript doit les recevoir **avec** les 661 IDL
# du depot. Sans eux il s'arrete sur :
#
#     RuntimeError: Included mixin 'GeneratedCSSStyleProperties' does not exist
#
# C'est ce que fait `LIBWEB_ALL_GENERATED_IDL` dans
# `Meta/CMake/libweb_generators.cmake` ; on le reproduit ici a l'identique.
#
# ## Rien n'est reecrit
#
# Tous les generateurs de LibWeb sont en Python — il n'y a aucun outil hote en
# C++ a construire d'abord, contrairement a ce que l'organisation de
# SerenityOS pouvait laisser croire. On appelle donc les scripts d'upstream tels
# quels, avec les memes arguments que le CMake, et c'est tout.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

LB="$RACINE/third_party/ladybird"
LW="$LB/Libraries/LibWeb"
GEN="$LB/Meta/Generators"
SORTIE="$RACINE/third_party/gen-libweb"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -d "$LW" ] || { rouge "Ladybird absent — lancer tools/ladybird/fetch.sh"; exit 1; }

PY=${PYTHON3:-python3}

rm -rf "$SORTIE"
mkdir -p "$SORTIE"/{CSS,CSS/Parser,HTML/Parser,WebGL,ARIA,MathML,SVG,Bindings}

NB=0

# `py <script> <en-tete> <impl> [args...]` — la forme `invoke_py_generator` du
# CMake : `-h` pour l'en-tete, `-c` pour l'implementation, le reste tel quel.
py() {
    local script=$1 entete=$2 impl=$3
    shift 3
    "$PY" "$GEN/$script" -h "$SORTIE/$entete" -c "$SORTIE/$impl" "$@"
    NB=$((NB + 2))
}

# Idem, plus un `-i` : le generateur produit aussi un fichier IDL, qui devra
# etre donne au generateur de liaisons.
py_idl() {
    local script=$1 entete=$2 impl=$3 idl=$4
    shift 4
    "$PY" "$GEN/$script" -h "$SORTIE/$entete" -c "$SORTIE/$impl" -i "$SORTIE/$idl" "$@"
    NB=$((NB + 3))
}

# `embed_as_string` : une feuille de style devient un `StringView` constant.
embed() {
    local source=$1 sortie=$2 variable=$3 espace=$4
    "$PY" "$GEN/embed_as_string.py" "$source" -o "$SORTIE/$sortie" -n "$variable" -s "$espace"
    NB=$((NB + 1))
}

# --- CSS ---------------------------------------------------------------------
info "== CSS =="
py generate_libweb_css_descriptors.py CSS/DescriptorID.h CSS/DescriptorID.cpp \
    -j "$LW/CSS/Descriptors.json"
py generate_libweb_css_enums.py CSS/Enums.h CSS/Enums.cpp \
    -j "$LW/CSS/Enums.json"
py generate_libweb_css_environment_variables.py CSS/EnvironmentVariable.h CSS/EnvironmentVariable.cpp \
    -j "$LW/CSS/EnvironmentVariables.json"
py generate_libweb_css_math_functions.py CSS/MathFunctions.h CSS/MathFunctions.cpp \
    -j "$LW/CSS/MathFunctions.json"
py generate_libweb_css_media_feature_id.py CSS/MediaFeatureID.h CSS/MediaFeatureID.cpp \
    -j "$LW/CSS/MediaFeatures.json"
# Seul generateur a prendre trois entrees : les groupes logiques decident des
# proprietes physiques equivalentes.
py generate_libweb_css_property_id.py CSS/PropertyID.h CSS/PropertyID.cpp \
    -j "$LW/CSS/Properties.json" \
    -e "$LW/CSS/Enums.json" \
    -g "$LW/CSS/LogicalPropertyGroups.json"
py generate_libweb_css_pseudo_class.py CSS/PseudoClass.h CSS/PseudoClass.cpp \
    -j "$LW/CSS/PseudoClasses.json"
py generate_libweb_css_pseudo_element.py CSS/PseudoElement.h CSS/PseudoElement.cpp \
    -j "$LW/CSS/PseudoElements.json"
py generate_libweb_css_transform_functions.py CSS/TransformFunctions.h CSS/TransformFunctions.cpp \
    -j "$LW/CSS/TransformFunctions.json"
py generate_libweb_css_value_types_parsing.py CSS/Parser/GeneratedValueTypesParsing.h CSS/Parser/GeneratedValueTypesParsing.cpp \
    -j "$LW/CSS/ValueTypes.json" \
    -u "$LW/CSS/Units.json"
py generate_libweb_css_units.py CSS/Units.h CSS/Units.cpp \
    -j "$LW/CSS/Units.json"
py generate_libweb_css_keyword.py CSS/Keyword.h CSS/Keyword.cpp \
    -j "$LW/CSS/Keywords.json"

# Les deux qui produisent de l'IDL.
py_idl generate_libweb_css_numeric_factory_methods.py \
    CSS/GeneratedCSSNumericFactoryMethods.h \
    CSS/GeneratedCSSNumericFactoryMethods.cpp \
    CSS/GeneratedCSSNumericFactoryMethods.idl \
    -j "$LW/CSS/Units.json"
py_idl generate_libweb_css_style_properties.py \
    CSS/GeneratedCSSStyleProperties.h \
    CSS/GeneratedCSSStyleProperties.cpp \
    CSS/GeneratedCSSStyleProperties.idl \
    -j "$LW/CSS/Properties.json"

# Feuilles de style embarquees.
embed "$LW/CSS/Default.css"    CSS/DefaultStyleSheetSource.cpp    default_stylesheet_source    Web::CSS
embed "$LW/CSS/QuirksMode.css" CSS/QuirksModeStyleSheetSource.cpp quirks_mode_stylesheet_source Web::CSS
embed "$LW/MathML/Default.css" MathML/MathMLStyleSheetSource.cpp  mathml_stylesheet_source      Web::CSS
embed "$LW/SVG/Default.css"    SVG/SVGStyleSheetSource.cpp        svg_stylesheet_source         Web::CSS

# --- HTML --------------------------------------------------------------------
info "== HTML =="
py generate_libweb_html_named_character_references.py \
    HTML/Parser/NamedCharacterReferences.h HTML/Parser/NamedCharacterReferences.cpp \
    -j "$LW/HTML/Parser/Entities.json"
# Les controles de lecture video sont un fragment HTML compile en arbre DOM
# constant. Le generateur lit les listes de balises et d'attributs pour refuser
# tout ce qui n'existe pas — d'ou les quatre en-tetes en entree.
py generate_dom_tree.py HTML/MediaControlsDOM.h HTML/MediaControlsDOM.cpp \
    -i "$LW/HTML/MediaControls.html" \
    -s MediaControlsDOM \
    -n "Web::HTML" \
    --html-tags "$LW/HTML/TagNames.h" \
    --html-attributes "$LW/HTML/AttributeNames.h" \
    --svg-tags "$LW/SVG/TagNames.h" \
    --svg-attributes "$LW/SVG/AttributeNames.h"

# --- WebGL -------------------------------------------------------------------
info "== WebGL =="
py generate_libweb_webgl_functions.py WebGL/GLFunctions.h WebGL/GLFunctions.cpp \
    -j "$LW/WebGL/GLFunctions.json"
py generate_libweb_webgl_commands.py WebGL/WebGLCommands.h WebGL/WebGLCommands.cpp \
    -j "$LW/WebGL/GLFunctions.json"
py generate_libweb_webgl_proxy.py WebGL/WebGLContextProxy.h WebGL/WebGLContextProxy.cpp \
    -j "$LW/WebGL/GLFunctions.json"

# --- ARIA --------------------------------------------------------------------
info "== ARIA =="
py generate_libweb_aria_roles.py ARIA/AriaRoles.h ARIA/AriaRoles.cpp \
    -j "$LW/ARIA/AriaRoles.json"

# --- Liaisons JavaScript -----------------------------------------------------
#
# 661 IDL du depot + les 2 produits ci-dessus. L'ordre importe : voir l'en-tete.
info "== liaisons JavaScript (IDL -> C++) =="
IDL_SOURCE=$(find "$LW" -name '*.idl' | sort)
# shellcheck disable=SC2086
"$PY" "$GEN/generate_libweb_bindings.py" -o "$SORTIE/Bindings" \
    $IDL_SOURCE \
    "$SORTIE/CSS/GeneratedCSSStyleProperties.idl" \
    "$SORTIE/CSS/GeneratedCSSNumericFactoryMethods.idl"

LIAISONS=$(find "$SORTIE/Bindings" -name '*.cpp' | wc -l)
NB=$((NB + LIAISONS * 2))

# --- IPC de LibWeb -----------------------------------------------------------
#
# LibWeb declare son propre endpoint de worker. Meme generateur qu'en M5.
info "== IPC LibWeb =="
mkdir -p "$SORTIE/LibWeb/Worker"
"$PY" "$GEN/generate_ipc_definitions.py" \
    --input "$LW/Worker/WebWorkerServer.ipc" \
    --output "$SORTIE/LibWeb/Worker/WebWorkerServerEndpoint.h"
NB=$((NB + 1))

# --- Verification ------------------------------------------------------------
#
# Un generateur qui echoue le dit (`set -e`), mais un generateur qui reussit en
# n'ecrivant rien ne le dirait pas. On compte, et on exige les fichiers dont
# l'absence est la plus parlante.
for atttendu in \
    CSS/PropertyID.h \
    CSS/GeneratedCSSStyleProperties.idl \
    HTML/Parser/NamedCharacterReferences.h \
    ARIA/AriaRoles.h \
    Bindings/Window.h \
    Bindings/WindowExposedInterfaces.cpp \
    LibWeb/Worker/WebWorkerServerEndpoint.h
do
    [ -s "$SORTIE/$atttendu" ] || { rouge "genere mais vide ou absent : $atttendu"; exit 1; }
done

if [ "$LIAISONS" -lt 600 ]; then
    rouge "seulement $LIAISONS liaisons generees — attendu ~669"
    exit 1
fi

echo
vert "LibWeb : $NB fichiers generes dans third_party/gen-libweb"
printf '  liaisons JavaScript : %s paires\n' "$LIAISONS"
printf '  taille              : %s\n' "$(du -sh "$SORTIE" | cut -f1)"
