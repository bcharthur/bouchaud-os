# Sécurité

Rôles principaux : UI/Broker, Renderer/Content, Worker, Network, ImageDecoder,
Compositor et Media selon le chemin.

`no_new_privs`, profils et tests négatifs empêchent les escalades implicites par
exec.

La sécurité inclut le recovery : une panne doit rester confinée, classée et
bornée.
