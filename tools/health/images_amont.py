#!/usr/bin/env python3
"""Les images d'epreuve venues du Ladybird EPINGLE, embarquees ici.

## Pourquoi elles sont embarquees et non lues sur disque

`third_party/ladybird/` est dans `.gitignore` : l'arbre amont est recupere par
`tools/ladybird/fetch.sh`, et il n'existe QUE dans le poste de construction du
navigateur. Un banc qui lit ces fichiers depuis le disque marche donc sur la
machine ou l'on vient de construire Ladybird, et casse partout ailleurs --
CI Fast, Reliability, un depot fraichement clone.

C'est exactement ce qui est arrive : le catalogue d'images lisait l'arbre
amont A L'IMPORT, si bien que son absence ne donnait pas un banc ignore mais
une exception a l'import, donc un lot rouge.

Les octets sont donc ICI. Le banc n'a plus aucune dependance hors du depot.

## Pourquoi ces fichiers-la

Les plus PETITS qui couvrent chaque chemin de decodage. Le premier jet prenait
`webp/4.webp` -- 176 972 octets a lui seul. `webp/simple-vp8.webp` emprunte le
meme chemin VP8 avec perte en 784 octets, et amont affirme deux de ses pixels.
Le catalogue embarque pese moins de quatre kibioctets en tout.

## D'ou viennent les pixels attendus

Deux sources, et la seconde a ete VERIFIEE contre la premiere.

  AMONT     `Tests/LibGfx/TestImageDecoder.cpp` affirme lui-meme certains
            pixels. C'est la reference la plus forte : c'est le decodeur que
            nous compilons qui doit les rendre.

  CHROMIUM  La ou amont n'affirme que les dimensions, les pixels ont ete
            MESURES avec le Chromium sans tete de cet environnement, qui est
            un decodeur de reference independant.

Le croisement, aux coordonnees exactes affirmees par amont :

    simple-vp8.webp        (120,232)  amont=241,239,240  chromium=241,239,240
    simple-vp8.webp        (198,202)  amont=122,170,213  chromium=122,170,213
    cmyk-no-adobe-marker   (8,1)      amont=44,184,97    chromium=44,184,97
    cmyk-no-adobe-marker   (1,8)      amont=184,44,97    chromium=184,44,97
    cmyk-no-adobe-marker   (9,9)      amont=24,24,194    chromium=24,24,194

Ecart nul sur les cinq. Les deux decodeurs s'accordent, ce qui autorise a se
servir du second la ou le premier est muet.

## La derive est surveillee

`CHEMIN_AMONT` et `SHA256` disent d'ou vient chaque octet. Quand l'arbre amont
est present -- sur le poste de construction du navigateur -- le controle du
banc REVERIFIE que l'embarque lui correspond encore. Quand il est absent, ce
controle est passe, et il le dit.

Fichier GENERE par le script qui l'accompagne dans le message de commit.
Ne pas l'editer a la main : le SHA256 ne suivrait pas.
"""
import base64

AMONT = {
    "jpeg_cmyk": {
        "chemin_amont": "jpg/cmyk-no-adobe-marker.jpg",
        "octets_attendus": 577,
        "sha256": "9722799a057f7e034133e2f37be76e6259a23009254e68a6cb90728cfa19da93",
        "b64": (
        "/9j/2wBDAAIBAQEBAQIBAQECAgICAgQDAgICAgUEBAMEBgUGBgYFBgYGBwkIBgcJBwYGCAsI"
        "CQoKCgoKBggLDAsKDAkKCgr/wAAUCAAKAAoEQxEATREAWREASxEA/8QAHwAAAQUBAQEBAQEA"
        "AAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1Fh"
        "ByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVW"
        "V1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5"
        "usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/9oADgRDAE0AWQBLAAA/"
        "APp7/guR/wAxj/tp/Wvp7/h+R/1OH/kf/wCvXrH/AAUR/wCCiP8Ax/f8T3+9/wAtK/TSv55f"
        "Ef8AyMN//wBfsv8A6GaP+H5H/U4f+R//AK9fl5qv/BRH/iaXP/E9/wCXh/8Alp/tGiv6Gv8A"
        "guR/zGP+2n9a/nl/4SPxD/0Hb3/wKf8Axo/4KI6rqn+nf8TK4/i/5bN/jRX88viP/kYb/wD6"
        "/Zf/AEM0f8JH4h/6Dt7/AOBT/wCNfud/wTd/4Ju/8E7vH3/BO74CeOvHX7BPwW1rW9a+C3ha"
        "/wBZ1nVvhbpFzdX91NpFrJLcTSyW5eWV3ZnZ2JZmYkkk0V//2Q=="
        ),
    },
    "jpeg_rgb24": {
        "chemin_amont": "jpg/rgb24.jpg",
        "octets_attendus": 2319,
        "sha256": "8da750311581f3f343536c214db39150aa812bc8ec0de0cd41bcab9d5e9660d5",
        "b64": (
        "/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRof"
        "Hh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwh"
        "MjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAAR"
        "CABAAH8DASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAA"
        "AgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkK"
        "FhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWG"
        "h4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl"
        "5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREA"
        "AgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYk"
        "NOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOE"
        "hYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk"
        "5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDm9Nk6V12nSdK4fTZOldfp0nSvHowP0jMc"
        "RudxpsnSuu02TpXD6dJ0rrtNk6V6dGB8LmOI3PO7WfpzW5az9Oa5C1n6c1t2s/TmvlMJRPzT"
        "A0Njr7WfpzW3az9Oa5G1n6c1t2s/Tmvp8JRPscDQ2PlzST92u60k/drgdJb7td1pJ+7X6jmO"
        "I3CCO/0k/drutJP3a4HST92u60k/dr4XMcRudUEedh6XfVYPS765IjLO+l31W30u+t4iPMdN"
        "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"
        ),
    },
    "webp_vp8": {
        "chemin_amont": "webp/simple-vp8.webp",
        "octets_attendus": 784,
        "sha256": "777a2b1285b2885a34ca42de685ec055f2fad4242a1e9b23534fd255f772fa32",
        "b64": (
        "UklGRggDAABXRUJQVlA4IPwCAACQJACdASrwAPAAPm02lkc/v6IhKvd4A/ANiWVu3/gp3pjQ"
        "5t39wz9AP4tgF2nfMfjL/QD3AeYB5//QB5gP/16fPot/+np/+kL6gG9Tf6fJx/CH8RBLk6F2"
        "n/IU+x0lg+v4qFfphU1Ljv7sROoWV46Kds0eun6ZOXympTGtRuEocmwqezR7CCjvyyYU7IRt"
        "M6+ibUcX9PPrM0+lebQOhc+YyXJBlLcKbCfl5QUTihbce1MrOo5twDdWgqFwlwikJoXChtQ9"
        "62SguApFoa3pJCu9ZNdmJ1CyvHr+o3a34bAMuZGX6Ytx9ezdFTpLGHJnLrFaMclofRtlPCpI"
        "LNRMZZNtuT9sLoDTbcJ4MMliWMbty5rlx/ceFyvATw3vLmt7cWgVwoUbILpK7Vh1f8qAAP75"
        "2oABXZDyHvd2jxXQQyl+g0X9OpWQb+BshKwM9lymyN1FHTv5pDaG2RedBagQftEz5Qyc0l0k"
        "sDneXqnlajn4bdnoNq4MSCIxFI6yf/NIbQ2s/QWoEH6opY1fonnp37UTmkukk6GBlL9Bk2LC"
        "2CZRpLP5O2CeVDU9TDdnJByWp7kyNvueL64NVKbPlt1ECScUp2/FkZbJMu4xOWeiULpFWvBN"
        "TQI0F5avGN052CcAAIrvKkGm69rCGw4j3rPvCprpJFL1eIweQxJUawr2dQQd5ENppYMFJzbI"
        "S+m5NDRq+tL+TmxW7dlaVWMdDobICAF5HwMPIs90zdo8WSG1TgFEFsQiFZg8G+aRNRxhd+Iu"
        "Tr6hGNd9+zOhXOHhzsKR1n50Z6JcR/moICVC1Xn11AAwe6heh1sbjBlYDDyKTRiGxtZ76TOf"
        "gH+qazJKyCQ99TxMBsZ8Wb1JrCoIO8iG1nR4pKAKgzopbxDDCbN+5HUxTH8UnXCFIbTmUlVC"
        "2TKXn8VOliAphTj45FKdcpvB92/SQo39Ye+WN3TIVypmAhtq4C047fTZCS1FGEU3D6ZONtF6"
        "Dso4gx2BWJhw8PJzcNj4VJAmI2KAAQd8IHwAAA=="
        ),
    },
    "webp_vp8l": {
        "chemin_amont": "webp/width11-height11-colors3.webp",
        "octets_attendus": 190,
        "sha256": "f017e1a974e5fdcdd9d163a7f4860da5deafad0b995b1baa3a192ba6b040f743",
        "b64": (
        "UklGRrYAAABXRUJQVlA4WAoAAAAAAAAACgAACgAAVlA4TEEAAAAvCoACABcw/2LBNMS1L4oE"
        "AmkIKoQVnv9IT4GCto2ckjgIv4riF4Q7aAfhJ+KI/geqQAUzwEI1qpfptEX79Ef/BABQU0FJ"
        "TgAAADhCSU0D7QAAAAAAEABIAAAAAQABAEgAAAABAAE4QklNBCgAAAAAAAwAAAACP/AAAAAA"
        "AAA4QklNBEMAAAAAAA1QYmVXARAABQEAAAAAAA=="
        ),
    },
    "webp_alpha": {
        "chemin_amont": "webp/semi-transparent-pixel.webp",
        "octets_attendus": 38,
        "sha256": "886ded6e8d7c1962f9374121af1f0f75d4f0410f13d39c99425192997eb63c0c",
        "b64": (
        "UklGRh4AAABXRUJQVlA4TBEAAAAvAAAAEAfQ//73v4CBiOh/AAA="
        ),
    },
}


def octets(nom: str) -> bytes:
    """Les octets du fichier amont embarque sous ce nom."""
    return base64.b64decode(AMONT[nom]["b64"])
