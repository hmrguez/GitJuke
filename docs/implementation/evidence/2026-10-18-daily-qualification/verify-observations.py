"""Verify captured observations, not an unattended replay of the native UI."""
import hashlib,json,pathlib
p=pathlib.Path(__file__).resolve().parent
checks=0
def read(name):return (p/name).read_text()
def obj(name):return json.loads(read(name+'.json'))
def check(ok,label):
 global checks
 assert ok,label
 checks+=1
def seen(name):return {k.rsplit(':',1)[-1]:v for k,v in obj(name)['seen'].items() if ':local-' in k}
def metric(name):
 for line in read(name).splitlines():
  if 'text "' in line and '\\"viewport\\"' in line:
   return json.loads(json.JSONDecoder().raw_decode(line.split('text ',1)[1])[0])
 raise AssertionError(name+' has no evaluated metric result')
initial=seen('initial');check(len(initial)==13 and not any(initial.values()),'thirteen initially unseen sources')
short=dict(initial,**{'local-short':1});seg=dict(short,**{'local-segments':1});final=dict(seg,**{'local-empty':1})
for stage in ['short-opened','segments-partial','segments-second']:check(seen(stage)==short,stage+' exact partial receipt map')
for stage in ['segments-complete-oversized','before-final-restart','after-final-restart','stable-retry-locked']:check(seen(stage)==seg,stage+' preserves exact completed identities')
for stage in ['final-reader','final-storage']:check(seen(stage)==final,stage+' complete textless review only')
check(obj('before-final-restart')['queries']==obj('after-final-restart')['queries']==obj('final-storage')['queries'],'query configuration survives all restarts')
for stage in ['initial','before-final-restart','after-final-restart','stable-retry-locked','final-storage']:check(obj(stage)['paused'],stage+' retains Pause')
check(obj('resumed')['paused'] is False,'explicit Resume was exercised')
check(obj('paused-observation')['noAutomaticReads'] and obj('paused-observation')['seenUnchanged'],'12-second Pause observation')
check(obj('initial')['logReads']==0 and obj('workflow-mid')['logReads']==0,'no log read before explicit action')
check(obj('before-final-restart')['logReads']==obj('after-final-restart')['logReads']==1,'restart does not replay logs')
check(obj('final-storage')['logReads']==2,'two explicit native log reads only')
check(obj('final-storage')['pendingLogs']==0 and not obj('final-storage')['logCanaryPersisted'],'no synthetic log canary or descriptor persisted')
check('Read Actions log' in read('native-log-return-ax.txt') and 'container Log text' not in read('native-log-return-ax.txt'),'navigation releases log')
check('Complete supplied change hunks' in read('final-native-changes-ax.txt') and '403 rows' in read('final-native-changes-ax.txt'),'final artifact complete patch')
check('50 loaded / 55 known' in read('final-native-conversation-ax.txt') and '5 loaded / 55 known' in read('native-conversation-last-ax.txt'),'bounded native source paging')
check('Overview is from old head aaaaaaaaaa. Current head: cccccccccc.' in read('native-stale-head-ax.txt'),'stale head is labeled')
a=obj('before-close');b=obj('after-close');c=obj('mixed-launches')
check(a['afterPids']==b['afterPids']==c['afterPids'] and len(a['afterPids'])==1,'close/reopen retain one owner')
check(['PR_1','selected'] in a['watchReasons'] and ['PR_1','selected'] not in b['watchReasons'] and ['PR_1','authored'] in b['watchReasons'],'close removes only selected watch reason')
check(c['contenderExits']==[0]*8,'eight mixed contenders forward or opt out')
check(obj('opted-out-background')['backgroundExit']==0 and not obj('opted-out-background')['afterPids'],'background opt-out exits')
r=obj('recovery-stable-before');repair=obj('recovery-stable-repaired');owner=obj('stable-retry-owner')
check(repair['filesUnchanged'] and repair['helperReadsBefore']==repair['helperReadsAfter'],'unrepaired Retry preserves bytes and stops reads')
check(owner['afterPids']==[r['pid']],'successful Retry retains the same background owner')
check(obj('stable-retry-locked')['helperReads']==r['helperReadsBefore'],'Retry does not bypass identity validation')
check('Validate your GitHub connection' in read('native-stable-retry-success-ax.txt'),'successful Retry remains identity locked')
check(obj('recovery-stable-exit')['exit']==0,'retained recovery owner quits cleanly')
for name,size,font,nav in [('final-native-metrics-100-ax.txt','12px','13px',162),('final-native-metrics-200-ax.txt','24px','26px',259)]:
 m=metric(name);check(m['viewport']==[960,640],name+' measured content viewport');check(m['root']==size and m['body']==font,name+' actual text scale');check(m['nav']==[nav,nav] and m['inlineMin']=='',name+' source CSS without inline probe');check(m['reduced'] and m['transition']=='0s',name+' system reduced motion')
check('focused UI element is 15 button Preferences' in read('final-native-preferences-focus-ax.txt'),'Preferences focus returns')
check('focused UI element is 13 button New query' in read('final-native-query-focus-ax.txt'),'query focus returns')
check('focused UI element is 39 button Enable notifications' in read('final-native-monitoring-keyboard-200-ax.txt'),'keyboard reaches offscreen alert action without invocation')
check('Not requested. Alerts disabled' in read('final-native-monitoring-200-ax.txt'),'actual permission wire state renders')
check('Value: off' in read('system-motion-restored.txt') and 'Value: off' in read('system-voiceover-restored.txt'),'temporary system preferences restored')
check(obj('voiceover-process')['runningProcesses']==[],'VoiceOver prerequisite failure recorded without a pass claim')
normal=obj('normal-final');check(normal['choiceFieldsUnchanged'] and normal['pauseUnchanged'] and normal['normalLoginUnchanged'],'normal choices and login unchanged')
cleanup=obj('final-cleanup');check(not cleanup['afterPids'] and not cleanup['loginPreferenceExists'] and not cleanup['launchAgentExists'],'isolated process and temporary login intent cleaned up')
source=json.loads(read('source-final.json'));check(all(pathlib.Path(k).is_file() and hashlib.sha256(pathlib.Path(k).read_bytes()).hexdigest()==v for k,v in source.items()),'final source hashes unchanged')
for name,a in obj('artifacts-final').items():
 if isinstance(a,dict) and 'executableSha256' in a:check(hashlib.sha256((pathlib.Path(a['path'])/'Contents/MacOS/gitjuke').read_bytes()).hexdigest()==a['executableSha256'],name+' final executable hash')
print(json.dumps({'observationChecks':checks,'unattendedUiRerun':False,'scope':'captured synthetic-local native WebKit/IPC/worker/SQLite plus filesystem/process metadata','voiceOver':'Blocked: process did not start','referencePerformance':'Blocked: required M1/8-GB/macOS-15 reference unavailable','normalCleanOwnerSmoke':'unqualified: existing tauri dev owner preserved','groupedReleaseScenariosPromoted':0},indent=2))
