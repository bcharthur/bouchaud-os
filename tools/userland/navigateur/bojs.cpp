// Pont entre CPython et QuickJS : le module Python `bojs`.
//
// C'est tout ce que le C apporte au JavaScript du navigateur. Le DOM, les
// evenements, les minuteries et les requetes ne sont pas ici : ils sont ecrits
// en Python (`moteur/js.py`) et en JavaScript (`moteur/prelude.js`), au-dessus
// des quelques primitives definies dans ce fichier.
//
// ## Une seule fonction de passage
//
// Exposer le DOM depuis le C demanderait des centaines de liaisons, a refaire a
// chaque methode ajoutee. On expose donc **une** fonction — `__bo_appel(op,
// ...)` cote JavaScript — qui aboutit a un appelable Python. Le reste se decide
// en Python : ajouter `document.createComment` ne touche plus au C.
//
// Le chemin inverse existe aussi : [`bojs_appelle`] permet a Python d'appeler
// une fonction globale JavaScript, ce dont la distribution d'evenements a
// besoin.
//
// ## Conversion des valeurs
//
// `None`/`null`/`undefined`, booleens, entiers, flottants, chaines, listes et
// dictionnaires traversent dans les deux sens. Le reste — fonctions, classes,
// objets hotes — ne traverse pas : ce qui doit etre designe de part et d'autre
// l'est par un entier (l'identifiant d'un nœud), jamais par une reference.
//
// ## Verrou global et budget de temps
//
// Le JavaScript s'execute sur le fil qui appelle `evalue`, verrou Python tenu :
// les rappels vers Python se font donc sans ceremonie. En echange, une page qui
// boucle bloquerait tout — d'ou le gardien d'interruption, qui arrete un script
// depassant son budget au lieu de figer la machine.

#define PY_SSIZE_T_CLEAN
#include <Python.h>

#include "bojs.h"

extern "C" {
#include "quickjs.h"
}

#include <cstring>
#include <ctime>
#include <vector>

namespace {

/// Profondeur maximale de conversion : une structure plus profonde est tronquee
/// plutot que de faire deborder la pile sur un objet cyclique.
const int PROFONDEUR_MAX = 32;

/// Budget par defaut d'une execution, en millisecondes.
const long BUDGET_MS_DEFAUT = 5000;

struct Contexte {
    JSRuntime *execution = nullptr;
    JSContext *contexte = nullptr;
    /// L'appelable Python derriere `__bo_appel`.
    PyObject *pont = nullptr;
    /// Echeance de l'execution en cours (millisecondes monotones), 0 si aucune.
    long echeance = 0;
    long budget = BUDGET_MS_DEFAUT;
    bool interrompu = false;
};

std::vector<Contexte *> g_contextes;

long maintenant_ms()
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return long(ts.tv_sec) * 1000 + ts.tv_nsec / 1000000;
}

Contexte *contexte_par_indice(int indice)
{
    if (indice < 0 || size_t(indice) >= g_contextes.size() || !g_contextes[indice]) {
        PyErr_SetString(PyExc_ValueError, "bojs: contexte inconnu");
        return nullptr;
    }
    return g_contextes[indice];
}

// --- Conversion JavaScript -> Python ----------------------------------------

PyObject *versPython(JSContext *ctx, JSValueConst valeur, int profondeur);

PyObject *chaineVersPython(JSContext *ctx, JSValueConst valeur)
{
    size_t taille = 0;
    const char *brut = JS_ToCStringLen(ctx, &taille, valeur);
    if (!brut)
        return nullptr;
    PyObject *resultat = PyUnicode_DecodeUTF8(brut, Py_ssize_t(taille), "replace");
    JS_FreeCString(ctx, brut);
    return resultat;
}

PyObject *tableauVersPython(JSContext *ctx, JSValueConst valeur, int profondeur)
{
    JSValue longueur = JS_GetPropertyStr(ctx, valeur, "length");
    uint32_t nombre = 0;
    JS_ToUint32(ctx, &nombre, longueur);
    JS_FreeValue(ctx, longueur);

    PyObject *liste = PyList_New(Py_ssize_t(nombre));
    if (!liste)
        return nullptr;
    for (uint32_t i = 0; i < nombre; ++i) {
        JSValue element = JS_GetPropertyUint32(ctx, valeur, i);
        PyObject *converti = versPython(ctx, element, profondeur + 1);
        JS_FreeValue(ctx, element);
        if (!converti) {
            Py_DECREF(liste);
            return nullptr;
        }
        PyList_SET_ITEM(liste, Py_ssize_t(i), converti);
    }
    return liste;
}

PyObject *objetVersPython(JSContext *ctx, JSValueConst valeur, int profondeur)
{
    JSPropertyEnum *proprietes = nullptr;
    uint32_t nombre = 0;
    if (JS_GetOwnPropertyNames(ctx, &proprietes, &nombre, valeur,
                               JS_GPN_STRING_MASK | JS_GPN_ENUM_ONLY) < 0)
        return PyDict_New();

    PyObject *dictionnaire = PyDict_New();
    if (!dictionnaire) {
        js_free(ctx, proprietes);
        return nullptr;
    }
    for (uint32_t i = 0; i < nombre; ++i) {
        JSValue element = JS_GetProperty(ctx, valeur, proprietes[i].atom);
        PyObject *converti = versPython(ctx, element, profondeur + 1);
        JS_FreeValue(ctx, element);
        const char *nom = JS_AtomToCString(ctx, proprietes[i].atom);
        if (converti && nom)
            PyDict_SetItemString(dictionnaire, nom, converti);
        Py_XDECREF(converti);
        if (nom)
            JS_FreeCString(ctx, nom);
        JS_FreeAtom(ctx, proprietes[i].atom);
    }
    js_free(ctx, proprietes);
    return dictionnaire;
}

PyObject *versPython(JSContext *ctx, JSValueConst valeur, int profondeur)
{
    if (profondeur > PROFONDEUR_MAX)
        Py_RETURN_NONE;

    switch (JS_VALUE_GET_NORM_TAG(valeur)) {
    case JS_TAG_UNDEFINED:
    case JS_TAG_NULL:
    case JS_TAG_UNINITIALIZED:
        Py_RETURN_NONE;
    case JS_TAG_BOOL:
        return PyBool_FromLong(JS_VALUE_GET_BOOL(valeur));
    case JS_TAG_INT:
        return PyLong_FromLong(JS_VALUE_GET_INT(valeur));
    case JS_TAG_FLOAT64: {
        const double d = JS_VALUE_GET_FLOAT64(valeur);
        return PyFloat_FromDouble(d);
    }
    case JS_TAG_STRING:
        return chaineVersPython(ctx, valeur);
    default:
        break;
    }

    if (JS_IsArray(ctx, valeur) > 0)
        return tableauVersPython(ctx, valeur, profondeur);
    if (JS_IsFunction(ctx, valeur))
        // Une fonction ne traverse pas : cote Python elle n'aurait aucun sens,
        // et la laisser filer masquerait l'erreur de conception a l'appelant.
        Py_RETURN_NONE;
    if (JS_IsObject(valeur))
        return objetVersPython(ctx, valeur, profondeur);

    // Nombres hors des etiquettes rapides (BigInt notamment).
    double nombre = 0;
    if (JS_ToFloat64(ctx, &nombre, valeur) == 0)
        return PyFloat_FromDouble(nombre);
    Py_RETURN_NONE;
}

// --- Conversion Python -> JavaScript ----------------------------------------

JSValue versJs(JSContext *ctx, PyObject *valeur, int profondeur)
{
    if (profondeur > PROFONDEUR_MAX || !valeur)
        return JS_UNDEFINED;
    if (valeur == Py_None)
        return JS_NULL;
    if (PyBool_Check(valeur))
        return JS_NewBool(ctx, valeur == Py_True);
    if (PyLong_Check(valeur)) {
        int debordement = 0;
        const long long entier = PyLong_AsLongLongAndOverflow(valeur, &debordement);
        if (debordement || PyErr_Occurred()) {
            PyErr_Clear();
            return JS_NewFloat64(ctx, PyLong_AsDouble(valeur));
        }
        return JS_NewInt64(ctx, int64_t(entier));
    }
    if (PyFloat_Check(valeur))
        return JS_NewFloat64(ctx, PyFloat_AsDouble(valeur));
    if (PyUnicode_Check(valeur)) {
        Py_ssize_t taille = 0;
        const char *brut = PyUnicode_AsUTF8AndSize(valeur, &taille);
        if (!brut) {
            PyErr_Clear();
            return JS_UNDEFINED;
        }
        return JS_NewStringLen(ctx, brut, size_t(taille));
    }
    if (PyBytes_Check(valeur))
        return JS_NewStringLen(ctx, PyBytes_AS_STRING(valeur),
                               size_t(PyBytes_GET_SIZE(valeur)));
    if (PyList_Check(valeur) || PyTuple_Check(valeur)) {
        PyObject *sequence = PySequence_Fast(valeur, "sequence attendue");
        if (!sequence)
            return JS_UNDEFINED;
        const Py_ssize_t nombre = PySequence_Fast_GET_SIZE(sequence);
        JSValue tableau = JS_NewArray(ctx);
        for (Py_ssize_t i = 0; i < nombre; ++i) {
            PyObject *element = PySequence_Fast_GET_ITEM(sequence, i); // empruntee
            JS_SetPropertyUint32(ctx, tableau, uint32_t(i),
                                 versJs(ctx, element, profondeur + 1));
        }
        Py_DECREF(sequence);
        return tableau;
    }
    if (PyDict_Check(valeur)) {
        JSValue objet = JS_NewObject(ctx);
        PyObject *cle = nullptr, *element = nullptr;
        Py_ssize_t position = 0;
        while (PyDict_Next(valeur, &position, &cle, &element)) {
            if (!PyUnicode_Check(cle))
                continue;
            const char *nom = PyUnicode_AsUTF8(cle);
            if (!nom) {
                PyErr_Clear();
                continue;
            }
            JS_SetPropertyStr(ctx, objet, nom, versJs(ctx, element, profondeur + 1));
        }
        return objet;
    }
    // Tout le reste est rendu par sa representation : mieux vaut une chaine
    // lisible dans la console qu'un `undefined` muet.
    PyObject *texte = PyObject_Str(valeur);
    if (!texte) {
        PyErr_Clear();
        return JS_UNDEFINED;
    }
    JSValue resultat = versJs(ctx, texte, profondeur + 1);
    Py_DECREF(texte);
    return resultat;
}

// --- Le passage unique ------------------------------------------------------

JSValue passage(JSContext *ctx, JSValueConst, int nombre, JSValueConst *arguments)
{
    Contexte *etat = static_cast<Contexte *>(JS_GetContextOpaque(ctx));
    if (!etat || !etat->pont)
        return JS_ThrowInternalError(ctx, "bojs: aucun pont installe");

    PyObject *liste = PyTuple_New(nombre);
    if (!liste)
        return JS_ThrowOutOfMemory(ctx);
    for (int i = 0; i < nombre; ++i) {
        PyObject *converti = versPython(ctx, arguments[i], 0);
        if (!converti) {
            PyErr_Clear();
            converti = Py_None;
            Py_INCREF(converti);
        }
        PyTuple_SET_ITEM(liste, i, converti);
    }

    PyObject *resultat = PyObject_CallObject(etat->pont, liste);
    Py_DECREF(liste);

    if (!resultat) {
        // L'exception Python devient une exception JavaScript : le script peut
        // la rattraper, et sinon elle remonte a `evalue` avec sa trace.
        PyObject *type = nullptr, *valeur = nullptr, *trace = nullptr;
        PyErr_Fetch(&type, &valeur, &trace);
        PyErr_NormalizeException(&type, &valeur, &trace);
        PyObject *texte = valeur ? PyObject_Str(valeur) : nullptr;
        const char *message = texte ? PyUnicode_AsUTF8(texte) : "erreur Python";
        JSValue exception = JS_ThrowInternalError(ctx, "%s", message ? message : "erreur");
        Py_XDECREF(texte);
        Py_XDECREF(type);
        Py_XDECREF(valeur);
        Py_XDECREF(trace);
        return exception;
    }

    JSValue converti = versJs(ctx, resultat, 0);
    Py_DECREF(resultat);
    return converti;
}

/// Gardien : arrete un script qui depasse son budget.
///
/// Sans lui, `while (true) {}` dans une page figerait la fenetre, et donc la
/// machine — il n'y a ni onglet a fermer ni gestionnaire de taches.
int gardien(JSRuntime *, void *opaque)
{
    Contexte *etat = static_cast<Contexte *>(opaque);
    if (!etat || etat->echeance == 0)
        return 0;
    if (maintenant_ms() > etat->echeance) {
        etat->interrompu = true;
        return 1;
    }
    return 0;
}

/// Transforme l'exception JavaScript courante en exception Python.
void remonteException(Contexte *etat)
{
    JSContext *ctx = etat->contexte;
    JSValue exception = JS_GetException(ctx);

    const char *message = JS_ToCString(ctx, exception);
    PyObject *texte = nullptr;

    JSValue pile = JS_GetPropertyStr(ctx, exception, "stack");
    const char *trace = JS_IsUndefined(pile) ? nullptr : JS_ToCString(ctx, pile);

    if (etat->interrompu) {
        texte = PyUnicode_FromFormat("script interrompu apres %ld ms", etat->budget);
    } else if (message && trace) {
        texte = PyUnicode_FromFormat("%s\n%s", message, trace);
    } else if (message) {
        texte = PyUnicode_FromString(message);
    } else {
        texte = PyUnicode_FromString("exception JavaScript");
    }

    if (trace)
        JS_FreeCString(ctx, trace);
    JS_FreeValue(ctx, pile);
    if (message)
        JS_FreeCString(ctx, message);
    JS_FreeValue(ctx, exception);

    if (texte) {
        PyErr_SetObject(bojs_erreur(), texte);
        Py_DECREF(texte);
    } else {
        PyErr_SetString(bojs_erreur(), "exception JavaScript");
    }
}

/// Vide la file des taches differees (resolution des promesses).
void pompeTaches(Contexte *etat)
{
    JSContext *courant = nullptr;
    for (;;) {
        const int reste = JS_ExecutePendingJob(etat->execution, &courant);
        if (reste <= 0) {
            if (reste < 0 && courant)
                // Une promesse rejetee sans gestionnaire : on vide l'exception,
                // elle a deja ete signalee au script.
                JS_FreeValue(courant, JS_GetException(courant));
            break;
        }
    }
}

// --- Methodes du module ------------------------------------------------------

PyObject *bojs_cree(PyObject *, PyObject *args)
{
    long budget = BUDGET_MS_DEFAUT;
    if (!PyArg_ParseTuple(args, "|l", &budget))
        return nullptr;

    Contexte *etat = new Contexte();
    etat->budget = budget > 0 ? budget : BUDGET_MS_DEFAUT;
    etat->execution = JS_NewRuntime();
    if (!etat->execution) {
        delete etat;
        return PyErr_NoMemory();
    }
    // Un plafond memoire : une page qui alloue sans fin doit echouer dans son
    // propre bac a sable, pas epuiser le tas de tout le processus.
    JS_SetMemoryLimit(etat->execution, 128 * 1024 * 1024);
    JS_SetMaxStackSize(etat->execution, 1024 * 1024);
    etat->contexte = JS_NewContext(etat->execution);
    if (!etat->contexte) {
        JS_FreeRuntime(etat->execution);
        delete etat;
        return PyErr_NoMemory();
    }
    JS_SetContextOpaque(etat->contexte, etat);
    JS_SetInterruptHandler(etat->execution, gardien, etat);

    // `__bo_appel` : l'unique porte vers Python.
    JSValue global = JS_GetGlobalObject(etat->contexte);
    JS_SetPropertyStr(etat->contexte, global, "__bo_appel",
                      JS_NewCFunction(etat->contexte, passage, "__bo_appel", 1));
    JS_FreeValue(etat->contexte, global);

    g_contextes.push_back(etat);
    return PyLong_FromLong(long(g_contextes.size() - 1));
}

PyObject *bojs_pont(PyObject *, PyObject *args)
{
    int indice;
    PyObject *fonction;
    if (!PyArg_ParseTuple(args, "iO", &indice, &fonction))
        return nullptr;
    Contexte *etat = contexte_par_indice(indice);
    if (!etat)
        return nullptr;
    if (!PyCallable_Check(fonction)) {
        PyErr_SetString(PyExc_TypeError, "bojs.pont attend un appelable");
        return nullptr;
    }
    Py_INCREF(fonction);
    Py_XDECREF(etat->pont);
    etat->pont = fonction;
    Py_RETURN_NONE;
}

PyObject *bojs_evalue(PyObject *, PyObject *args)
{
    int indice;
    const char *source;
    Py_ssize_t taille;
    const char *nom = "<page>";
    if (!PyArg_ParseTuple(args, "is#|s", &indice, &source, &taille, &nom))
        return nullptr;
    Contexte *etat = contexte_par_indice(indice);
    if (!etat)
        return nullptr;

    etat->interrompu = false;
    etat->echeance = maintenant_ms() + etat->budget;
    JSValue resultat = JS_Eval(etat->contexte, source, size_t(taille), nom,
                               JS_EVAL_TYPE_GLOBAL);
    if (JS_IsException(resultat)) {
        JS_FreeValue(etat->contexte, resultat);
        etat->echeance = 0;
        remonteException(etat);
        return nullptr;
    }
    pompeTaches(etat);
    etat->echeance = 0;

    PyObject *converti = versPython(etat->contexte, resultat, 0);
    JS_FreeValue(etat->contexte, resultat);
    return converti;
}

PyObject *bojs_appelle(PyObject *, PyObject *args)
{
    int indice;
    const char *nom;
    PyObject *arguments = nullptr;
    if (!PyArg_ParseTuple(args, "is|O", &indice, &nom, &arguments))
        return nullptr;
    Contexte *etat = contexte_par_indice(indice);
    if (!etat)
        return nullptr;

    JSContext *ctx = etat->contexte;
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue fonction = JS_GetPropertyStr(ctx, global, nom);
    if (!JS_IsFunction(ctx, fonction)) {
        JS_FreeValue(ctx, fonction);
        JS_FreeValue(ctx, global);
        // Absente n'est pas une erreur : le prelude peut ne pas l'avoir definie.
        Py_RETURN_NONE;
    }

    int nombre = 0;
    JSValue *convertis = nullptr;
    if (arguments && arguments != Py_None) {
        PyObject *sequence = PySequence_Fast(arguments, "sequence attendue");
        if (!sequence) {
            JS_FreeValue(ctx, fonction);
            JS_FreeValue(ctx, global);
            return nullptr;
        }
        nombre = int(PySequence_Fast_GET_SIZE(sequence));
        convertis = new JSValue[nombre > 0 ? nombre : 1];
        for (int i = 0; i < nombre; ++i)
            convertis[i] = versJs(ctx, PySequence_Fast_GET_ITEM(sequence, i), 0);
        Py_DECREF(sequence);
    }

    etat->interrompu = false;
    etat->echeance = maintenant_ms() + etat->budget;
    JSValue resultat = JS_Call(ctx, fonction, global, nombre, convertis);
    for (int i = 0; i < nombre; ++i)
        JS_FreeValue(ctx, convertis[i]);
    delete[] convertis;
    JS_FreeValue(ctx, fonction);
    JS_FreeValue(ctx, global);

    if (JS_IsException(resultat)) {
        JS_FreeValue(ctx, resultat);
        etat->echeance = 0;
        remonteException(etat);
        return nullptr;
    }
    pompeTaches(etat);
    etat->echeance = 0;

    PyObject *converti = versPython(ctx, resultat, 0);
    JS_FreeValue(ctx, resultat);
    return converti;
}

PyObject *bojs_pompe(PyObject *, PyObject *args)
{
    int indice;
    if (!PyArg_ParseTuple(args, "i", &indice))
        return nullptr;
    Contexte *etat = contexte_par_indice(indice);
    if (!etat)
        return nullptr;
    etat->echeance = maintenant_ms() + etat->budget;
    pompeTaches(etat);
    etat->echeance = 0;
    Py_RETURN_NONE;
}

PyObject *bojs_detruit(PyObject *, PyObject *args)
{
    int indice;
    if (!PyArg_ParseTuple(args, "i", &indice))
        return nullptr;
    Contexte *etat = contexte_par_indice(indice);
    if (!etat)
        return nullptr;
    Py_XDECREF(etat->pont);
    JS_FreeContext(etat->contexte);
    JS_FreeRuntime(etat->execution);
    delete etat;
    g_contextes[indice] = nullptr;
    Py_RETURN_NONE;
}

// `CONFIG_VERSION` n'est defini que pour les unites de QuickJS lui-meme ; la
// chaine est donc passee a la compilation de ce fichier, et vaut « inconnue »
// si personne ne l'a fournie.
#ifndef BOJS_VERSION_QUICKJS
#define BOJS_VERSION_QUICKJS "inconnue"
#endif

PyObject *bojs_version(PyObject *, PyObject *)
{
    return PyUnicode_FromString(BOJS_VERSION_QUICKJS);
}

PyMethodDef bojs_methodes[] = {
    {"cree", bojs_cree, METH_VARARGS,
     "cree(budget_ms=5000) -> contexte JavaScript neuf"},
    {"pont", bojs_pont, METH_VARARGS,
     "pont(ctx, fonction) : installe l'appelable derriere __bo_appel"},
    {"evalue", bojs_evalue, METH_VARARGS,
     "evalue(ctx, source, nom='<page>') -> valeur du dernier calcul"},
    {"appelle", bojs_appelle, METH_VARARGS,
     "appelle(ctx, nom_global, arguments=None) -> valeur rendue"},
    {"pompe", bojs_pompe, METH_VARARGS,
     "pompe(ctx) : execute les taches differees (promesses)"},
    {"detruit", bojs_detruit, METH_VARARGS, "detruit(ctx)"},
    {"version", bojs_version, METH_NOARGS, "version() -> version de QuickJS"},
    {nullptr, nullptr, 0, nullptr},
};

PyModuleDef bojs_module = {
    PyModuleDef_HEAD_INIT, "bojs",
    "Moteur JavaScript QuickJS, cote Python.",
    -1, bojs_methodes, nullptr, nullptr, nullptr, nullptr,
};

PyObject *g_erreur = nullptr;

} // namespace

PyObject *bojs_erreur()
{
    return g_erreur ? g_erreur : PyExc_RuntimeError;
}

PyObject *initialise_bojs()
{
    PyObject *module = PyModule_Create(&bojs_module);
    if (!module)
        return nullptr;
    g_erreur = PyErr_NewException("bojs.Erreur", nullptr, nullptr);
    if (g_erreur) {
        Py_INCREF(g_erreur);
        PyModule_AddObject(module, "Erreur", g_erreur);
    }
    return module;
}

#ifdef BOJS_MODULE_PARTAGE
// Compile a part, ce meme fichier donne une extension chargeable par le Python
// de la machine de developpement. C'est ce qui permet d'eprouver le pont et le
// prelude sans reconstruire le binaire de l'OS ni demarrer l'emulateur — voir
// `tools/userland/test-moteur.sh`.
extern "C" PyMODINIT_FUNC PyInit_bojs()
{
    return initialise_bojs();
}
#endif
