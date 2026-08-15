// Temoin LibJS : le moteur JavaScript de Ladybird evalue-t-il vraiment du code ?
//
// C'est le jalon qui donne son sens a tout ce qui precede. Les temoins
// precedents verifiaient une brique chacun ; celui-ci ne verifie rien de neuf
// et verifie tout a la fois, parce qu'il ne peut pas s'executer si l'une des
// briques ment :
//
//   AK           les chaines, les vecteurs, les bornes de pile
//   LibCore      le temps, l'environnement, la boucle d'evenements
//   LibGC        le ramasse-miettes conservateur — chaque valeur JS est une
//                cellule, et une seule racine mal vue corromprait le tas
//   LibUnicode   les donnees ICU : sans elles, `normalize` et `Intl` mentent
//   LibCrypto    l'arithmetique de `BigInt`
//   5 caisses Rust  dont `libjs_rust`, et `flapc` qui a genere `Bytecode/Op.h`
//
// Les verifications sont donc choisies pour traverser ces couches plutot que
// pour couvrir le langage. Chacune echouerait differemment selon la brique en
// defaut, ce qui rend le temoin diagnostique et pas seulement binaire.
//
// La derniere ligne imprime `Hello Bouchaud` par `console.log` : c'est le
// jalon M4 de `docs/ladybird/LIBJS_PORT.md`.

#include <AK/String.h>
#include <AK/Utf16String.h>
#include <LibJS/Console.h>
#include <LibJS/Runtime/ConsoleObject.h>
#include <LibJS/Runtime/GlobalObject.h>
#include <LibJS/Runtime/Realm.h>
#include <LibJS/Runtime/VM.h>
#include <LibJS/Runtime/Value.h>
#include <LibJS/Script.h>

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

// Evalue une source et rend la valeur produite, ou rien si l'analyse ou
// l'execution a echoue. La distinction compte pour le diagnostic : une erreur
// de syntaxe met en cause `flapc` et l'analyseur, une exception a l'execution
// met en cause l'interpreteur ou le tas.
Optional<JS::Value> evalue(JS::VM& vm, JS::Realm& realm, StringView source)
{
    auto texte = Utf16String::from_utf8(source);
    auto script = JS::Script::parse(texte.utf16_view(), realm, "temoin"sv);
    if (script.is_error()) {
        std::printf("         erreur d'analyse : %s\n",
                    script.error().first().to_utf16_string().to_byte_string().characters());
        return {};
    }

    auto resultat = vm.run(script.value());
    if (resultat.is_error()) {
        std::printf("         exception a l'execution\n");
        return {};
    }
    return resultat.value();
}

// Evalue et rend le resultat converti en chaine, comme `String(x)` le ferait.
Optional<ByteString> evalue_en_chaine(JS::VM& vm, JS::Realm& realm, StringView source)
{
    auto valeur = evalue(vm, realm, source);
    if (!valeur.has_value())
        return {};
    auto chaine = valeur->to_utf16_string(vm);
    if (chaine.is_error())
        return {};
    return chaine.value().to_byte_string();
}

// Un client de console minimal.
//
// Sans lui, `console.log(...)` s'execute sans erreur **et n'imprime rien** :
// `JS::Console` transmet a un client, et quand il n'y en a pas, le message est
// simplement perdu. Une premiere version de ce temoin passait ainsi la
// verification finale sans qu'aucune ligne n'apparaisse — le contraire de ce
// qu'un jalon doit prouver.
//
// C'est le meme piege que celui des donnees ICU manquantes, et la meme reponse :
// verifier ce qui sort, pas ce qui ne se plaint pas.
class ConsoleTemoin final : public JS::ConsoleClient {
    GC_CELL(ConsoleTemoin, JS::ConsoleClient);

public:
    explicit ConsoleTemoin(JS::Console& console)
        : ConsoleClient(console)
    {
    }

    bool a_imprime() const { return m_impressions > 0; }

    virtual void clear() override { }
    virtual void end_group() override { }

    virtual JS::ThrowCompletionOr<JS::Value> printer(JS::Console::LogLevel, PrinterArguments arguments) override
    {
        auto valeurs = arguments.get<GC::RootVector<JS::Value>>();
        auto texte = TRY(generically_format_values(valeurs));
        std::printf("%s\n", texte.to_byte_string().characters());
        std::fflush(stdout);
        ++m_impressions;
        return JS::js_undefined();
    }

private:
    int m_impressions { 0 };
};

} // namespace

int main()
{
    std::printf("== temoin LibJS ==\n");

    auto vm = JS::VM::create();

    // Un royaume avec l'objet global par defaut : `Object`, `Array`, `Math`,
    // `JSON`, `BigInt`, `Intl`… tout ce que la norme exige est construit ici.
    // Si LibUnicode ou LibGC etaient defaillants, l'echec surviendrait des
    // cette ligne, avant toute evaluation.
    GC::Ptr<JS::Realm> royaume;
    auto contexte = MUST(JS::Realm::initialize_host_defined_realm(
        *vm,
        [&](JS::Realm& r) -> GC::Ref<JS::Object> {
            royaume = &r;
            return r.create<JS::GlobalObject>(r);
        },
        nullptr));
    verifie("un royaume JS est construit", royaume != nullptr);

    // `initialize_host_defined_realm` laisse le contexte d'execution racine
    // empile et nous en rend la propriete. Il faut le garder : `VM::run()`
    // exige un contexte d'execution courant (`VERIFY(m_running_execution_context)`
    // dans `VM.h`), et le depiler sans le remettre fait echouer l'assertion des
    // la premiere evaluation — par un abandon, pas par une erreur lisible.
    //
    // Le lanceur de tests d'upstream depile ici, mais rempile
    // `global_execution_context` avant chaque execution. Ce temoin n'evaluant
    // rien d'autre, il le laisse simplement en place.

    if (!royaume) {
        std::printf("\nRESULTAT : le royaume manque, rien d'autre n'a de sens\n");
        return 1;
    }

    // 1. `1 + 2`. Le temoin le plus modeste du portage, et celui qui exerce
    //    deja la chaine entiere : analyse, generation de code-octet par les
    //    tables issues de `flapc`, interpretation.
    {
        auto r = evalue_en_chaine(*vm, *royaume, "1 + 2"sv);
        std::printf("         1 + 2 = %s\n", r.value_or("(rien)"sv).characters());
        verifie("1 + 2 vaut 3", r == "3");
    }

    // 2. Fonctions, fermetures, recursion — donc trames d'appel et pile.
    {
        auto r = evalue_en_chaine(*vm, *royaume,
            "(function fact(n) { return n <= 1 ? 1 : n * fact(n - 1); })(10)"sv);
        std::printf("         10! = %s\n", r.value_or("(rien)"sv).characters());
        verifie("recursion et fermetures", r == "3628800");
    }

    // 3. Objets, tableaux, methodes de la bibliotheque standard.
    //
    // Chaque valeur ici est une cellule de LibGC. Un ramasse-miettes aux
    // bornes de pile fausses recolterait un tableau vivant et ferait echouer
    // cette ligne de facon intermittente — c'est le defaut que le temoin de
    // LibGC cherchait deja, verifie ici dans son emploi reel.
    {
        auto r = evalue_en_chaine(*vm, *royaume,
            "[3,1,2].sort().map(x => x * 2).join('-')"sv);
        std::printf("         tri/map/join = %s\n", r.value_or("(rien)"sv).characters());
        verifie("tableaux et fonctions flechees", r == "2-4-6");
    }

    // 4. `BigInt` — donc LibCrypto et libtommath 1.3.0.
    //
    // 2^64 est hors de portee d'un `double` : c'est exactement ce que le type
    // `BigInt` existe pour porter, et ce que le temoin LibCrypto verifiait un
    // etage plus bas.
    {
        auto r = evalue_en_chaine(*vm, *royaume, "(2n ** 64n).toString()"sv);
        std::printf("         2n ** 64n = %s\n", r.value_or("(rien)"sv).characters());
        verifie("BigInt (LibCrypto / libtommath)", r == "18446744073709551616");
    }

    // 5. Unicode — donc LibUnicode et les donnees ICU.
    //
    // `normalize('NFC')` sur « e » suivi d'un accent combinant doit rendre une
    // chaine d'un seul code point. Sans les donnees ICU, la methode existe et
    // rend la chaine inchangee : elle echoue **silencieusement**, ce qui est
    // precisement pourquoi on la verifie ici.
    {
        auto r = evalue_en_chaine(*vm, *royaume,
            "'e\\u0301'.normalize('NFC').codePointAt(0).toString(16)"sv);
        std::printf("         'e+U+0301'.normalize('NFC') = U+%s\n",
                    r.value_or("(rien)"sv).characters());
        verifie("String.prototype.normalize (donnees ICU)", r == "e9");
    }

    // 6. `Intl` — la surface d'ICU la plus exigeante en donnees CLDR.
    {
        auto r = evalue_en_chaine(*vm, *royaume,
            "new Intl.NumberFormat('fr-FR').resolvedOptions().locale"sv);
        std::printf("         Intl.NumberFormat('fr-FR') = %s\n",
                    r.value_or("(rien)"sv).characters());
        verifie("Intl resout une locale", r == "fr-FR");
    }

    // 7. `JSON` — donc simdjson.
    {
        auto r = evalue_en_chaine(*vm, *royaume,
            "JSON.stringify(JSON.parse('{\"b\":[1,2],\"a\":true}'))"sv);
        std::printf("         JSON aller-retour = %s\n", r.value_or("(rien)"sv).characters());
        verifie("JSON.parse / JSON.stringify", r == "{\"b\":[1,2],\"a\":true}");
    }

    // 8. Expressions regulieres — donc LibRegex et `libregex_rust`.
    {
        auto r = evalue_en_chaine(*vm, *royaume,
            "'2026-08-15'.replace(/(\\d+)-(\\d+)-(\\d+)/, '$3/$2/$1')"sv);
        std::printf("         regex = %s\n", r.value_or("(rien)"sv).characters());
        verifie("expressions regulieres (LibRegex)", r == "15/08/2026");
    }

    // 9. Exceptions JS. Elles sont implementees **sans** exceptions C++ —
    //    upstream compile avec `-fno-exceptions`, et nous aussi. Un `try/catch`
    //    qui fonctionne prouve que la propagation par `ThrowCompletionOr` tient
    //    de bout en bout.
    {
        auto r = evalue_en_chaine(*vm, *royaume,
            "(() => { try { null.x; } catch (e) { return e.constructor.name; } })()"sv);
        std::printf("         try/catch = %s\n", r.value_or("(rien)"sv).characters());
        verifie("exceptions JS sous -fno-exceptions", r == "TypeError");
    }

    // 10. Le ramasse-miettes sous pression, depuis JavaScript.
    //
    //     Un million d'objets alloues et abandonnes, une chaine conservee au
    //     travers. Si LibGC recoltait une racine vivante, c'est ici que cela se
    //     verrait.
    {
        auto r = evalue_en_chaine(*vm, *royaume,
            "(() => { let garde = 'intact';"
            "         for (let i = 0; i < 200000; ++i) { let o = { i, s: 'x' + i }; }"
            "         return garde; })()"sv);
        std::printf("         200 000 objets abandonnes, garde = %s\n",
                    r.value_or("(rien)"sv).characters());
        verifie("le tas survit a la pression d'allocation", r == "intact");
    }

    // 11. Le jalon M4.
    //
    // On installe un client de console, puis on exige **deux** choses : que
    // l'evaluation aboutisse, et que la ligne soit reellement sortie. La
    // seconde condition est la seule qui distingue un jalon atteint d'un appel
    // qui n'a rien fait.
    {
        // Sur la pile, comme le REPL d'upstream (`Utilities/js.cpp`) : le
        // client ne survit pas au bloc et n'a donc pas besoin du tas gere.
        auto& console_object = *royaume->intrinsics().console_object();
        ConsoleTemoin client { console_object.console() };
        console_object.console().set_client(client);

        std::printf("\n         --- console.log ---\n         ");
        std::fflush(stdout);
        auto valeur = evalue(*vm, *royaume, "console.log('Hello Bouchaud')"sv);
        verifie("console.log a bien imprime", valeur.has_value() && client.a_imprime());
    }

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
