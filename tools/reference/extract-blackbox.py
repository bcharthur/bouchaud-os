#!/usr/bin/env python3
from __future__ import annotations
import argparse, csv, json, struct, zlib
from pathlib import Path

SECTOR = 512
RECORD = 4096
HEADER = 64
MAGIC = b"BOUBBX01"
PART_NAME = "BOUCHAUD-BLACKBOX"

KIND_NAMES = {1:"serial",2:"sample",3:"marker",4:"flight",5:"memory",9:"fatal"}
EVENT_NAMES = {1:"timer-enter",2:"timer-exit",10:"gfx-enter",11:"gfx-exit"}

def read_exact(f, offset, size):
    """Lecture brute alignee secteurs pour Windows PhysicalDrive."""
    if offset < 0 or size < 0:
        raise ValueError("offset/size negatifs")
    if size == 0:
        return b""

    aligned_start = (offset // SECTOR) * SECTOR
    aligned_end = ((offset + size + SECTOR - 1) // SECTOR) * SECTOR
    aligned_size = aligned_end - aligned_start

    f.seek(aligned_start)
    data = f.read(aligned_size)
    if len(data) != aligned_size:
        raise RuntimeError(
            f"lecture courte offset={aligned_start} wanted={aligned_size} got={len(data)}"
        )

    begin = offset - aligned_start
    return data[begin:begin + size]

def parse_gpt_partition(f):
    hdr = read_exact(f, SECTOR, SECTOR)
    if hdr[:8] != b"EFI PART":
        raise RuntimeError("GPT primaire absente")
    entries_lba = struct.unpack_from("<Q", hdr, 72)[0]
    entries_count = struct.unpack_from("<I", hdr, 80)[0]
    entry_size = struct.unpack_from("<I", hdr, 84)[0]
    if entry_size < 128 or entry_size > 4096:
        raise RuntimeError(f"taille entree GPT invalide: {entry_size}")
    for i in range(entries_count):
        raw = read_exact(f, entries_lba*SECTOR + i*entry_size, entry_size)
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
        first,last,name=parse_gpt_partition(f)
        part_offset=first*SECTOR
        part_bytes=(last-first+1)*SECTOR
        slots=part_bytes//RECORD
        print(f"BLACKBOX partition={name} first_lba={first} last_lba={last} bytes={part_bytes} slots={slots}")
        records=[]
        bad=0
        for slot in range(slots):
            raw=read_exact(f, part_offset+slot*RECORD, RECORD)
            rec=parse_record(raw,slot)
            if rec is None:
                if raw[:8]==MAGIC: bad+=1
                continue
            records.append(rec)

    sessions={}
    for rec in records:
        sessions.setdefault(rec["boot_id"],[]).append(rec)

    manifest=dict(source=source, partition_first_lba=first, partition_last_lba=last,
                  partition_bytes=part_bytes, record_slots=slots,
                  valid_records=len(records), invalid_magic_or_crc_records=bad, sessions=[])

    for boot_id in sorted(sessions):
        recs=sorted(sessions[boot_id], key=lambda r:r["seq"])
        sdir=output/session_dir_name(boot_id)
        sdir.mkdir(parents=True, exist_ok=True)
        serial=bytearray(); samples=[]; memory=[]; markers=[]; fatal=[]; flight=[]
        for r in recs:
            p=r["payload"]
            if r["kind"]==1: serial.extend(p)
            elif r["kind"]==2: samples.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==3: markers.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==4: flight.extend(decode_flight(p))
            elif r["kind"]==5: memory.append(p.decode("utf-8",errors="replace"))
            elif r["kind"]==9: fatal.append(p.decode("utf-8",errors="replace"))
        (sdir/"serial.log").write_bytes(serial)
        (sdir/"samples.log").write_text("".join(samples),encoding="utf-8")
        (sdir/"memory.log").write_text("".join(memory),encoding="utf-8")
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
            serial_bytes=len(serial),flight_events=len(flight)
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

