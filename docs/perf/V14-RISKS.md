# V14 — Risques et garde-fous

Le changement le plus sensible est le clustered paging. Il reste borné à huit
pages et uniquement sur un VMA `File` read-only, disk-backed et non chevauché.
Chaque frame acquise est soit publiée avec sa référence `clean_pages`, soit
relâchée immédiatement. Le `MappingToken` est revalidé sous `Mm` avant mapping.

MUNMAP/MADVISE sont libérés du BKL parce que leurs dépendances actuelles ont
déjà des domaines SMP : `Mm`, clean cache, shared cache et protocole TLB. Le
writeback RAMFS de shared cache prend déjà son BKL interne.

Le profil WHPX SMP4 reste expérimental : l'historique du runner a observé une
découverte APIC incomplète avec WHPX multi-vCPU. V14 ne le rend pas par défaut.
