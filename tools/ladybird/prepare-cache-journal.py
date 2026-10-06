#!/usr/bin/env python3
"""Le cache HTTP disque dit ce qu'il fait : MISS, STORE, HIT, REVALIDATE,
INVALIDATE.

BOUCHAUD_CACHE_JOURNAL_V1 (convergence P2)

`disk_cache=enabled` ne prouve rien : il faut voir qu'une ressource est
ECRITE dans le cache, puis RELUE depuis lui apres un redemarrage complet du
navigateur, sans nouveau telechargement. LibHTTP a deja les bons points
d'observation, mais derriere `HTTP_DISK_CACHE_DEBUG` (compile a 0). Ce
preparateur ajoute, a cote de chacun, une ligne `[LB]` courte et toujours
active -- une par evenement de cache, pas par octet :

  [LB] MISS url=...                     aucune entree
  [LB] STORE url=... bytes=...          entree completement ecrite
  [LB] HIT url=... bytes=... age_s=...  entree fraiche servie depuis le disque
  [LB] REVALIDATE url=... age_s=...     entree perimee : requete conditionnelle
  [LB] REVALIDATED url=...              304 : l'entree est reutilisee
  [LB] INVALIDATE url=... raison=...    entree abandonnee (revalidation ratee)
  [LB] INVALIDATE key=...               entree supprimee du disque

Les URL sont tronquees a 160 caracteres. Rien d'autre ne change : memes
decisions de cache, memes chemins. Ancres strictes, fail-closed, idempotent.
"""
import sys
from pathlib import Path

MARQUEUR = "BOUCHAUD_CACHE_JOURNAL_V1"


def ajoute_apres(chemin: Path, ancre: str, ajout: str) -> None:
    texte = chemin.read_text(encoding="utf-8")
    if ancre + ajout in texte:
        return
    if texte.count(ancre) != 1:
        raise SystemExit(f"cache journal : ancre introuvable ou ambigue dans {chemin} :\n{ancre}")
    chemin.write_text(texte.replace(ancre, ancre + ajout, 1), encoding="utf-8")


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: prepare-cache-journal.py <arbre-ladybird>", file=sys.stderr)
        return 2
    cache = Path(sys.argv[1]).resolve() / "Libraries/LibHTTP/Cache"
    disque = cache / "DiskCache.cpp"
    entree = cache / "CacheEntry.cpp"

    ajoute_apres(
        disque,
        '        dbgln_if(HTTP_DISK_CACHE_DEBUG, "\\033[36m[disk]\\033[0m \\033[35;1mNo cache entry for\\033[0m {}", url);\n',
        f'        dbgln("[LB] MISS url={{}}", lb_url_courte(url)); // {MARQUEUR}\n',
    )
    ajoute_apres(
        disque,
        '            dbgln_if(HTTP_DISK_CACHE_DEBUG, "\\033[36m[disk]\\033[0m \\033[32;1mOpened cache entry for\\033[0m {} (lifetime={}s age={}s) ({} bytes)", url, freshness_lifetime.to_seconds(), current_age.to_seconds(), index_entry->data_size);\n',
        f'            dbgln("[LB] HIT url={{}} bytes={{}} age_s={{}}", lb_url_courte(url), index_entry->data_size, current_age.to_seconds()); // {MARQUEUR}\n',
    )
    ajoute_apres(
        disque,
        '        dbgln_if(HTTP_DISK_CACHE_DEBUG, "\\033[36m[disk]\\033[0m \\033[36;1mMust revalidate cache entry for\\033[0m {} (lifetime={}s age={}s)", url, freshness_lifetime.to_seconds(), current_age.to_seconds());\n',
        f'        dbgln("[LB] REVALIDATE url={{}} age_s={{}}", lb_url_courte(url), current_age.to_seconds()); // {MARQUEUR}\n',
    )
    ajoute_apres(
        disque,
        "void DiskCache::delete_entry(u64 cache_key, u64 vary_key)\n{\n",
        f'    dbgln("[LB] INVALIDATE key={{}} vary={{}}", cache_key, vary_key); // {MARQUEUR}\n',
    )
    # Une fonction locale : tronquer l'URL sans dupliquer l'expression.
    texte = disque.read_text(encoding="utf-8")
    if "lb_url_courte" in texte and "lb_url_courte(URL::URL const& url)" not in texte:
        ancre = "static constexpr StringView cache_directory_for_mode(DiskCache::Mode mode)\n"
        if texte.count(ancre) != 1:
            raise SystemExit("cache journal : ancre de la fonction locale introuvable")
        aide = (
            f"// {MARQUEUR}\n"
            "static ByteString lb_url_courte(URL::URL const& url)\n"
            "{\n"
            "    auto serialized = url.serialize().to_byte_string();\n"
            "    return serialized.length() <= 160 ? serialized : serialized.substring(0, 160);\n"
            "}\n\n"
        )
        disque.write_text(texte.replace(ancre, aide + ancre, 1), encoding="utf-8")

    ajoute_apres(
        entree,
        '    dbgln_if(HTTP_DISK_CACHE_DEBUG, "\\033[36m[disk]\\033[0m \\033[34;1mFinished caching\\033[0m {} ({} bytes)", m_url, m_cache_footer.data_size);\n',
        f'    dbgln("[LB] STORE url={{}} bytes={{}}", lb_tronque(m_url.bytes_as_string_view()), m_cache_footer.data_size); // {MARQUEUR}\n',
    )
    ajoute_apres(
        entree,
        '    dbgln_if(HTTP_DISK_CACHE_DEBUG, "\\033[36m[disk]\\033[0m \\033[34;1mCache revalidation succeeded for\\033[0m {}", m_url);\n',
        f'    dbgln("[LB] REVALIDATED url={{}}", lb_tronque(m_url.bytes_as_string_view())); // {MARQUEUR}\n',
    )
    ajoute_apres(
        entree,
        '    dbgln_if(HTTP_DISK_CACHE_DEBUG, "\\033[36m[disk]\\033[0m \\033[33;1mCache revalidation failed for\\033[0m {}", m_url);\n',
        f'    dbgln("[LB] INVALIDATE url={{}} raison=revalidation", lb_tronque(m_url.bytes_as_string_view())); // {MARQUEUR}\n',
    )
    texte = entree.read_text(encoding="utf-8")
    if "static StringView lb_tronque" not in texte:
        ancre = "namespace HTTP {\n"
        if texte.count(ancre) != 1:
            raise SystemExit("cache journal : namespace HTTP introuvable dans CacheEntry.cpp")
        aide = (
            ancre + "\n"
            f"// {MARQUEUR}\n"
            "static StringView lb_tronque(StringView url)\n"
            "{\n"
            "    return url.substring_view(0, min<size_t>(160, url.length()));\n"
            "}\n"
        )
        entree.write_text(texte.replace(ancre, aide, 1), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
