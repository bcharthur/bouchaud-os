// Module Python `bomedia` : le decodage audio et video, vu depuis Python.
//
// `hote.cpp` l'enregistre par `PyImport_AppendInittab` au meme titre que `bo` et
// `bojs` — un binaire statique ne pouvant charger aucune extension, les modules
// sont lies en dur.

#pragma once

#include <Python.h>

/// Cree le module `bomedia`. A passer a `PyImport_AppendInittab`.
PyObject *initialise_bomedia();

/// Type d'exception leve quand un flux est illisible (`bomedia.Erreur`).
PyObject *bomedia_erreur();
