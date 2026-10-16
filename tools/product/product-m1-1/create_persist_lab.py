#!/usr/bin/env python3
"""Create the dedicated, disposable 128-MiB Bouchaud PERSIST test volume.

Never writes physical disks. Refuses overwrite. Works with existing Volume::DONNEES
when QEMU attaches this as the SECOND IDE disk. Does not alter the UEFI image.
"""
from __future__ import annotations
import argparse
from pathlib import Path
import hashlib
import json
import sys

ZONE_SECTORS=262144
ARCHIVE_PREFIX_SECTORS=1026
SECTOR=512
BYTES=(ZONE_SECTORS+ARCHIVE_PREFIX_SECTORS)*SECTOR

def create(repo:Path,force:bool=False)->Path:
    target=repo/'target'/'reference'/'qemu-data'/'m1-1-persist-lab.img'
    # Force is deliberately unsupported: only a deliberate manual path deletion
    # can reset a saved user session.
    if target.exists():raise FileExistsError(f'REFUSED_EXISTING_DATA: {target}')
    target.parent.mkdir(parents=True,exist_ok=True)
    with target.open('xb') as f:
        f.truncate(BYTES)
        f.flush()
    return target

def main()->int:
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--repo',type=Path,required=True)
    ap.add_argument('--create',action='store_true')
    a=ap.parse_args()
    target=a.repo.resolve()/'target'/'reference'/'qemu-data'/'m1-1-persist-lab.img'
    if a.create:
        try:create(a.repo.resolve())
        except FileExistsError as e:print(e,file=sys.stderr);return 2
    if not target.is_file():print('DATA_IMAGE_ABSENT '+str(target));return 3
    if target.stat().st_size!=BYTES:print('WRONG_IMAGE_SIZE',target.stat().st_size,BYTES);return 4
    print('BOUCHAUD_PERSIST_LAB_READY path='+str(target)+' bytes='+str(BYTES))
    print('SECOND_IDE_DATA_DISK_ONLY; NO_BOOT_IMAGE_MODIFICATION; NO_PHYSICAL_DEVICE_WRITES')
    return 0
if __name__=='__main__':sys.exit(main())
