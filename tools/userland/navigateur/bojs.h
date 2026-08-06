// Module Python `bojs` : le moteur JavaScript, vu depuis Python.
//
// `hote.cpp` l'enregistre par `PyImport_AppendInittab` au meme titre que `bo` —
// un binaire statique ne pouvant charger aucune extension, les deux modules
// sont lies en dur.

#pragma once

#include <Python.h>

/// Cree le module `bojs`. A passer a `PyImport_AppendInittab`.
PyObject *initialise_bojs();

/// Type d'exception leve quand un script echoue (`bojs.Erreur`).
PyObject *bojs_erreur();
