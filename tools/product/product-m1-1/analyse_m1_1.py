#!/usr/bin/env python3
"""Strict Product M1.1 serial log gate with explicit unproven outcomes.

Usage: python analyse_m1_1.py logs/run.log [--json out.json]
Use baseline and candidate logs to compare; never call a log without a
reached marker a successful test. Does not infer smoothness from FPS alone.
"""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import re
from collections import Counter

CSI=re.compile(r'\x1b\[[0-9;?]*[ -/]*[@-~]')

def val(line:str,key:str):
    m=re.search(r'(?<!\w)'+re.escape(key)+r'=([^\s]+)',line)
    return m.group(1) if m else None

def analyze(content:str)->dict:
    data=CSI.sub('',content)
    lines=data.splitlines()
    def filt(needle):return [x for x in lines if needle in x]
    def unique(needle):return bool(filt(needle))
    def max_num(needle,key):
        vals=[]
        for l in filt(needle):
            t=val(l,key)
            if t is not None:
                try: vals.append(int(t))
                except ValueError:pass
        return max(vals) if vals else None
    def last(needle,key):
        matches=filt(needle)
        return val(matches[-1],key) if matches else None
    notes=filt('BOUCHAUD_PRODUCT_NOTES_SAVED')
    durable=[x for x in notes if val(x,'durable') in {'true','1'}]
    browser_complete=filt('[LB:NAV]')
    docs=[x for x in browser_complete if 'document_charge' in x]
    panics=[x for x in lines if ('panicked at ' in x or 'KERNEL_FAULT' in x or 'BOUCHAUD_TAS_OOM' in x)]
    acpi=filt('BOUCHAUD_ACPI_S5')
    return {
       'schema':'BOUCHAUD_M1_1_GATE_V1','lines':len(lines),
       'boot_verified':unique('BOUCHAUD_UEFI_ENTRY_OK'),
       'desktop_ready':unique('BOUCHAUD_STAGE2_WINDOW_MANAGER_READY'),
       'usb_mouse_events':unique('BOUCHAUD_HID_MOUSE_INPUT_GREEN'),
       'usb_keyboard_events':unique('BOUCHAUD_HID_KEYBOARD_INPUT_GREEN'),
       'click_dispatch_count':len(filt('BOUCHAUD_STAGE2_CLICK_DISPATCHED')),
       'notes_save_count':len(notes),'notes_durable_ack_count':len(durable),
       'browser_launched':unique('BOUCHAUD_STAGE2_LADYBIRD_LAUNCHED'),
       'loaded_documents':sorted(set(val(x,'url') for x in docs if val(x,'url'))),
       'browser_pagefaults_interval':max_num('[PERF-BROWSER]','pf_delta'),
       'hid_latency_spikes':len(filt('HID_LATENCY_SPIKE')),
       'hid_latency_max_us':max_num('HID_LATENCY_SPIKE','delta_us'),
       'hid_stall_count':len(filt('BOUCHAUD_HID_STALL_CONFIRMED')),
       'hid_rearmed_count':len(filt('BOUCHAUD_HID_REARMED')),
       'scheduler_interactive_p99_ns':last('[SCHED-NG-CENTILES] classe=interactive','p99_ns'),
       'scheduler_normal_p99_ns':last('[SCHED-NG-CENTILES] classe=normale','p99_ns'),
       'gui_last_present_calls':last('[GUI-PRESENT]','present_calls'),
       'gui_last_lfb_pixels':last('[GUI-PRESENT]','lfb_pixels'),
       'acpi_fadt_length':last('BOUCHAUD_M11_ACPI_FADT_LENGTH','length'),
       'acpi_ready':any('etat=pret' in x for x in acpi),
       'acpi_failure':next((x[x.find('BOUCHAUD_ACPI_S5'):] for x in acpi if 'etat=absent' in x),None),
       'timebase_incoherent':unique('BOUCHAUD_HORLOGE_INCOHERENTE'),
       'persist_volume_detected':unique('persistance: zone vierge') or unique('persistance:') and not unique('persistance: aucune zone'),
       'shutdown_requested':unique('BOUCHAUD_SYSTEM_EXIT'),
       'panic_lines':panics[:15],
       'verdict': 'INCOMPLETE_OR_UNPROVEN',
    }

def main():
    p=argparse.ArgumentParser();p.add_argument('log',type=Path);p.add_argument('--json',type=Path);p.add_argument('--compare',type=Path);a=p.parse_args()
    report=analyze(a.log.read_text(errors='replace'))
    if a.compare:
        baseline=analyze(a.compare.read_text(errors='replace'))
        report['comparison_baseline']=str(a.compare)
        report['comparison']={
         'hid_spikes_diff':report['hid_latency_spikes']-baseline['hid_latency_spikes'],
         'hid_stalls_diff':report['hid_stall_count']-baseline['hid_stall_count'],
         'browser_pagefault_peak_diff':(report['browser_pagefaults_interval'] or 0)-(baseline['browser_pagefaults_interval'] or 0),
         'disclaimer':'Run duration, workload, acceleration and clock MUST match before interpreting deltas.'
        }
    # Only declare done with proofs, not mere ability to construct a disk.
    if (report['boot_verified'] and report['desktop_ready'] and report['usb_mouse_events']
        and report['usb_keyboard_events'] and report['browser_launched']
        and report['notes_durable_ack_count'] and report['acpi_ready'] and not report['panic_lines']):
        report['verdict']='M1_1_CORE_GATES_REACHED_NOT_ENDURANCE'
    payload=json.dumps(report,indent=2,ensure_ascii=False)
    print(payload)
    if a.json:
        a.json.parent.mkdir(parents=True,exist_ok=True);a.json.write_text(payload+'\n',encoding='utf-8')
    return 0
if __name__=='__main__':raise SystemExit(main())
