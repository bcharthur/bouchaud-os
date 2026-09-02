#!/usr/bin/env python3
"""Garde-fou structurel du RSS O(1) et des grappes de fautes groupees."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def source(rel: str) -> str:
    path = ROOT / rel
    if not path.is_file():
        raise SystemExit(f"rss-o1: fichier absent: {rel}")
    text = path.read_text(encoding="utf-8")
    if "\r" in text:
        raise SystemExit(f"rss-o1: CRLF interdit dans {rel}")
    return text


def body(text: str, signature: str) -> str:
    start = text.find(signature)
    if start < 0:
        raise SystemExit(f"rss-o1: fonction absente: {signature}")
    brace = text.find("{", start)
    depth = 0
    for pos in range(brace, len(text)):
        if text[pos] == "{":
            depth += 1
        elif text[pos] == "}":
            depth -= 1
            if depth == 0:
                return text[brace : pos + 1]
    raise SystemExit(f"rss-o1: fonction incomplete: {signature}")


virtual = source("src/kernel/memory/virtual.rs")
for token in (
    "PTE_RSS_KIND_MASK",
    "resident: ResidentStats",
    "pub enum ResidentKind",
    "self.resident.mapped",
    "self.resident.unmapped",
    "ADDR_MASK | PTE_RSS_KIND_MASK",
):
    if token not in virtual:
        raise SystemExit(f"rss-o1: contrat VMM absent: {token}")

snapshot = body(virtual, "pub fn resident_stats(&self)")
for forbidden in ("for ", "while ", "iter_user_pages", "table_at", "entry_mut"):
    if forbidden in snapshot:
        raise SystemExit(f"rss-o1: resident_stats n'est plus O(1): {forbidden}")
if "self.resident" not in snapshot:
    raise SystemExit("rss-o1: resident_stats ne rend pas le compteur incremental")

resource = source("src/kernel/process/resource.rs")
memory_usage = body(resource, "pub fn memory_usage(process: &Process)")
if "resident_stats()" not in memory_usage:
    raise SystemExit("rss-o1: Resource Core n'utilise pas l'instantane O(1)")
for forbidden in ("iter_user_pages", "octets_virtuels(&mm.promesses)"):
    if forbidden in memory_usage:
        raise SystemExit(f"rss-o1: travail non borne revenu sous Mm: {forbidden}")
if "(resident, promises) = {" not in memory_usage or "octets_virtuels(&promises)" not in memory_usage:
    raise SystemExit("rss-o1: le calcul VSS doit rester hors de la portee Mm")

metrics = source("src/kernel/process/thread/metriques.rs")
loop_start = metrics.find("for task in tasks().iter()")
none_start = metrics.find("None => {", loop_start)
usage_call = metrics.find("crate::kernel::resource::memory_usage(process)", loop_start)
if min(loop_start, none_start, usage_call) < 0 or usage_call < none_start:
    raise SystemExit("rss-o1: le RSS doit etre lu dans la branche nouveau PID")
if "[MM-RSS-O1]" not in metrics:
    raise SystemExit("rss-o1: preuve runtime absente")

fault = source("src/kernel/process/thread/faute_memoire.rs")
for kind in ("Anonymous", "FilePrivate", "Shared", "Device"):
    if f"ResidentKind::{kind}" not in fault:
        raise SystemExit(f"rss-o1: classe de fault absente: {kind}")

cluster = source("src/kernel/process/thread/faute_cluster.rs")
for token in (
    "FAULT_CLUSTER_MM_LOCKS",
    "let mut ready = Vec::with_capacity",
    "let mut mm = processus.mm.lock();",
    "for key in releases",
    "ZERO_CLUSTER_MAX_PAGES: u64 = 32",
    "memory_pressure::Level::Critical => 2",
):
    if token not in cluster:
        raise SystemExit(f"rss-o1: contrat de grappe absent: {token}")
cluster_body = body(cluster, "fn fault_cluster_after_clean(")
if cluster_body.count("processus.mm.lock()") != 1:
    raise SystemExit("rss-o1: la grappe fichier doit prendre Mm exactement une fois")

print("ok  RSS incremental O(1), un releve par processus, publication groupee sous une prise Mm")
