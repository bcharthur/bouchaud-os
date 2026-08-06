// Pont entre libavcodec et Python : le module `bomedia`.
//
// Il ouvre un flux media tenu **en memoire** — pas un fichier — et rend des
// trames pretes a l'emploi : de l'image en BGRA que la toile Qt peint telle
// quelle, et du PCM 16 bits que `/dev/dsp` avale sans conversion.
//
// ## Pourquoi en memoire
//
// Le contenu ne vient pas d'un disque : il arrive par le reseau, ou par
// `SourceBuffer.appendBuffer` quand c'est le JavaScript de la page qui alimente
// le lecteur. On installe donc un `AVIOContext` sur un tampon que Python
// nourrit, ce qui est exactement le modele des Media Source Extensions.
//
// ## Ce que le module ne fait pas
//
// Il ne joue rien et n'affiche rien. Il decode, et rend chaque trame avec son
// horodatage ; c'est le lecteur Python qui decide quand la montrer et quand
// pousser le son. Cette separation est ce qui permet de synchroniser l'image
// sur l'horloge audio sans que le C ait a connaitre ni la toile ni `/dev/dsp`.

#define PY_SSIZE_T_CLEAN
#include <Python.h>

#include "bomedia.h"

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/imgutils.h>
#include <libavutil/opt.h>
#include <libswresample/swresample.h>
#include <libswscale/swscale.h>
}

#include <cstring>
#include <vector>

namespace {

/// Taille du tampon que `AVIOContext` demande a chaque lecture.
const int TAILLE_LECTURE = 32768;

/// Format de sortie audio : ce que `/dev/dsp` prend sans y toucher.
const int FREQUENCE_SORTIE = 48000;
const int VOIES_SORTIE = 2;

struct Source {
    /// Les octets du media, alimentes depuis Python.
    std::vector<unsigned char> donnees;
    /// Position de lecture de `AVIOContext`.
    size_t position = 0;
    /// L'appelant a-t-il annonce la fin de l'alimentation ?
    bool complet = false;

    AVFormatContext *format = nullptr;
    AVIOContext *io = nullptr;
    unsigned char *tampon_io = nullptr;

    int piste_video = -1;
    int piste_audio = -1;
    AVCodecContext *video = nullptr;
    AVCodecContext *audio = nullptr;
    SwsContext *echelle = nullptr;
    SwrContext *reechantillonneur = nullptr;

    AVPacket *paquet = nullptr;
    AVFrame *trame = nullptr;

    /// Dimensions de sortie de l'image, apres mise a l'echelle.
    int largeur = 0;
    int hauteur = 0;
    /// Tampon BGRA de la derniere image convertie.
    std::vector<unsigned char> image;
    bool ouvert = false;
};

std::vector<Source *> g_sources;

Source *source_par_indice(int indice)
{
    if (indice < 0 || size_t(indice) >= g_sources.size() || !g_sources[indice]) {
        PyErr_SetString(PyExc_ValueError, "bomedia: flux inconnu");
        return nullptr;
    }
    return g_sources[indice];
}

// --- Alimentation depuis la memoire -----------------------------------------

int lit_depuis_memoire(void *opaque, unsigned char *destination, int taille)
{
    Source *source = static_cast<Source *>(opaque);
    const size_t restant = source->donnees.size() - source->position;
    if (restant == 0)
        return AVERROR_EOF;
    const int pris = int(restant < size_t(taille) ? restant : size_t(taille));
    std::memcpy(destination, source->donnees.data() + source->position, size_t(pris));
    source->position += size_t(pris);
    return pris;
}

int64_t deplace_dans_memoire(void *opaque, int64_t decalage, int origine)
{
    Source *source = static_cast<Source *>(opaque);
    if (origine == AVSEEK_SIZE)
        return int64_t(source->donnees.size());
    int64_t cible = decalage;
    if (origine == SEEK_CUR)
        cible += int64_t(source->position);
    else if (origine == SEEK_END)
        cible += int64_t(source->donnees.size());
    if (cible < 0 || cible > int64_t(source->donnees.size()))
        return -1;
    source->position = size_t(cible);
    return cible;
}

// --- Ouverture ---------------------------------------------------------------

void ferme_source(Source *source)
{
    if (source->echelle) {
        sws_freeContext(source->echelle);
        source->echelle = nullptr;
    }
    if (source->reechantillonneur) {
        swr_free(&source->reechantillonneur);
    }
    if (source->video)
        avcodec_free_context(&source->video);
    if (source->audio)
        avcodec_free_context(&source->audio);
    if (source->paquet)
        av_packet_free(&source->paquet);
    if (source->trame)
        av_frame_free(&source->trame);
    if (source->format) {
        avformat_close_input(&source->format);
    } else if (source->io) {
        av_freep(&source->io->buffer);
        avio_context_free(&source->io);
    }
    source->io = nullptr;
    source->ouvert = false;
}

/// Prepare un decodeur pour une piste.
AVCodecContext *ouvre_decodeur(AVFormatContext *format, int piste)
{
    AVStream *flux = format->streams[piste];
    const AVCodec *codec = avcodec_find_decoder(flux->codecpar->codec_id);
    if (!codec)
        return nullptr;
    AVCodecContext *contexte = avcodec_alloc_context3(codec);
    if (!contexte)
        return nullptr;
    if (avcodec_parameters_to_context(contexte, flux->codecpar) < 0) {
        avcodec_free_context(&contexte);
        return nullptr;
    }
    // Un seul fil : l'OS sait en creer, mais un decodeur mono-fil se raisonne
    // mieux et suffit largement aux resolutions qu'un framebuffer affiche.
    contexte->thread_count = 1;
    if (avcodec_open2(contexte, codec, nullptr) < 0) {
        avcodec_free_context(&contexte);
        return nullptr;
    }
    return contexte;
}

bool ouvre_flux(Source *source, int largeur_voulue, int hauteur_voulue)
{
    source->tampon_io = static_cast<unsigned char *>(av_malloc(TAILLE_LECTURE));
    if (!source->tampon_io)
        return false;
    source->position = 0;
    source->io = avio_alloc_context(source->tampon_io, TAILLE_LECTURE, 0, source,
                                    lit_depuis_memoire, nullptr, deplace_dans_memoire);
    if (!source->io) {
        av_freep(&source->tampon_io);
        return false;
    }

    source->format = avformat_alloc_context();
    if (!source->format)
        return false;
    source->format->pb = source->io;
    source->format->flags |= AVFMT_FLAG_CUSTOM_IO;

    if (avformat_open_input(&source->format, nullptr, nullptr, nullptr) < 0) {
        // `avformat_open_input` a deja libere le contexte en cas d'echec.
        source->format = nullptr;
        return false;
    }
    if (avformat_find_stream_info(source->format, nullptr) < 0)
        return false;

    source->piste_video =
        av_find_best_stream(source->format, AVMEDIA_TYPE_VIDEO, -1, -1, nullptr, 0);
    source->piste_audio =
        av_find_best_stream(source->format, AVMEDIA_TYPE_AUDIO, -1, -1, nullptr, 0);

    if (source->piste_video >= 0) {
        source->video = ouvre_decodeur(source->format, source->piste_video);
        if (source->video) {
            source->largeur = largeur_voulue > 0 ? largeur_voulue : source->video->width;
            source->hauteur = hauteur_voulue > 0 ? hauteur_voulue : source->video->height;
            if (source->largeur <= 0 || source->hauteur <= 0) {
                source->largeur = source->video->width;
                source->hauteur = source->video->height;
            }
            source->image.resize(size_t(source->largeur) * size_t(source->hauteur) * 4);
        }
    }
    if (source->piste_audio >= 0) {
        source->audio = ouvre_decodeur(source->format, source->piste_audio);
        if (source->audio) {
            // Tout est ramene a 48 kHz stereo 16 bits : c'est ce que la carte
            // sait jouer, et convertir ici evite de le refaire en Python.
            AVChannelLayout sortie;
            av_channel_layout_default(&sortie, VOIES_SORTIE);
            if (swr_alloc_set_opts2(&source->reechantillonneur, &sortie,
                                    AV_SAMPLE_FMT_S16, FREQUENCE_SORTIE,
                                    &source->audio->ch_layout,
                                    source->audio->sample_fmt,
                                    source->audio->sample_rate, 0, nullptr) < 0
                || swr_init(source->reechantillonneur) < 0) {
                swr_free(&source->reechantillonneur);
            }
            av_channel_layout_uninit(&sortie);
        }
    }

    source->paquet = av_packet_alloc();
    source->trame = av_frame_alloc();
    if (!source->paquet || !source->trame)
        return false;

    source->ouvert = (source->video != nullptr) || (source->audio != nullptr);
    return source->ouvert;
}

// --- Methodes du module ------------------------------------------------------

PyObject *bomedia_ouvre(PyObject *, PyObject *args)
{
    Py_buffer donnees;
    int largeur = 0, hauteur = 0;
    if (!PyArg_ParseTuple(args, "y*|ii", &donnees, &largeur, &hauteur))
        return nullptr;

    Source *source = new Source();
    source->donnees.assign(static_cast<const unsigned char *>(donnees.buf),
                           static_cast<const unsigned char *>(donnees.buf) + donnees.len);
    PyBuffer_Release(&donnees);

    if (!ouvre_flux(source, largeur, hauteur)) {
        ferme_source(source);
        delete source;
        PyErr_SetString(bomedia_erreur(), "flux illisible ou format non reconnu");
        return nullptr;
    }

    g_sources.push_back(source);
    return PyLong_FromLong(long(g_sources.size() - 1));
}

PyObject *bomedia_info(PyObject *, PyObject *args)
{
    int indice;
    if (!PyArg_ParseTuple(args, "i", &indice))
        return nullptr;
    Source *source = source_par_indice(indice);
    if (!source)
        return nullptr;

    double duree = 0.0;
    if (source->format && source->format->duration != AV_NOPTS_VALUE)
        duree = double(source->format->duration) / AV_TIME_BASE;

    const char *nom_video = source->video ? avcodec_get_name(source->video->codec_id) : "";
    const char *nom_audio = source->audio ? avcodec_get_name(source->audio->codec_id) : "";

    double images_par_seconde = 0.0;
    if (source->piste_video >= 0) {
        const AVRational taux = source->format->streams[source->piste_video]->avg_frame_rate;
        if (taux.den > 0)
            images_par_seconde = double(taux.num) / double(taux.den);
    }

    return Py_BuildValue(
        "{s:d,s:i,s:i,s:s,s:s,s:d,s:i,s:i,s:s}",
        "duree", duree,
        "largeur", source->largeur,
        "hauteur", source->hauteur,
        "codec_video", nom_video,
        "codec_audio", nom_audio,
        "images_par_seconde", images_par_seconde,
        "a_video", source->video != nullptr ? 1 : 0,
        "a_audio", source->audio != nullptr ? 1 : 0,
        "conteneur", source->format && source->format->iformat
                         ? source->format->iformat->name : "");
}

/// Convertit la trame video courante en BGRA et rend le tampon.
PyObject *rend_image(Source *source, double horodatage)
{
    if (!source->echelle) {
        source->echelle = sws_getContext(
            source->trame->width, source->trame->height,
            AVPixelFormat(source->trame->format),
            source->largeur, source->hauteur, AV_PIX_FMT_BGRA,
            SWS_BILINEAR, nullptr, nullptr, nullptr);
        if (!source->echelle) {
            PyErr_SetString(bomedia_erreur(), "conversion d'image impossible");
            return nullptr;
        }
    }

    unsigned char *plans[4] = { source->image.data(), nullptr, nullptr, nullptr };
    int lignes[4] = { source->largeur * 4, 0, 0, 0 };
    sws_scale(source->echelle, source->trame->data, source->trame->linesize, 0,
              source->trame->height, plans, lignes);

    PyObject *octets = PyBytes_FromStringAndSize(
        reinterpret_cast<const char *>(source->image.data()),
        Py_ssize_t(source->image.size()));
    if (!octets)
        return nullptr;
    PyObject *resultat = Py_BuildValue("{s:s,s:N,s:i,s:i,s:d}",
                                       "type", "video",
                                       "donnees", octets,
                                       "largeur", source->largeur,
                                       "hauteur", source->hauteur,
                                       "horodatage", horodatage);
    return resultat;
}

/// Convertit la trame audio courante en PCM 16 bits stereo 48 kHz.
PyObject *rend_son(Source *source, double horodatage)
{
    if (!source->reechantillonneur) {
        Py_RETURN_NONE;
    }
    const int max_sortie = int(av_rescale_rnd(
        swr_get_delay(source->reechantillonneur, source->audio->sample_rate)
            + source->trame->nb_samples,
        FREQUENCE_SORTIE, source->audio->sample_rate, AV_ROUND_UP));
    if (max_sortie <= 0)
        Py_RETURN_NONE;

    std::vector<unsigned char> pcm(size_t(max_sortie) * VOIES_SORTIE * 2);
    unsigned char *sorties[1] = { pcm.data() };
    const int produits = swr_convert(
        source->reechantillonneur, sorties, max_sortie,
        const_cast<const unsigned char **>(source->trame->data),
        source->trame->nb_samples);
    if (produits <= 0)
        Py_RETURN_NONE;

    PyObject *octets = PyBytes_FromStringAndSize(
        reinterpret_cast<const char *>(pcm.data()),
        Py_ssize_t(size_t(produits) * VOIES_SORTIE * 2));
    if (!octets)
        return nullptr;
    return Py_BuildValue("{s:s,s:N,s:i,s:i,s:d}",
                         "type", "audio",
                         "donnees", octets,
                         "frequence", FREQUENCE_SORTIE,
                         "voies", VOIES_SORTIE,
                         "horodatage", horodatage);
}

PyObject *bomedia_trame(PyObject *, PyObject *args)
{
    int indice;
    if (!PyArg_ParseTuple(args, "i", &indice))
        return nullptr;
    Source *source = source_par_indice(indice);
    if (!source)
        return nullptr;
    if (!source->ouvert)
        Py_RETURN_NONE;

    // Le decodage se fait hors du verrou : c'est le seul endroit du navigateur
    // ou le processeur travaille assez longtemps pour que le fil d'interface
    // s'en ressente.
    for (;;) {
        // D'abord, une trame deja prete dans le decodeur.
        for (int passe = 0; passe < 2; ++passe) {
            AVCodecContext *contexte = passe == 0 ? source->video : source->audio;
            if (!contexte)
                continue;
            int etat;
            Py_BEGIN_ALLOW_THREADS
            etat = avcodec_receive_frame(contexte, source->trame);
            Py_END_ALLOW_THREADS
            if (etat == 0) {
                const int piste = passe == 0 ? source->piste_video : source->piste_audio;
                const AVRational base = source->format->streams[piste]->time_base;
                double horodatage = 0.0;
                if (source->trame->pts != AV_NOPTS_VALUE)
                    horodatage = double(source->trame->pts) * av_q2d(base);
                PyObject *resultat = passe == 0 ? rend_image(source, horodatage)
                                                : rend_son(source, horodatage);
                av_frame_unref(source->trame);
                if (resultat != Py_None)
                    return resultat;
                Py_XDECREF(resultat);
            }
        }

        // Sinon, on alimente les decodeurs avec le paquet suivant.
        int etat;
        Py_BEGIN_ALLOW_THREADS
        etat = av_read_frame(source->format, source->paquet);
        Py_END_ALLOW_THREADS
        if (etat < 0) {
            // Fin du flux : on vide ce qui reste dans les decodeurs.
            if (source->video)
                avcodec_send_packet(source->video, nullptr);
            if (source->audio)
                avcodec_send_packet(source->audio, nullptr);
            for (int passe = 0; passe < 2; ++passe) {
                AVCodecContext *contexte = passe == 0 ? source->video : source->audio;
                if (contexte && avcodec_receive_frame(contexte, source->trame) == 0) {
                    PyObject *resultat = passe == 0 ? rend_image(source, 0.0)
                                                    : rend_son(source, 0.0);
                    av_frame_unref(source->trame);
                    if (resultat != Py_None)
                        return resultat;
                    Py_XDECREF(resultat);
                }
            }
            Py_RETURN_NONE;
        }

        AVCodecContext *destination = nullptr;
        if (source->paquet->stream_index == source->piste_video)
            destination = source->video;
        else if (source->paquet->stream_index == source->piste_audio)
            destination = source->audio;
        if (destination) {
            Py_BEGIN_ALLOW_THREADS
            avcodec_send_packet(destination, source->paquet);
            Py_END_ALLOW_THREADS
        }
        av_packet_unref(source->paquet);
    }
}

PyObject *bomedia_alimente(PyObject *, PyObject *args)
{
    int indice;
    Py_buffer donnees;
    if (!PyArg_ParseTuple(args, "iy*", &indice, &donnees))
        return nullptr;
    Source *source = source_par_indice(indice);
    if (!source) {
        PyBuffer_Release(&donnees);
        return nullptr;
    }
    // C'est le chemin des Media Source Extensions : la page ajoute un segment,
    // il s'ajoute a la fin du tampon et le decodeur le trouvera en avancant.
    source->donnees.insert(source->donnees.end(),
                           static_cast<const unsigned char *>(donnees.buf),
                           static_cast<const unsigned char *>(donnees.buf) + donnees.len);
    PyBuffer_Release(&donnees);
    return PyLong_FromSize_t(source->donnees.size());
}

PyObject *bomedia_cherche(PyObject *, PyObject *args)
{
    int indice;
    double seconde;
    if (!PyArg_ParseTuple(args, "id", &indice, &seconde))
        return nullptr;
    Source *source = source_par_indice(indice);
    if (!source || !source->format)
        return nullptr;
    const int64_t cible = int64_t(seconde * AV_TIME_BASE);
    if (av_seek_frame(source->format, -1, cible, AVSEEK_FLAG_BACKWARD) < 0)
        Py_RETURN_FALSE;
    if (source->video)
        avcodec_flush_buffers(source->video);
    if (source->audio)
        avcodec_flush_buffers(source->audio);
    Py_RETURN_TRUE;
}

PyObject *bomedia_ferme(PyObject *, PyObject *args)
{
    int indice;
    if (!PyArg_ParseTuple(args, "i", &indice))
        return nullptr;
    Source *source = source_par_indice(indice);
    if (!source)
        return nullptr;
    ferme_source(source);
    delete source;
    g_sources[indice] = nullptr;
    Py_RETURN_NONE;
}

PyObject *bomedia_codecs(PyObject *, PyObject *)
{
    PyObject *liste = PyList_New(0);
    if (!liste)
        return nullptr;
    void *etat = nullptr;
    const AVCodec *codec = nullptr;
    while ((codec = av_codec_iterate(&etat)) != nullptr) {
        if (!av_codec_is_decoder(codec))
            continue;
        PyObject *nom = PyUnicode_FromString(codec->name);
        if (nom) {
            PyList_Append(liste, nom);
            Py_DECREF(nom);
        }
    }
    return liste;
}

PyObject *bomedia_version(PyObject *, PyObject *)
{
    return PyUnicode_FromString(av_version_info());
}

PyMethodDef bomedia_methodes[] = {
    {"ouvre", bomedia_ouvre, METH_VARARGS,
     "ouvre(octets, largeur=0, hauteur=0) -> flux"},
    {"info", bomedia_info, METH_VARARGS, "info(flux) -> dictionnaire"},
    {"trame", bomedia_trame, METH_VARARGS,
     "trame(flux) -> trame suivante (image ou son), ou None a la fin"},
    {"alimente", bomedia_alimente, METH_VARARGS,
     "alimente(flux, octets) : ajoute un segment (Media Source Extensions)"},
    {"cherche", bomedia_cherche, METH_VARARGS, "cherche(flux, seconde) -> bool"},
    {"ferme", bomedia_ferme, METH_VARARGS, "ferme(flux)"},
    {"codecs", bomedia_codecs, METH_NOARGS, "codecs() -> decodeurs disponibles"},
    {"version", bomedia_version, METH_NOARGS, "version() -> version de FFmpeg"},
    {nullptr, nullptr, 0, nullptr},
};

PyModuleDef bomedia_module = {
    PyModuleDef_HEAD_INIT, "bomedia",
    "Decodage audio et video par libavcodec.",
    -1, bomedia_methodes, nullptr, nullptr, nullptr, nullptr,
};

PyObject *g_erreur = nullptr;

} // namespace

PyObject *bomedia_erreur()
{
    return g_erreur ? g_erreur : PyExc_RuntimeError;
}

PyObject *initialise_bomedia()
{
    PyObject *module = PyModule_Create(&bomedia_module);
    if (!module)
        return nullptr;
    g_erreur = PyErr_NewException("bomedia.Erreur", nullptr, nullptr);
    if (g_erreur) {
        Py_INCREF(g_erreur);
        PyModule_AddObject(module, "Erreur", g_erreur);
    }
    return module;
}

#ifdef BOMEDIA_MODULE_PARTAGE
extern "C" PyMODINIT_FUNC PyInit_bomedia()
{
    return initialise_bomedia();
}
#endif
