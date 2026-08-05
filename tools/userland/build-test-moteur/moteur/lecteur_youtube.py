"""De l'adresse YouTube a une page de lecture.

C'est le point ou les trois morceaux se rejoignent : [`youtube`] dit ou est le
flux, [`signature`] rend son adresse utilisable, et la page batie ici confie le
reste au `<video>` du moteur — donc a libavcodec et au pilote audio.

La page produite est volontairement sobre : le titre, l'auteur, la video, et ce
que le navigateur a reellement choisi. C'est la seule facon honnete de presenter
la chose — on ne fait pas semblant d'etre YouTube, on lit une video.
"""

import html as _html_std

from . import reseau, signature, youtube


def est_pris_en_charge(url):
    return youtube.est_youtube(url)


def page(url, journal=None, hauteur_max=None, codecs=None):
    """Rend une [`reseau.Reponse`] contenant la page de lecture."""
    journal = journal or (lambda niveau, texte: None)
    video = youtube.identifiant(url)
    if not video:
        return None

    try:
        donnees, client = youtube.reponse_lecteur(video, journal)
    except LookupError as e:
        return _page_erreur(url, "YouTube n'a pas rendu de flux", str(e))
    except Exception as e:  # noqa: BLE001 — le navigateur ne doit pas tomber
        return _page_erreur(url, "Interrogation de YouTube impossible", str(e))

    flux_video, flux_audio = youtube.choisit_flux(
        donnees,
        hauteur_max=hauteur_max or youtube.HAUTEUR_MAX_PAR_DEFAUT,
        codecs=codecs)
    if flux_video is None and flux_audio is None:
        return _page_erreur(url, "Aucun format lisible",
                            "les formats proposes ne sont pas decodables ici")

    dechiffreur = None
    for flux in (flux_video, flux_audio):
        if flux is not None and flux.signature_a_dechiffrer:
            dechiffreur = signature.dechiffreur_pour(video, journal=journal)
            break

    adresses = {}
    for nom, flux in (("video", flux_video), ("audio", flux_audio)):
        if flux is None:
            continue
        resolue = signature.resout(flux.url, flux.signature_a_dechiffrer, dechiffreur)
        if resolue is None:
            return _page_erreur(
                url, "Signature indechiffrable",
                "la fonction de base.js n'a pas pu etre appliquee")
        adresses[nom] = signature.applique_n(resolue, dechiffreur)

    details = youtube.details(donnees)
    journal("log", "youtube: %s — flux %s" % (details["titre"], flux_video or flux_audio))
    return reseau.Reponse(
        url,
        _html(url, details, flux_video, flux_audio, adresses, client),
        "text/html", 200)


def _html(url, details, flux_video, flux_audio, adresses, client):
    e = _html_std.escape
    principal = adresses.get("video") or adresses.get("audio", "")
    flux = flux_video or flux_audio

    if flux_video is not None:
        media = ('<video id="lecteur" src="%s" width="854" height="480"%s>'
                 '<p>Le lecteur n\'a pas pu ouvrir ce flux.</p></video>'
                 % (e(principal),
                    (' data-audio="%s"' % e(adresses["audio"]))
                    if flux_audio is not None and "audio" in adresses else ""))
    else:
        media = '<audio id="lecteur" src="%s"></audio>' % e(principal)

    description = e(details["description"])[:1200].replace("\n", "<br>")
    piste = ("%s, %s" % (flux.codec_video or "son seul",
                         flux.codec_audio or "sans son")) if flux else ""

    return """<!doctype html>
<html><head><title>%(titre)s</title>
<style>
 body { font-family: sans-serif; margin: 0; background-color: #0f1115;
        color: #e9edf5 }
 .tete { padding: 14px 28px; background-color: #1a1f2b }
 .tete h1 { margin: 0; font-size: 21px }
 .tete p { margin: 4px 0 0; color: #9aa8bf; font-size: 14px }
 .scene { padding: 16px 28px }
 .info { color: #9aa8bf; font-size: 14px; margin: 10px 0 0 }
 .info b { color: #ffd479 }
 .desc { margin: 14px 0 0; color: #c6d0e0; font-size: 14px; line-height: 1.5 }
 .barre { margin: 12px 0 0 }
 a { color: #7fb2ff }
</style></head>
<body>
 <div class="tete">
  <h1>%(titre)s</h1>
  <p>%(auteur)s%(duree)s</p>
 </div>
 <div class="scene">
  %(media)s
  <p class="info" id="etat">Ouverture du flux…</p>
  <p class="info">Format : <b>itag %(itag)s</b> — %(piste)s — %(hauteur)s p
     — obtenu par le client <b>%(client)s</b></p>
  <div class="desc">%(description)s</div>
  <p class="barre"><a href="%(url)s">Adresse d'origine</a></p>
 </div>
 <script>
  const lecteur = document.getElementById('lecteur');
  const etat = document.getElementById('etat');
  lecteur.play();
  function rafraichis() {
    const position = lecteur.currentTime;
    const total = lecteur.duration;
    etat.innerHTML = 'Lecture : <b>' + position.toFixed(1) + ' s</b>'
      + (total > 0 ? ' / ' + total.toFixed(1) + ' s' : '')
      + ' — image <b>' + lecteur.videoWidth + '×' + lecteur.videoHeight + '</b>'
      + ' — ' + (lecteur.paused ? 'en pause' : 'en cours');
  }
  setInterval(rafraichis, 250);
  rafraichis();
 </script>
</body></html>""" % {
        "titre": e(details["titre"]),
        "auteur": e(details["auteur"]),
        "duree": (" — " + youtube.duree_lisible(details["duree"]))
                 if details["duree"] else "",
        "media": media,
        "itag": e(str(flux.itag if flux else "?")),
        "piste": e(piste),
        "hauteur": e(str(flux.hauteur if flux else 0)),
        "client": e(client),
        "description": description,
        "url": e(url),
    }


def _page_erreur(url, titre, detail):
    e = _html_std.escape
    return reseau.Reponse(url, """<!doctype html>
<html><head><title>%s</title></head>
<body style="font-family: sans-serif; margin: 40px">
 <h1 style="color:#b3261e">%s</h1>
 <p><code>%s</code></p>
 <p style="color:#5f6368">%s</p>
 <p><a href="%s">Adresse d'origine</a></p>
</body></html>""" % (e(titre), e(titre), e(url), e(detail), e(url)),
        "text/html", 0, titre)
