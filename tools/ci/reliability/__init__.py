"""Bouchaud OS reliability helpers.

Pure-host tooling used by GitHub Actions and by local validation.  Nothing here
is a substitute for a kernel runtime test: the tools launch QEMU, scan the
actual serial journal and fail closed on kernel-fatal markers.
"""
