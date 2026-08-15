// Temoin LibUnicode : les *donnees* d'ICU sont-elles vraiment la ?
//
// Ce temoin ne verifie presque pas d'API. Les signatures, le compilateur les a
// deja validees — les 23 sources ont compile. Ce qu'il verifie, c'est la seule
// chose que la compilation ne dit pas : que le jeu de donnees Unicode/CLDR est
// bien present dans le binaire et lisible a l'execution.
//
// La question n'est pas theorique. `build-icu.sh` construit ICU avec
// `--with-data-packaging=static` : les donnees deviennent `libicudata.a`, 31
// Mio en un seul objet. Et le document de portage prevoit, si cette taille
// devient inacceptable pour l'image, de la reduire avec `ICU_DATA_FILTER_FILE`.
//
// Or un jeu de donnees ampute ne casse rien a la compilation, et ne leve
// aucune erreur a l'execution : les fonctions rendent simplement des reponses
// **fausses mais plausibles**. Une normalisation qui ne normalise pas, une
// locale qui ne se canonicalise plus, un decoupage en graphemes qui coupe un
// emoji en deux. Chacune de ces regressions ressortirait quelque part dans
// LibJS — dans `String.prototype.normalize`, dans `Intl`, dans la longueur
// d'une chaine — loin de sa cause.
//
// Chaque verification ci-dessous vise donc une table de donnees distincte, et
// une reponse que seul l'acces reel a cette table peut produire.

#include <AK/String.h>
#include <AK/Utf16String.h>
#include <AK/Utf16View.h>
#include <LibUnicode/CharacterTypes.h>
#include <LibUnicode/Locale.h>
#include <LibUnicode/Normalize.h>
#include <LibUnicode/Segmenter.h>

#include <cstdio>

namespace {

int echecs = 0;
int passees = 0;

void verifie(char const* quoi, bool ok)
{
    if (ok) {
        ++passees;
        std::printf("  ok     %s\n", quoi);
    } else {
        ++echecs;
        std::printf("  ECHEC  %s\n", quoi);
    }
}

} // namespace

int main()
{
    std::printf("== temoin LibUnicode ==\n");

    // 1. Proprietes de code point — table `uprops`.
    //
    // La plus simple, et donc la premiere : si celle-ci echoue, les donnees ne
    // sont pas chargees du tout et rien de ce qui suit n'a de sens.
    {
        bool ok = Unicode::code_point_has_letter_general_category('A')
            && !Unicode::code_point_has_letter_general_category('7')
            && Unicode::code_point_has_number_general_category('7')
            // U+4E9C, ideogramme : lettre pour Unicode. Un jeu de donnees
            // reduit au latin repondrait non.
            && Unicode::code_point_has_letter_general_category(0x4E9C);
        verifie("proprietes de code point (uprops)", ok);
    }

    // 2. Emoji — table `emoji-data`, distincte de `uprops`.
    {
        bool ok = Unicode::code_point_has_emoji_property(0x1F600)
            && !Unicode::code_point_has_emoji_property('A');
        verifie("proprietes emoji", ok);
    }

    // 3. Normalisation — table `nfc`/`nfkc`.
    //
    // « é » s'ecrit de deux facons : U+00E9 precompose, ou U+0065 U+0301
    // (« e » suivi d'un accent combinant). NFC doit ramener la seconde a la
    // premiere, NFD faire l'inverse. C'est exactement ce que
    // `String.prototype.normalize` promet en JavaScript.
    {
        auto compose = "é"_string;    // é, un seul code point
        auto decompose = "é"_string; // e + accent combinant

        auto nfc = Unicode::normalize(decompose, Unicode::NormalizationForm::NFC);
        auto nfd = Unicode::normalize(compose, Unicode::NormalizationForm::NFD);

        std::printf("         NFC(e+U+0301) = %zu octet(s), NFD(U+00E9) = %zu octet(s)\n",
                    nfc.bytes().size(), nfd.bytes().size());
        verifie("normalisation NFC", nfc == compose);
        verifie("normalisation NFD", nfd == decompose);
    }

    // 4. Locales — donnees CLDR.
    //
    // `add_likely_subtags` est le test le plus revelateur d'un jeu de donnees
    // filtre : il consulte `likelySubtags`, une table que les filtres
    // suppriment volontiers parce qu'elle ne ressemble a rien d'essentiel.
    // « fr » doit devenir « fr-Latn-FR » — une reponse qu'on ne peut pas
    // deviner sans la table.
    {
        auto canon = Unicode::canonicalize_unicode_locale_id("FR-fr"sv);
        std::printf("         canonicalize(\"FR-fr\") = %s\n", canon.to_byte_string().characters());
        verifie("canonicalisation de locale", canon == "fr-FR"_utf16);

        // `utf16_view()` est supprime sur un rvalue — AK refuse de rendre une
        // vue sur un temporaire qui mourrait avant elle. Il faut donc nommer
        // la chaine. C'est la meme discipline que LibJS emploie partout.
        auto fr = "fr"_utf16;
        auto etendu = Unicode::add_likely_subtags(fr.utf16_view());
        std::printf("         add_likely_subtags(\"fr\") = %s\n",
                    etendu.has_value() ? etendu->to_byte_string().characters() : "(rien)");
        verifie("sous-etiquettes probables (CLDR likelySubtags)",
                etendu.has_value() && *etendu == "fr-Latn-FR"_utf16);

        verifie("locale disponible", Unicode::is_locale_available("en-US"sv));
    }

    // 5. Segmentation en graphemes — donnees de `brkitr`.
    //
    // Le cas qui compte : un emoji drapeau est fait de **deux** indicateurs
    // regionaux (U+1F1EB U+1F1F7 = 🇫🇷), donc de deux code points, mais d'un
    // seul grapheme. Un segmenteur sans donnees de rupture en compterait deux
    // et couperait le drapeau en son milieu — le genre de defaut qu'on ne voit
    // qu'a l'affichage, une fois tout le reste construit.
    {
        auto segmenteur = Unicode::Segmenter::create(Unicode::SegmenterGranularity::Grapheme);
        auto drapeau = "\U0001F1EB\U0001F1F7"_utf16; // 🇫🇷

        size_t graphemes = 0;
        segmenteur->for_each_boundary(drapeau.utf16_view(), [&](size_t) {
            ++graphemes;
            return IterationDecision::Continue;
        });

        // `for_each_boundary` rend les *frontieres*, debut et fin comprises :
        // un grapheme unique donne donc deux frontieres.
        std::printf("         \"🇫🇷\" : %zu unite(s) UTF-16, %zu frontiere(s)\n",
                    drapeau.length_in_code_units(), graphemes);
        verifie("un drapeau regional est un seul grapheme", graphemes == 2);
    }

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
