#!/usr/bin/env python3
"""Faire sortir la console JavaScript sur la console serie.

## Pourquoi

Deux besoins distincts se rejoignent ici.

Le premier est celui de tous les jours : un navigateur dont on ne peut pas lire
`console.log` ne se debogue pas. Chez upstream, les messages partent en IPC vers
le processus UI (`async_did_output_js_console_message`), qui les affiche. Sous
Bouchaud, le pair de cette socket ne fait que la vider : chaque message y
disparait sans trace, y compris les exceptions non rattrapees.

Le second est celui des tests. Une page de fixture peut verifier une quarantaine
de choses — qu'une image a bien ete **decodee** et pas seulement telechargee,
qu'une promesse s'est resolue, qu'un minuteur a tire, que `localStorage` rend ce
qu'on y a mis — mais rien de tout cela n'est observable si son verdict ne
franchit pas la limite du processus. Une capture d'ecran ne dit pas *pourquoi*
elle differe.

Avec cette passerelle, la page ecrit ses resultats et l'integration continue les
lit dans le journal serie. Le test devient une assertion de texte, pas une
comparaison de pixels.

## Ce que cela n'est pas

Ce n'est pas un remplacement de l'IPC : `async_did_output_js_console_message`
continue d'etre appele. La sortie serie s'ajoute, sous `BOUCHAUD_M9`, et le
jour ou Bouchaud aura un vrai processus hote, seule la ligne serie sera a
retirer.

Les erreurs et les traces d'appel passent aussi, avec leur niveau : une
exception silencieuse dans un script de page est exactement le genre de panne
qui ferait perdre une journee autrement.
"""
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-console.py <ladybird-worktree>")

root = Path(sys.argv[1])
page_cpp = root / "Services/WebContent/PageClient.cpp"
data = page_cpp.read_text()

if "JS_CONSOLE" in data:
    print("passerelle console deja en place")
    raise SystemExit(0)

ancre = """void PageClient::did_output_js_console_message(WebView::ConsoleOutput console_output)
{
    client().async_did_output_js_console_message(m_id, move(console_output));"""

remplacement = """void PageClient::did_output_js_console_message(WebView::ConsoleOutput console_output)
{
#if defined(BOUCHAUD_PORT)
    // Voir tools/ladybird/prepare-console.py. La copie serie precede l'envoi
    // IPC parce que `move(console_output)` vide la valeur.
    if (bouchaud_m9_enabled()) {
        console_output.output.visit(
            [](WebView::ConsoleLog const& journal) {
                StringBuilder texte;
                for (auto const& argument : journal.arguments) {
                    if (!texte.is_empty())
                        texte.append(' ');
                    // `as_string()` n'existe que pour une chaine ; les autres
                    // valeurs se serialisent en JSON, ce qui est exactement ce
                    // que la console d'upstream affiche.
                    if (argument.is_string())
                        texte.append(argument.as_string());
                    else
                        texte.append(argument.serialized());
                }
                // `LogLevel` n'a pas de conversion en chaine dans l'arbre
                // epingle ; on nomme les niveaux qui comptent pour un journal
                // et on rend les autres par leur rang, sans inventer de noms.
                StringView niveau;
                switch (journal.level) {
                case JS::Console::LogLevel::Debug:  niveau = "debug"sv;  break;
                case JS::Console::LogLevel::Error:  niveau = "erreur"sv; break;
                case JS::Console::LogLevel::Info:   niveau = "info"sv;   break;
                case JS::Console::LogLevel::Log:    niveau = "log"sv;    break;
                case JS::Console::LogLevel::Warn:   niveau = "avert"sv;  break;
                default:                            niveau = "autre"sv;  break;
                }
                outln("[ladybird-bouchaud] JS_CONSOLE {} {}", niveau, texte.string_view());
            },
            [](WebView::ConsoleError const& erreur) {
                outln("[ladybird-bouchaud] JS_CONSOLE_ERREUR {} : {}", erreur.name, erreur.message);
                for (auto const& cadre : erreur.trace) {
                    outln("[ladybird-bouchaud] JS_CONSOLE_CADRE {} {}:{}",
                        cadre.function.value_or("(anonyme)"_string),
                        cadre.file.value_or("(inconnu)"_string),
                        cadre.line.value_or(0));
                }
            },
            [](WebView::ConsoleTrace const& trace) {
                outln("[ladybird-bouchaud] JS_CONSOLE_TRACE {}", trace.label);
            });
    }
#endif
    client().async_did_output_js_console_message(m_id, move(console_output));"""

if ancre not in data:
    raise SystemExit("console : ancre did_output_js_console_message introuvable")

data = data.replace(ancre, remplacement, 1)

if "#include <LibWebView/ConsoleOutput.h>" not in data:
    data = data.replace(
        "#include <WebContent/WebUIConnection.h>",
        "#include <LibWebView/ConsoleOutput.h>\n#include <WebContent/WebUIConnection.h>",
        1,
    )

page_cpp.write_text(data)
print("Passerelle console JavaScript -> serie appliquee a", page_cpp)
