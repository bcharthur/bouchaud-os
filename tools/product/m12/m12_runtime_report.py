#!/usr/bin/env python3
"""Diagnostic M1.2: analyse locale de logs QEMU sans faux vert, baseline comparable.
Entrées: fichiers .log produits par le lanceur Windows; pas de réseau.
"""
import argparse, collections, datetime, json, pathlib, re, statistics, sys

ANSI=re.compile(r'\x1b\[[0-9;]*[A-Za-z]')
CLOCK=re.compile(r'\[(\d\d):(\d\d):(\d\d)\]')
US=re.compile(r'\bdelta_us=(\d+)')
NOTES=re.compile(r'BOUCHAUD_PRODUCT_NOTES_SAVED[^\n]*\bdurable=(\w+)')
FPS=re.compile(r'\[FPS:\s*([0-9]+)\]')
MARKERS={
 'uefi':'BOUCHAUD_UEFI_ENTRY_OK',
 'ramdisk':'BOUCHAUD_TRIGKEY_LADYBIRD_RAMDISK_OK',
 'arch_p1_runtime': 'NATIVE_IPC_RING3_OK',  # sonde distincte, pas journal desktop
 'm11':'BOUCHAUD_PRODUCT_M11_ACTIVE',
 'acpi_ready':'BOUCHAUD_ACPI_S5 etat=pret',
 'shutdown_requested':'BOUCHAUD_SYSTEM_EXIT',
 'shutdown_after_finish':'BOUCHAUD_M12_SHUTDOWN_STAGE after_finish',
 'shutdown_emulator_boundary':'BOUCHAUD_M12_SHUTDOWN_STAGE before_emulator_ports',
 'acpi_attempt':'BOUCHAUD_M12_ACPI_S5_ATTEMPT',
 'acpi_returned':'BOUCHAUD_M12_ACPI_S5_RETURNED',
 'm12_clock_quality':'BOUCHAUD_M12_CLOCK_QUALITY',
 'm12_hid_quota':'[M12-HID-DIAG]',
 'm12_gui_aggregation':'[M12-GUI-COMPOSITION]',
 'acpi_port_write':'BOUCHAUD_ACPI_S5 extinction',
 'panic':'KERNEL PANIC',
 'double_fault':'DOUBLE FAULT',
 'hid_stall':'BOUCHAUD_HID_STALL_CONFIRMED',
 'hid_rearmed':'BOUCHAUD_HID_REARMED',
 'hid_interrupt_back':'BOUCHAUD_HID_INTERRUPT_BACK',
 'hid_latency':'HID_LATENCY_SPIKE',
 'sched_dump':'[SCHED-DUMP]',
 'sched_task':'[SCHED-TACHE]',
 'gui_composition_old':'GUI_COMPOSITION_NAVIGATEUR',
 'document_charge':'document_charge',
 'browser_child_exit':'LADYBIRD_CHILD_EXIT',
 'no_persist':'persistance: aucune zone',
 'user_note_save':'BOUCHAUD_PRODUCT_NOTES_SAVED',
 'time_incoherent':'BOUCHAUD_HORLOGE_INCOHERENTE',
 'duplicate_ladybird':'[ladybird-bouchaud]',
 'memory_oom':'BOUCHAUD_TAS_OOM',
}

def time_window(lines):
    first=last=None; totalday=0; last_sec=None
    for line in lines:
        m=CLOCK.search(line)
        if not m:continue
        val=int(m[1])*3600+int(m[2])*60+int(m[3])
        if last_sec is not None and val+totalday<last_sec-12*3600:totalday+=86400
        if first is None:first=val
        last=val+totalday
        last_sec=last
    return 0 if first is None or last is None else max(0,last-first)

def analyze(path):
    raw=path.read_text('utf-8',errors='replace')
    lines=[ANSI.sub('',line) for line in raw.splitlines()]
    counts={k:sum(marker in line for line in lines) for k,marker in MARKERS.items()}
    spikes=[int(m[1]) for line in lines if 'HID_LATENCY_SPIKE' in line for m in [US.search(line)] if m]
    notes=NOTES.findall(raw)
    eligible=[line for line in lines if '[ladybird-bouchaud]' in line]
    duplicated=sum(1 for a,b in zip(eligible,eligible[1:]) if a==b)
    # Réponses HTTP, pas des garanties de rendu.
    statuses=collections.Counter(int(m[1]) for s in lines for m in [re.search(r'M9_RS_HEADERS.*?statut=(\d+)',s)] if m)
    secs=time_window(lines)
    stats={
      'path':str(path),'bytes':path.stat().st_size,'lines':len(lines),'duration_seconds':secs,
      'counts':counts,
      'latency_spikes_us':{'count':len(spikes),'max':max(spikes) if spikes else None,
          'median':statistics.median(spikes) if spikes else None,
          'p95':sorted(spikes)[min(len(spikes)-1,max(0,int(len(spikes)*0.95)-1))] if spikes else None,
          'per_min':round(len(spikes)*60/secs,2) if secs else None},
      'ladybird_adjacent_exact_duplicates':duplicated,
      'http_header_raw_counts':dict(sorted(statuses.items())),
      'notes_durable_values':notes,
      'notes_durable_observed':bool(notes and any(n.lower() in ('1','true','yes') for n in notes)),
      'shutdown_evidence':shutdown_verdict(counts),
      'runtime_persistence_proved':False,  # nécessite même document après DEUX boots distincts
      'ipc_negative_proved':False,  # nécessite sonde ring3 échec copie, pas le test positif
    }
    return stats

def shutdown_verdict(c):
    if not c['shutdown_requested']:return 'NOT_REQUESTED'
    if not c['shutdown_after_finish']:return 'UNKNOWN_BEFORE_FINISH_BOUNDARY'
    if not c['shutdown_emulator_boundary']:return 'BLOCKED_BETWEEN_FINISH_AND_EMULATOR'
    if c['acpi_returned']:return 'ACPI_RETURNED_WITH_POWER_ON'
    if c['acpi_attempt']:return 'ACPI_ATTEMPTED_NO_POWER_CONFIRMATION'
    return 'EMULATOR_REQUEST_SENT_OR_PROCESS_EXITED_UNPROVEN'

def compare(before,after):
    # Même architecture/hyperviseur/durée/scénario indispensable.
    results={}
    for key in ('hid_latency','sched_dump','sched_task','gui_composition_old','hid_stall'):
        old=before['counts'][key];new=after['counts'][key]
        otime=before['duration_seconds'];ntime=after['duration_seconds']
        old_rate=old*60/otime if otime else None
        new_rate=new*60/ntime if ntime else None
        results[key]={'before_events':old,'after_events':new,
            'before_per_min':round(old_rate,3) if old_rate is not None else None,
            'after_per_min':round(new_rate,3) if new_rate is not None else None,
            'relative_rate_percent':round((new_rate/old_rate-1)*100,1) if old_rate and new_rate is not None else None}
    return results

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--log',required=True,type=pathlib.Path)
    p.add_argument('--baseline',type=pathlib.Path,help='Journal anterieur, meme scenario')
    p.add_argument('--json',type=pathlib.Path,help='Ecrit le rapport (pas le log sensible)')
    x=p.parse_args()
    if not x.log.is_file():p.error('journal absent')
    report={'current':analyze(x.log)}
    if x.baseline:
        if not x.baseline.is_file():p.error('baseline absente')
        report['baseline']=analyze(x.baseline)
        report['comparison']=compare(report['baseline'],report['current'])
        report['comparison_is_qualified']=False
        report['comparison_note']='A confirmer: meme QEMU, acceleration, CPU, sites, duree et actions.'
    if x.json:
        x.json.parent.mkdir(parents=True,exist_ok=True)
        x.json.write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
        print('M12_REPORT_JSON='+str(x.json))
    c=report['current'];print('M12_REPORT_OK lines={} seconds={} bytes={}'.format(c['lines'],c['duration_seconds'],c['bytes']))
    print('M12_RUNTIME: shutdown={} acpi_attempt={} notes_durable={} persistent_reboot=UNPROVEN ipc_negative=UNPROVEN'.format(
        c['shutdown_evidence'],c['counts']['acpi_attempt'],c['notes_durable_observed']))
    print('M12_PERF: hid_spikes={} hid_per_min={} max_us={} sched_dumps={} sched_tasks={} gui_old={} gui_new={} duplicated_browser={}'.format(
        c['counts']['hid_latency'],c['latency_spikes_us']['per_min'],c['latency_spikes_us']['max'],
        c['counts']['sched_dump'],c['counts']['sched_task'],c['counts']['gui_composition_old'],c['counts']['m12_gui_aggregation'],c['ladybird_adjacent_exact_duplicates']))
    if c['counts']['panic'] or c['counts']['double_fault']:
        print('M12_FATAL_PRESENT; do not qualify as green')
    if x.baseline:
        for k,v in report['comparison'].items():print('M12_COMPARE {} old/min={} new/min={} delta_pct={}'.format(k,v['before_per_min'],v['after_per_min'],v['relative_rate_percent']))
    print('M12_TRUTH: no automatic P0/P1/P2 completion claim. Read JSON.')

if __name__=='__main__':main()
