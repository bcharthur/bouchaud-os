#!/usr/bin/env python3
"""Témoin déterministe du changement de frame entre la copie et la prise."""

import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent


def fonction(texte, signature):
    debut = texte.index(signature)
    return texte[debut:texte.index("\n}", debut) + 2]


def execute(code):
    with tempfile.TemporaryDirectory() as temp:
        source = pathlib.Path(temp) / "temoin.rs"
        binaire = pathlib.Path(temp) / "temoin"
        source.write_text(code)
        subprocess.run(["rustc", "--edition=2021", str(source), "-o", str(binaire)], check=True)
        return subprocess.run([str(binaire)], capture_output=True)


def main():
    cache = (ROOT / "src/kernel/memory/page_cache.rs").read_text()
    retenir = fonction(cache, "pub fn retain_frame(")
    partage = (ROOT / "src/kernel/memory/shared.rs").read_text()
    retenir_partage = fonction(partage, "pub fn retient_mappage(").replace("CACHE", "SHARED")
    fork = fonction((ROOT / "src/compat/linux/proc.rs").read_text(), "pub fn sys_fork_demande(")
    indices = [fork.index(x) for x in (
        "parent.mm.lock()", "mm.space.duplicate()", "space.translate(mapping.virt)",
        "retain_frame(mapping.key, frame)", "retient_mappage(plage.node)",
        "invalidation.execute()", "let apres_references_ns", "drop(space)",
        "release(mapping.key)", "demappe(plage.node)", "construit_fils(",
    )]
    assert indices == sorted(indices), "ordre protection, acquisition, rollback ou publication"
    assert "in &clean_pages[..propres_prises]" in fork
    assert "in &partages[..partages_pris]" in fork
    fixture = r'''
use std::collections::BTreeMap;
use std::sync::Mutex;
type Key = u32;
#[derive(Clone, Copy)]
enum State { Present { frame: u64, mappings: usize } }
struct Entry { state: Mutex<State> }
struct Cache { entrees: BTreeMap<Key, Entry> }
struct Shared { node: usize, mappages: usize, evicting: bool }
static CACHE: Mutex<Cache> = Mutex::new(Cache { entrees: BTreeMap::new() });
static SHARED: Mutex<Vec<Shared>> = Mutex::new(Vec::new());
fn cesse_d_etre_recuperable() {}
fn remplit(key: Key, frame: u64, mappings: usize) {
    CACHE.lock().unwrap().entrees.insert(key, Entry {
        state: Mutex::new(State::Present {frame, mappings})
    });
}
fn compte(key: Key) -> usize {
    let cache = CACHE.lock().unwrap();
    let State::Present { mappings, .. } = *cache.entrees[&key].state.lock().unwrap();
    mappings
}
fn main() {
    remplit(1, 0x1000, 1);
    remplit(2, 0x2000, 1);
    // Le fils a copie deux PTE. L'ancienne entree n'a plus de reference,
    // est evincee, puis la cle est rechargee sur une autre frame.
    remplit(2, 0x2000, 0);
    CACHE.lock().unwrap().entrees.remove(&2);
    remplit(2, 0x3000, 1);
    let lot = [(1, 0x1000), (2, 0x2000)];
    let prises = lot.iter().take_while(|&&(key, frame)| retain_frame(key, frame)).count();
    assert_eq!(prises, 1, "substitution acceptee");
    assert_eq!(compte(2), 1);
    // Compense uniquement le prefixe acquis, comme le chemin ENOMEM.
    for &(key, _) in &lot[..prises] {
        let cache = CACHE.lock().unwrap();
        let mut state = cache.entrees[&key].state.lock().unwrap();
        let State::Present { frame, mappings } = *state;
        *state = State::Present {frame, mappings: mappings - 1};
    }
    assert_eq!(compte(1), 1);
    assert!(retain_frame(2, 0x3000));
    assert_eq!(compte(2), 2);
    assert!(!retient_mappage(7));
    SHARED.lock().unwrap().push(Shared { node: 7, mappages: 0, evicting: true });
    assert!(!retient_mappage(7));
    {
        let mut shared = SHARED.lock().unwrap();
        shared[0].evicting = false;
        assert_eq!(shared[0].mappages, 0);
        shared[0].mappages = 1;
    }
    assert!(retient_mappage(7));
    assert_eq!(SHARED.lock().unwrap()[0].mappages, 2);
}
'''
    def fixture_avec(prise_cache):
        corps = (prise_cache + retenir_partage).replace(".lock()", ".lock().unwrap()")
        return "#![allow(irrefutable_let_patterns)]\n" + corps + fixture

    corrige = execute(fixture_avec(retenir))
    assert corrige.returncode == 0, corrige.stderr.decode()
    condition = "if frame != attendue { return false; }"
    assert condition in retenir
    ancien = execute(fixture_avec(retenir.replace(condition, "").replace("attendue: u64", "_attendue: u64")))
    assert ancien.returncode != 0 and b"substitution acceptee" in ancien.stderr
    print("FORK_REFERENCES_OK identite rollback partage mutant=refuse")


if __name__ == "__main__":
    main()
