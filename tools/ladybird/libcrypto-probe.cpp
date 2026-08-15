// Temoin LibCrypto : l'arithmetique a precision arbitraire, telle que LibJS
// l'emploiera.
//
// LibJS ne prend de LibCrypto que `BigInt` et `BigFraction` — la mesure des
// inclusions est dans `build-libcrypto.sh`. Ce temoin verifie donc ce qui
// portera reellement du sens en JavaScript, pas la surface de la
// bibliotheque :
//
//   - `SignedBigInteger` est ce qui donne le type `BigInt` du langage. Sa
//     promesse est l'exactitude au-dela de 2^53, la ou `double` decroche ;
//   - `BigFraction` sert aux conversions numeriques exactes (`Intl`,
//     `Number.prototype.toFixed`), la ou arrondir trop tot donne un chiffre
//     faux au bout de la chaine.
//
// L'enjeu n'est pas que le code compile : c'est que `libtommath` **1.2.1** de
// la distribution rende les memes reponses que le **1.3.0** epingle par
// upstream. Un ecart d'arithmetique ne se manifesterait pas par un plantage,
// mais par un dernier chiffre different — et personne ne remonterait de la a
// la version d'une bibliotheque tierce.

#include <AK/String.h>
#include <LibCrypto/BigFraction/BigFraction.h>
#include <LibCrypto/BigInt/SignedBigInteger.h>
#include <LibCrypto/BigInt/UnsignedBigInteger.h>

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
    std::printf("== temoin LibCrypto (BigInt / BigFraction) ==\n");

    // 1. Au-dela de ce qu'un double sait porter.
    //
    // 2^53 + 1 est le premier entier que `double` ne peut pas representer : il
    // s'arrondit a 2^53. C'est precisement la raison d'etre du type `BigInt` en
    // JavaScript, donc le premier point a verifier.
    {
        auto deux_53 = MUST(Crypto::UnsignedBigInteger::from_base(10, "9007199254740992"sv));
        auto un = Crypto::UnsignedBigInteger { 1 };
        auto somme = deux_53.plus(un);
        auto texte = MUST(somme.to_base(10));
        std::printf("         2^53 + 1 = %s\n", texte.to_byte_string().characters());
        verifie("2^53 + 1 est exact", texte == "9007199254740993"_string);
    }

    // 2. Une multiplication que rien de natif ne porte : 30! .
    //
    // 265 252 859 812 191 058 636 308 480 000 000 tient sur 108 bits. Le
    // resultat est verifie chiffre par chiffre — un calcul faux mais
    // vraisemblable serait sinon indetectable.
    {
        auto facto = Crypto::UnsignedBigInteger { 1 };
        for (u32 i = 2; i <= 30; ++i)
            facto = facto.multiplied_by(Crypto::UnsignedBigInteger { i });
        auto texte = MUST(facto.to_base(10));
        std::printf("         30! = %s\n", texte.to_byte_string().characters());
        verifie("30! est exact", texte == "265252859812191058636308480000000"_string);
    }

    // 3. Le signe, la soustraction, la division.
    {
        auto a = MUST(Crypto::SignedBigInteger::from_base(10, "-170141183460469231731687303715884105728"sv));
        auto b = Crypto::SignedBigInteger { 2 };
        auto quotient = a.divided_by(b).quotient;
        auto texte = MUST(quotient.to_base(10));
        std::printf("         -2^127 / 2 = %s\n", texte.to_byte_string().characters());
        verifie("division signee exacte",
                texte == "-85070591730234615865843651857942052864"_string);
        verifie("le signe est conserve", quotient.is_negative());
    }

    // 4. Une base autre que 10 : JavaScript accepte `0x`, `0o`, `0b`, et
    //    `BigInt.prototype.toString(radix)` rend n'importe quelle base.
    {
        auto hexa = MUST(Crypto::UnsignedBigInteger::from_base(16, "deadbeefcafe"sv));
        auto en_dec = MUST(hexa.to_base(10));
        auto retour = MUST(hexa.to_base(16));
        std::printf("         0xdeadbeefcafe = %s (retour : %s)\n",
                    en_dec.to_byte_string().characters(), retour.to_byte_string().characters());
        verifie("conversion de base aller-retour",
                en_dec == "244837814094590"_string && retour == "deadbeefcafe"_string);
    }

    // 5. `BigFraction` : la ou l'arrondi tardif change le resultat.
    //
    // 1/3 arrondi a 5 decimales doit donner 0.33333, et 2/3 doit donner
    // 0.66667 — arrondi *superieur*. Une implementation qui tronquerait
    // rendrait 0.66666, et l'ecart ne se verrait qu'au dernier chiffre d'un
    // `toFixed()` quelque part dans une page.
    {
        auto tiers = Crypto::BigFraction { Crypto::SignedBigInteger { 1 }, Crypto::UnsignedBigInteger { 3 } };
        auto deux_tiers = Crypto::BigFraction { Crypto::SignedBigInteger { 2 }, Crypto::UnsignedBigInteger { 3 } };
        auto t = tiers.to_utf16_string(5).to_byte_string();
        auto d = deux_tiers.to_utf16_string(5).to_byte_string();
        std::printf("         1/3 = %s   2/3 = %s\n", t.characters(), d.characters());
        verifie("BigFraction arrondit correctement", t == "0.33333" && d == "0.66667");
        verifie("BigFraction vers double", tiers.to_double() > 0.33333 && tiers.to_double() < 0.33334);
    }

    // 6. L'egalite et le hachage, dont LibJS se sert pour les cles de table.
    {
        auto x = MUST(Crypto::UnsignedBigInteger::from_base(10, "123456789012345678901234567890"sv));
        auto y = MUST(Crypto::UnsignedBigInteger::from_base(10, "123456789012345678901234567890"sv));
        verifie("egalite de deux grands entiers construits separement", x == y);
        verifie("hachage stable", x.hash() == y.hash());
    }

    std::printf("\nRESULTAT : %d verification(s) en echec (%d passees)\n", echecs, passees);
    return echecs == 0 ? 0 : 1;
}
