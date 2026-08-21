#!/usr/bin/env python3
"""Brancher le vrai ImageDecoderServer de Ladybird sur WebContent.

## Le defaut

Wikipedia chargeait, puis WebContent mourait :

    VERIFICATION FAILED: s_the
      at Libraries/LibWeb/Platform/ImageCodecPlugin.cpp:18
    instruction illegale dans le programme utilisateur
    ECHEC M9 WebContent signal=11

`ImageCodecPlugin::the()` est un singleton **installe par le lanceur**, pas par
LibWeb. Tant qu'aucune image n'apparait dans le document, personne ne le
demande ; la premiere `<img>` le demande, et il n'existe pas.

## Ce qu'upstream fait exactement

Rien d'exotique, et surtout rien qu'il faille reinventer :

  1. le processus UI cree une paire de sockets
     (`LibWebView/Process.cpp`, branche non-macOS) ;
  2. il pose `SOCKET_TAKEOVER=ImageDecoder:<fd>` dans l'environnement et
     execute `Services/ImageDecoder/main.cpp`, qui adopte le descripteur par
     `IPC::take_over_accepted_client_from_system_server` ;
  3. il envoie l'autre extremite a WebContent par le message IPC
     `connect_to_image_decoder(IPC::TransportHandle handle)` ;
  4. WebContent, dans `main.cpp`, execute alors :

         auto new_client = TRY(try_make_ref_counted<ImageDecoderClient::Client>(move(transport)));
         Web::Platform::ImageCodecPlugin::install(*new WebView::ImageCodecPlugin(move(new_client)));

## Ce que Bouchaud change, et pourquoi si peu

Bouchaud n'a pas encore de processus UI : le lanceur est
`tools/ladybird/webcontent-bootstrap.c`. Les etapes 1 et 2 y sont reproduites
telles quelles — le service ImageDecoder est le binaire upstream **non
modifie**, et il adopte sa socket par le mecanisme upstream.

Seule l'etape 3 change de forme : au lieu de faire voyager le descripteur dans
un message IPC vers WebContent, le lanceur le lui **transmet par heritage** et
le nomme dans `BOUCHAUD_IMAGEDECODER_FD`. C'est le meme choix que celui deja
retenu pour RequestServer (`prepare-m9-source.py`) : sans processus UI, il n'y
a personne pour emettre le message, et l'heritage de descripteur est le moyen
POSIX de dire la meme chose.

L'etape 4 est reprise mot pour mot. Meme client, meme greffon, meme service.

## Ce que cela n'est pas

Ce n'est pas un contournement de `VERIFY(s_the)` : le `VERIFY` reste en place et
reste vrai. Ce n'est pas non plus un decodeur maison : tout le decodage PNG,
JPEG, WebP, GIF, ICO, BMP, TIFF et JPEG2000 reste celui de `LibImageDecoders`,
execute dans son propre processus, comme upstream.
"""
from pathlib import Path
import sys

if len(sys.argv) != 2:
    raise SystemExit("usage: prepare-image-decoder.py <ladybird-worktree>")

root = Path(sys.argv[1])
web_main = root / "Services/WebContent/main.cpp"
data = web_main.read_text()

if "BOUCHAUD_IMAGEDECODER_FD" in data:
    print("ImageDecoder deja branche")
    raise SystemExit(0)

# Le bloc se place juste avant l'installation des rappels `on_*_connection`,
# donc apres la construction de `webcontent_client` et avant toute creation de
# page. Une image ne peut pas etre demandee plus tot.
anchor = """    webcontent_client->on_image_decoder_connection = [](auto const& handle) {"""

replacement = """#if defined(BOUCHAUD_PORT)
    // Voir tools/ladybird/prepare-image-decoder.py. Le lanceur Bouchaud nous
    // passe la socket du service ImageDecoder par heritage, faute de processus
    // UI capable d'emettre `connect_to_image_decoder`. Tout ce qui suit la
    // socket est le chemin d'upstream, inchange.
    if (getenv("BOUCHAUD_BROWSER_HOST") != nullptr) {
        // Le vrai BrowserHost fournit ImageDecoder via le canal IPC upstream.
        // `BOUCHAUD_IMAGEDECODER_FD` appartient uniquement au bootstrap M8/M9
        // sans processus UI.
        outln("[ladybird-bouchaud] IMAGE_DECODER_MODE browser-host-upstream");
    } else if (auto* inherited_image_fd = getenv("BOUCHAUD_IMAGEDECODER_FD")) {
        auto image_fd = atoi(inherited_image_fd);
        if (image_fd < 0) {
            warnln("[ladybird-bouchaud] IMAGE_DECODER_FD_INVALID {}", inherited_image_fd);
            return 66;
        }

        auto image_socket = TRY(Core::LocalSocket::adopt_fd(image_fd));
        auto image_client = TRY(try_make_ref_counted<ImageDecoderClient::Client>(
            make<IPC::Transport>(move(image_socket))));

        Web::Platform::ImageCodecPlugin::install(*new WebView::ImageCodecPlugin(move(image_client)));
        outln("[ladybird-bouchaud] IMAGE_DECODER_CONNECTED pid={} fd={}",
            Core::System::getpid(), image_fd);
    } else {
        // Pas de repli silencieux : sans greffon, la premiere image du premier
        // site fait tomber `VERIFY(s_the)` deux secondes plus tard, loin de la
        // cause. Le dire ici, une fois, au demarrage.
        warnln("[ladybird-bouchaud] IMAGE_DECODER_ABSENT "
               "aucun BOUCHAUD_IMAGEDECODER_FD : les images ne seront pas decodees");
    }
#endif
    webcontent_client->on_image_decoder_connection = [](auto const& handle) {"""

if anchor not in data:
    raise SystemExit("ImageDecoder : ancre on_image_decoder_connection introuvable")

data = data.replace(anchor, replacement, 1)
web_main.write_text(data)
print("ImageDecoder branche sur", web_main)
