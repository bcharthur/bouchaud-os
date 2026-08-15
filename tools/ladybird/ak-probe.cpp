// Temoin AK : la base de Ladybird tourne-t-elle, et ou ?
//
// AK est la couche que *toutes* les bibliotheques de Ladybird incluent — 630
// inclusions rien que depuis LibJS. Si AK ne fonctionne pas, rien ne
// fonctionnera, et le diagnostic sera d'autant plus penible que le defaut se
// manifestera loin de sa cause.
//
// Cette sonde exerce donc ce dont le reste depend :
//
//   - les chaines (`String`, `StringView`, `StringBuilder`, UTF-8/UTF-16) ;
//   - les conteneurs (`Vector`, `HashMap`, `Optional`) ;
//   - `ErrorOr` et `TRY`, le mecanisme de propagation d'erreur sans exception
//     qui remplace `throw` dans tout Ladybird ;
//   - le formatage (`AK::Format`, donc fmt) ;
//   - `JsonParser`, qui traverse chaines, conteneurs et nombres d'un coup ;
//   - `StackInfo`, qui lit les bornes de la pile — c'est **la** brique dont le
//     ramasse-miettes de LibGC se sert pour trouver ses racines. Une valeur
//     fausse ici ne se voit pas tout de suite : elle fait recolter des objets
//     vivants beaucoup plus tard.
//
// Convention de sortie identique aux sondes de `tools/userland/` : la derniere
// ligne est lisible par `tools/test.sh`.

#include <AK/ByteString.h>
#include <AK/Error.h>
#include <AK/Format.h>
#include <AK/HashMap.h>
#include <AK/JsonObject.h>
#include <AK/JsonParser.h>
#include <AK/JsonValue.h>
#include <AK/Optional.h>
#include <AK/StackInfo.h>
#include <AK/String.h>
#include <AK/StringBuilder.h>
#include <AK/StringView.h>
#include <AK/Utf8View.h>
#include <AK/Vector.h>

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

// `ErrorOr` + `TRY` : le remplacant des exceptions dans tout Ladybird.
ErrorOr<int> peut_echouer(bool echouer)
{
    if (echouer)
        return Error::from_string_literal("echec demande");
    return 21;
}

ErrorOr<int> enchaine()
{
    auto valeur = TRY(peut_echouer(false));
    return valeur * 2;
}

} // namespace

int main()
{
    std::printf("== temoin AK ==\n");

    // 1. Chaines : la structure la plus utilisee de Ladybird.
    {
        auto s = MUST(String::from_utf8("Bouchaud OS"sv));
        verifie("String::from_utf8", s.bytes_as_string_view() == "Bouchaud OS"sv);
        verifie("String::contains", s.contains("chaud"sv));
    }

    // 2. StringBuilder : la construction incrementale.
    {
        StringBuilder b;
        for (int i = 0; i < 100; ++i)
            b.appendff("{},", i);
        auto s = MUST(b.to_string());
        verifie("StringBuilder + appendff (100 elements)",
                s.bytes_as_string_view().starts_with("0,1,2,"sv)
                    && s.bytes_as_string_view().ends_with("99,"sv));
    }

    // 3. UTF-8 : Ladybird manipule du texte du monde entier, pas de l'ASCII.
    {
        auto vue = Utf8View { "héllo — 日本語"sv };
        size_t points = 0;
        for ([[maybe_unused]] auto point : vue)
            ++points;
        // « héllo — 日本語 » : 11 points de code pour 21 octets. L'ecart entre
        // les deux est exactement ce qu'un moteur Web doit savoir manipuler.
        verifie("Utf8View : parcours par point de code", points == 11);
        verifie("Utf8View : validation", vue.validate());
    }

    // 4. Conteneurs.
    {
        Vector<int> v;
        for (int i = 0; i < 1000; ++i)
            v.append(i);
        verifie("Vector : 1000 elements", v.size() == 1000 && v[999] == 999);

        HashMap<ByteString, int> m;
        m.set("un", 1);
        m.set("deux", 2);
        m.set("trois", 3);
        auto trouve = m.get("deux");
        verifie("HashMap", m.size() == 3 && trouve.has_value() && *trouve == 2);

        Optional<int> vide;
        Optional<int> plein { 7 };
        verifie("Optional", !vide.has_value() && plein.has_value() && *plein == 7);
    }

    // 5. ErrorOr / TRY : le coeur de la gestion d'erreur sans exception.
    {
        auto bon = enchaine();
        verifie("ErrorOr + TRY : succes", !bon.is_error() && bon.value() == 42);

        auto mauvais = peut_echouer(true);
        verifie("ErrorOr : erreur propagee", mauvais.is_error());
    }

    // 6. Formatage — c'est fmt qui travaille dessous.
    {
        // `{:.3}` compte les decimales chez AK (3.142), la ou `printf("%.3g")`
        // compterait les chiffres significatifs. Verifie contre le comportement
        // reel, pas contre celui qu'on suppose.
        auto s = MUST(String::formatted("{} {:04} {:.3}", "valeur"sv, 42, 3.14159));
        verifie("AK::Format", s.bytes_as_string_view() == "valeur 0042 3.142"sv);
    }

    // 7. JSON : traverse chaines, conteneurs et nombres d'un seul coup.
    {
        auto texte = R"({"nom":"Bouchaud","version":2,"actif":true,"liste":[1,2,3]})"sv;
        auto valeur = JsonParser::parse(texte);
        bool ok = !valeur.is_error();
        if (ok) {
            auto const& objet = valeur.value().as_object();
            auto nom = objet.get_string("nom"sv);
            ok = nom.has_value() && nom->bytes_as_string_view() == "Bouchaud"sv
                && objet.get_integer<int>("version"sv).value_or(0) == 2
                && objet.get_bool("actif"sv).value_or(false)
                && objet.get_array("liste"sv).has_value()
                && objet.get_array("liste"sv)->size() == 3;
        }
        verifie("JsonParser : objet complet", ok);
    }

    // 8. StackInfo : les bornes de pile dont le GC de LibJS a besoin.
    //
    // On ne verifie pas seulement que l'appel repond : on verifie que la
    // reponse a un sens. Une pile de taille nulle, ou qui ne contient pas
    // l'adresse d'une variable locale, ferait balayer au GC une zone qui n'est
    // pas la pile — et il recolterait des objets vivants.
    {
        StackInfo infos;
        int local = 0;
        auto adresse = reinterpret_cast<FlatPtr>(&local);
        bool coherent = infos.size() > 0
            && infos.base() != 0
            && adresse >= infos.base()
            && adresse < infos.top();
        std::printf("         pile : base=%#zx sommet=%#zx taille=%zu Kio\n",
                    static_cast<size_t>(infos.base()), static_cast<size_t>(infos.top()),
                    infos.size() / 1024);
        verifie("StackInfo : bornes coherentes (racines du GC)", coherent);
    }

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
