# Graphisme, input, audio

Skia peint actuellement en CPU.

P10 a seulement franchi :
PCI → virtio moderne → control queue → `GET_DISPLAY_INFO`.

Pas encore de virtio-gpu 2D complet, 3D, Vulkan, Skia GPU ou pilote AMD.

Les latences HID sont mesurées pour attribuer scheduler, I/O, raster ou
instrumentation.

Audio QEMU : AC'97 + `/dev/dsp`.
Audio physique HDA : non terminé.
