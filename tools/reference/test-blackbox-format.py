#!/usr/bin/env python3
import struct, zlib, importlib.util
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location("extract_blackbox", HERE/"extract-blackbox.py")
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

payload=b"blackbox-test\n"
raw=bytearray(m.RECORD)
raw[:8]=m.MAGIC
struct.pack_into("<HHI",raw,8,1,2,m.HEADER)
struct.pack_into("<QQQ",raw,16,20260909123456001,42,123456789)
struct.pack_into("<II",raw,40,len(payload),zlib.crc32(payload)&0xffffffff)
struct.pack_into("<QII",raw,48,777,0,0)
raw[m.HEADER:m.HEADER+len(payload)]=payload
rec=m.parse_record(bytes(raw),7)
assert rec and rec["kind"]==2 and rec["seq"]==42 and rec["payload"]==payload
raw[m.HEADER]^=1
assert m.parse_record(bytes(raw),7) is None
ev=struct.pack("<QQHHIQ",9,1000,1,0,1,0x80012ff162)
rows=m.decode_flight(ev)
assert rows[0]["kind_name"]=="timer-enter" and rows[0]["arg"]==0x80012ff162
print("BLACKBOX_FORMAT_TEST_OK")
