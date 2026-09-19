#!/usr/bin/env python3
from __future__ import annotations
import argparse, csv, json, struct, zlib
from pathlib import Path

SECTOR = 512
RECORD = 4096
HEADER = 64
MAGIC = b"BOUBBX01"
PART_NAME = "BOUCHAUD-BLACKBOX"

KIND_NAMES = {1:"serial",2:"sample",3:"marker",4:"flight",5:"memory",6:"network",7:"service",8:"terminal",9:"fatal"}
EVENT_NAMES = {1:"timer-enter",2:"timer-exit",10:"gfx-enter",11:"gfx-exit"}

class LecteurBrut:
    """Lecture d'un disque brut, y compris un `\\\\.\\PhysicalDriveN` Windows.

    # Le defaut que ceci corrige

    L'extraction a echoue sur `OSError: [Errno 22] Invalid argument` en plein
    parcours des emplacements, apres avoir pourtant lu la table GPT. Windows
    n'autorise l'acces brut a un disque que par lectures ALIGNEES sur la taille
    de secteur PHYSIQUE, qui vaut 4096 sur les supports 4Kn -- et la partition
    BLACKBOX commence a un LBA dont l'octet de depart, 1 659 913 216, n'est pas
    multiple de 4096.

    Ce n'etait pas un detail d'outillage : c'est l'outil qui rend la trace
    lisible, et il venait de refuser la seule archive contenant une panique
    noyau. Un extracteur qui plante fait perdre exactement ce que l'
    enregistreur a passe quatre sessions a savoir garder.

    # Ce que cette classe garantit

      * l'alignement est DECOUVERT, pas suppose : 512 d'abord, 4096 ensuite ;
      * les lectures se font par blocs d'un mebioctet, gardes en cache -- le
        parcours est sequentiel, et huit mille lectures de quatre kibioctets
        deviennent trente-deux lectures ;
      * une zone illisible est SAUTEE et comptee, jamais levee. Une archive
        amputee vaut infiniment mieux qu'une exception.
    """

    BLOC = 1 << 20

    def __init__(self, fichier):
        self.f = fichier
        self.alignement = SECTOR
        self.cache_debut = None
        self.cache = b""
        self.zones_illisibles = 0
        self._decouvre_alignement()

    def _decouvre_alignement(self):
        """Le plus petit alignement que le disque accepte reellement."""
        for essai in (SECTOR, 4096):
            try:
                self.f.seek(0)
                if len(self.f.read(essai)) == essai:
                    self.alignement = essai
                    if essai != SECTOR:
                        print(f"BLACKBOX alignement={essai} (secteur physique)")
                    return
            except OSError:
                continue
        # Aucun des deux n'a repondu : on garde 512 et on laissera la lecture
        # signaler zone par zone, plutot que de refuser toute l'archive ici.
        print("BLACKBOX ATTENTION: alignement indetermine, lecture au mieux")

    def _charge(self, debut):
        """Charge le bloc alignes contenant `debut`. Rend False s'il resiste."""
        pas = max(self.alignement, SECTOR)
        aligne = (debut // pas) * pas
        if self.cache_debut == aligne:
            return True
        taille = self.BLOC
        while taille >= pas:
            try:
                self.f.seek(aligne)
                data = self.f.read(taille)
            except OSError:
                taille //= 2
                continue
            if not data:
                break
            self.cache_debut = aligne
            self.cache = data
            return True
        self.cache_debut = None
        self.cache = b""
        return False

    def lit(self, offset, size):
        """Rend `size` octets, ou `None` si la zone est illisible."""
        if offset < 0 or size < 0:
            raise ValueError("offset/size negatifs")
        if size == 0:
            return b""
        out = bytearray()
        reste = size
        curseur = offset
        while reste:
            if not self._charge(curseur):
                self.zones_illisibles += 1
                return None
            dans_bloc = curseur - self.cache_debut
            if dans_bloc >= len(self.cache):
                self.zones_illisibles += 1
                return None
            morceau = self.cache[dans_bloc:dans_bloc + reste]
            if not morceau:
                self.zones_illisibles += 1
                return None
            out.extend(morceau)
            curseur += len(morceau)
            reste -= len(morceau)
        return bytes(out)

    def exige(self, offset, size):
        """Comme `lit`, mais leve : reserve a la table GPT, sans laquelle il
        n'y a rien a extraire du tout."""
        data = self.lit(offset, size)
        if data is None or len(data) != size:
            raise RuntimeError(
                f"lecture impossible offset={offset} taille={size} "
                f"(alignement={self.alignement})"
            )
        return data

def parse_gpt_partition(lecteur):
    hdr = lecteur.exige(SECTOR, SECTOR)
    if hdr[:8] != b"EFI PART":
        raise RuntimeError("GPT primaire absente")
    entries_lba = struct.unpack_from("<Q", hdr, 72)[0]
    entries_count = struct.unpack_from("<I", hdr, 80)[0]
    entry_size = struct.unpack_from("<I", hdr, 84)[0]
    if entry_size < 128 or entry_size > 4096:
        raise RuntimeError(f"taille entree GPT invalide: {entry_size}")
    for i in range(entries_count):
        raw = lecteur.exige(entries_lba*SECTOR + i*entry_size, entry_size)
        if raw[:16] == b"\0"*16:
            continue
        first = struct.unpack_from("<Q", raw, 32)[0]
        last = struct.unpack_from("<Q", raw, 40)[0]
        name = raw[56:128].decode("utf-16-le", errors="ignore").split("\0",1)[0]
        if name == PART_NAME:
            if not first or last < first:
                raise RuntimeError("partition BLACKBOX GPT invalide")
            return first, last, name
    raise RuntimeError(f"partition GPT {PART_NAME!r} introuvable")

def parse_record(raw, slot):
    if len(raw) != RECORD or raw[:8] != MAGIC:
        return None
    version, kind = struct.unpack_from("<HH", raw, 8)
    header_len = struct.unpack_from("<I", raw, 12)[0]
    boot_id, seq, ts_ns = struct.unpack_from("<QQQ", raw, 16)
    payload_len, payload_crc = struct.unpack_from("<II", raw, 40)
    trace_end = struct.unpack_from("<Q", raw, 48)[0]
    flags = struct.unpack_from("<I", raw, 56)[0]
    if version != 1 or header_len != HEADER or payload_len > RECORD-HEADER:
        return None
    payload = raw[HEADER:HEADER+payload_len]
    if (zlib.crc32(payload) & 0xffffffff) != payload_crc:
        return None
    return dict(slot=slot, version=version, kind=kind, boot_id=boot_id, seq=seq,
                ts_ns=ts_ns, trace_end=trace_end, flags=flags, payload=payload)

def decode_flight(payload):
    rows=[]
    fmt="<QQHHIQ"
    size=struct.calcsize(fmt)
    for off in range(0, len(payload)-size+1, size):
        seq, ts_ns, kind, cpu, stage, arg = struct.unpack_from(fmt, payload, off)
        rows.append(dict(event_seq=seq, ts_ns=ts_ns, kind=kind,
                         kind_name=EVENT_NAMES.get(kind,f"event-{kind}"),
                         cpu=cpu, stage=stage, arg=arg, arg_hex=f"0x{arg:x}"))
    return rows

def session_dir_name(boot_id):
    s=str(boot_id)
    if len(s)>=17:
        base,suffix=s[:-3],s[-3:]
        if len(base)==14:
            return f"{base[:4]}-{base[4:6]}-{base[6:8]}_{base[8:10]}-{base[10:12]}-{base[12:14]}_{suffix}"
    return f"boot-{boot_id}"

def extract(source, output):
    output.mkdir(parents=True, exist_ok=True)
    with open(source, "rb", buffering=0) as f:
        lecteur=LecteurBrut(f)
        first,last,name=parse_gpt_partition(lecteur)
        part_offset=first*SECTOR
        part_bytes=(last-first+1)*SECTOR
        slots=part_bytes//RECORD
        print(f"BLACKBOX partition={name} first_lba={first} last_lba={last} bytes={part_bytes} slots={slots}")
        records=[]
        bad=0
        illisibles=0
        for slot in range(slots):
            raw=lecteur.lit(part_offset+slot*RECORD, RECORD)
            if raw is None:
                # UNE ZONE QUI RESISTE NE DOIT PAS EMPORTER L'ARCHIVE.
                #
                # C'est ce qui s'est passe le 17 septembre : une seule lecture
                # refusee par Windows, une exception, et la trace d'une panique
                # noyau perdue pour rien.
                illisibles+=1
                continue
            rec=parse_record(raw,slot)
            if rec is None:
                if raw[:8]==MAGIC: bad+=1
                continue
            records.append(rec)
        if illisibles:
            print(f"BLACKBOX ATTENTION: {illisibles} emplacement(s) illisible(s) sur {slots}")

    sessions={}
    for rec in records:
        sessions.setdefault(rec["boot_id"],[]).append(rec)

    manifest=dict(source=source, partition_first_lba=first, partition_last_lba=last,
                  partition_bytes=part_bytes, record_slots=slots,
                  valid_records=len(records), invalid_magic_or_crc_records=bad,
                  unreadable_slots=illisibles, sessions=[])

    for boot_id in sorted(sessions):
        recs=sorted(sessions[boot_id], key=lambda r:r["seq"])
        sdir=output/session_dir_name(boot_id)
        sdir.mkdir(parents=True, exist_ok=True)
        serial=bytearray(); samples=[]; memory=[]; markers=[]; fatal=[]; flight=[]; network=[]; services=[]; terminal=[]
        for r in recs:
            p=r["payload"]
            if r["kind"]==1: serial.extend(p)
            elif r["kind"]==2: samples.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==3: markers.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==4: flight.extend(decode_flight(p))
            elif r["kind"]==5: memory.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==6: network.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==7: services.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==8: terminal.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==9: fatal.append(p.decode("utf-8",errors="replace"))
        (sdir/"serial.log").write_bytes(serial)
        (sdir/"samples.log").write_text("".join(samples),encoding="utf-8")
        (sdir/"memory.log").write_text("".join(memory),encoding="utf-8")
        # L'ETAT DE LA CARTE, RELISIBLE APRES COUP.
        #
        # Le releve du 17 septembre a ete extrait sans une seule occurrence de
        # `rx_cur`, `chip_cmd` ou `xid` : l'instantane vivait dans `netetat`, et
        # nulle part dans ce qui se relit une fois la machine eteinte.
        (sdir/"network.log").write_text("".join(network),encoding="utf-8")
        # LES EVENEMENTS QUI PERMETTENT DE REJOUER UNE PANNE.
        #
        # Ethernet pret -> DHCP pret -> DNS pret -> RX degrade -> ARP en echec
        # -> navigation en echec. Cette suite-la est exactement ce qui
        # manquait, et elle doit se relire seule, sans etre noyee.
        (sdir/"services.log").write_text("".join(services),encoding="utf-8")
        (sdir/"terminal.log").write_text("".join(terminal),encoding="utf-8")
        (sdir/"markers.log").write_text("".join(markers),encoding="utf-8")
        (sdir/"fatal.log").write_text("".join(fatal),encoding="utf-8")
        with (sdir/"flight.csv").open("w",newline="",encoding="utf-8") as fp:
            fields=["event_seq","ts_ns","kind","kind_name","cpu","stage","arg","arg_hex"]
            w=csv.DictWriter(fp,fieldnames=fields); w.writeheader(); w.writerows(flight)
        rec_manifest=[dict(slot=r["slot"],kind=r["kind"],kind_name=KIND_NAMES.get(r["kind"],f"kind-{r['kind']}"),
                           seq=r["seq"],ts_ns=r["ts_ns"],trace_end=r["trace_end"],
                           payload_len=len(r["payload"])) for r in recs]
        (sdir/"records.json").write_text(json.dumps(rec_manifest,indent=2),encoding="utf-8")
        manifest["sessions"].append(dict(
            boot_id=boot_id,directory=sdir.name,records=len(recs),
            first_seq=recs[0]["seq"] if recs else None,last_seq=recs[-1]["seq"] if recs else None,
            fatal_records=sum(1 for r in recs if r["kind"]==9),
            network_records=len(network),
            service_records=len(services),
            terminal_records=len(terminal),
            latency_spikes=sum(1 for l in "".join(services).splitlines() if l.startswith("pic_reveil")),
            serial_bytes=len(serial),flight_events=len(flight),
            # LA VERSION DU NOYAU QUI A PRODUIT CETTE ARCHIVE.
            #
            # Le releve du 17 septembre a ete lu comme s'il venait du commit
            # qu'on croyait avoir flashe. Il venait d'un autre : ni `netetat`,
            # ni `xid`, ni `chip_cmd` n'existaient dans ce binaire, et il a
            # fallu compter les occurrences d'un champ pour s'en rendre compte.
            # Une archive doit DIRE de quel noyau elle sort.
            build=prochaine_marque(markers, "BOUCHAUD_BUILD"),
        ))
    manifest["sessions"].sort(key=lambda x:x["boot_id"])
    (output/"manifest.json").write_text(json.dumps(manifest,indent=2),encoding="utf-8")
    if manifest["sessions"]:
        latest=manifest["sessions"][-1]
        (output/"LATEST.txt").write_text(
            f"{latest['directory']}\nboot_id={latest['boot_id']}\nrecords={latest['records']}\n"
            f"fatal_records={latest['fatal_records']}\nserial_bytes={latest['serial_bytes']}\n"
            f"flight_events={latest['flight_events']}\n",encoding="utf-8")
        print(f"OK: {len(records)} records, {len(manifest['sessions'])} session(s), latest={latest['directory']}")
    else:
        print("ATTENTION: aucun record BLACKBOX valide trouve")

def prochaine_marque(markers, prefixe):
    """La premiere marque portant ce prefixe, ou None.

    Les marques sont des lignes completes ; on rend ce qui suit le prefixe,
    debarrasse de ses espaces, pour que le manifeste porte l'identite du
    binaire sans qu'on ait a ouvrir un fichier de plus.
    """
    for ligne in "".join(markers).splitlines():
        place = ligne.find(prefixe)
        if place >= 0:
            return ligne[place + len(prefixe):].strip()
    return None


def main():
    ap=argparse.ArgumentParser()
    g=ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--disk")
    g.add_argument("--image")
    ap.add_argument("--output",default="target/blackbox-extract")
    a=ap.parse_args()
    extract(a.disk or a.image,Path(a.output))

if __name__=="__main__":
    main()
