# Maintainer tool: used for releases and proofs, not needed to contribute.
"""Render Settings in the isolated offscreen application, never on the desktop."""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import argparse,json,os,subprocess,sys,shutil,tempfile
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'artifacts/settings-build'
STATE=ROOT/'target/settings-build-render'
EXE=Path(os.environ.get('CARGO_TARGET_DIR',str(ROOT/'target')))/'x86_64-pc-windows-gnu/debug/sidenote.exe'
sys.path.insert(0,str(ROOT/'tools'))
from gnu_toolchain import gnu_bin
from prepare_people_fixture import prepare
PAGES=[('general','settings-general',900),('meetings-recording','settings-recording',2200),('meetings-summaries','settings-summaries',1200),('meetings-tasks','settings-tasks',900),('meetings-engine','settings-engine',4600),('meetings-engine-invalid','settings-engine-invalid',4600),('meetings-engine-notes-error','settings-engine-notes-error',4600),('prompt-library','settings-prompts',1800),('prompts-edited','settings-prompts-edited',900),('prompts-diff','settings-prompts-diff',900),('prompts-sort','settings-prompts-sort',900),('prompts-redo','settings-prompts-redo',900),('prompts-conflict','settings-prompts-conflict',900),('prompts-readonly','settings-prompts-readonly',900),('people','settings-people',1800),('people-external','people-external',1800),('hex-dictation','settings-hex',1800),('models-agents','settings-helpers',3200),('models-speech','settings-speech',1400),('connected-apps','settings-mcp-mixed',2200),('appearance','settings-appearance',900),('sync','settings-sync',900),('privacy-data','settings-privacy',1800),('updates','settings-updates',900),('diagnostics','settings-diagnostics',1800)]

PAGES.append(('experimental', 'settings-experimental', 900))

def render(job):
 name,scene,width,theme,height,events=job
 tool_path=os.pathsep.join((str(gnu_bin()),os.environ.get('PATH','')))
 base=OUT/f'settings-{name}-{width}-{theme}'
 state=STATE/base.name
 for d in ('appdata','localappdata','peers'):(state/d).mkdir(parents=True,exist_ok=True)
 fixture=json.loads((ROOT/'tools/ui-fixtures/hex.json').read_text(encoding='utf-8'))
 fixture['settings']+=json.loads((ROOT/'tools/ui-fixtures/settings-recording.json').read_text(encoding='utf-8'))
 (state/'hex-fixture.json').write_text(json.dumps(fixture),encoding='utf-8')
 features=dict(tasks=True,memory=True,daily_check=True,projects=True)
 if scene in ('settings-experimental', 'sidebar-experiments-off'):
  features=dict(tasks=True,people=False,projects=False,memory=False,annotations=False,indic_engine=False,translation=False)
 (state/'config.json').write_text(json.dumps(dict(version=1,features=features)),encoding='utf-8')
 (state/'projects.json').write_text(json.dumps(dict(projects=['Northwind Atlas','Northwind Field Guide'],aliases={})),encoding='utf-8')
 meetings=prepare(state)
 if scene=='memory':
  from prepare_memory_fixture import prepare as prepare_memory
  meetings=prepare_memory(state)
 if scene.startswith('screens-'):
  meetings=state/'screen-meetings'
  shutil.copytree(ROOT/'tools/ui-fixtures/screens/meetings',meetings,dirs_exist_ok=True)
  summary=meetings/'1793955600000-1/english-summary.json'
  if '-tabs-' in name:
   # The older tabbed meeting layout (off by default) is a saved preference.
   (meetings/'local').mkdir(parents=True,exist_ok=True)
   (meetings/'local'/'ui.json').write_text(json.dumps({'meeting_tabs':True}),encoding='utf-8')
  if name.startswith(('meeting-lift-', 'meeting-refine-')):
   jobs=state/'local/notes-jobs'
   jobs.mkdir(parents=True)
   (jobs/'1793955600000-1.json').write_text(json.dumps({
    'state':'done','fallback':False,'tasks_failed':True,'transcript_revision':'a'*64,
    'routing_fallbacks':[{'reason':{'cpu_language_models_missing':{'ready':2}}}],
   }),encoding='utf-8')
   if '-clean-' in name:
    (jobs/'1793955600000-1.json').unlink()
   if '-banner-' in name:
    (jobs/'1793955600000-1.json').write_text(json.dumps({'state':'done','fallback':True,'tasks_failed':False,'transcript_revision':'a'*64,'routing_fallbacks':[]}),encoding='utf-8')
   if name.startswith('meeting-refine-p3-'):
    if '-confirmed-' in name:
     projects=state/'projects.json'
     data=json.loads(projects.read_text(encoding='utf-8'))
     data['meeting_projects']={summary.parent.name:'Northwind Atlas'}
     projects.write_text(json.dumps(data),encoding='utf-8')
    elif '-unassigned-' not in name:
     (summary.parent/'tasks.json').write_text('{"meeting_project":"Northwind Atlas"}',encoding='utf-8')
  else:
   value=json.loads(summary.read_text(encoding='utf-8'));value['title']='Planning the next Northwind workshop, partner review, delivery timeline and follow-up decisions for the whole team'
   summary.write_text(json.dumps(value),encoding='utf-8')
 env={key: value for key, value in os.environ.items() if not key.startswith('HEX_MEETING_')}
 env.update({'SIDENOTE_DATA_DIR':str(state),'XDG_CONFIG_HOME':str(state/'config'),'XDG_DATA_HOME':str(state/'data'),'XDG_STATE_HOME':str(state/'xdg-state'),'XDG_CACHE_HOME':str(state/'cache')})
 env|={'PATH':tool_path,'HOME':str(state/'home'),'USERPROFILE':str(state/'home'),'CODEX_HOME':str(state/'codex'),'APPDATA':str(state/'appdata'),'LOCALAPPDATA':str(state/'localappdata'),'SIDENOTE_STATE_DIR':str(state)}
 args=[str(EXE),'--meetings-dir',str(meetings),'--data-dir',str(state),'--peers-dir',str(state/'peers'),'--scene',scene,'--theme',theme,'--size',f'{width}x{height}','--scale','1','--snap',str(base)+'.png','--tree',str(base)+'.json']
 if events:
  script=state/'events.json';script.write_text(json.dumps(dict(events=events+[dict(t=max(e['t'] for e in events)+1,type='mark',name='end')])),encoding='utf-8')
  args+=['--script',str(script),'--at','mark:end']
 with (state/'render.log').open('w') as log:
  result=subprocess.run(args,cwd=ROOT,env=env,stdout=log,stderr=log,creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),timeout=180)
 assert result.returncode==0,(base.name,(state/'render.log').read_text()[-2000:])
 assert base.with_suffix('.png').is_file(),base
 tree=json.loads(base.with_suffix('.json').read_text(encoding='utf-8-sig'))
 assert not tree['warnings'],(base.name,tree['warnings'])
 if scene == 'settings-general':
  name_field = next(element for element in tree['elements'] if element.get('key') == 'notes-name')
  name_height = name_field['rect'][3]
  assert name_height == 28, f"{base.name}: Your name height {name_height:g}, expected 28"
 if name.startswith('meeting-lift-after'):
  elements={e['key']:e for e in tree['elements'] if e.get('key')}
  for key in ('meeting-view','meeting-facts','meeting-people','meeting-status','suggest-0','ask-pill'):
   x,y,w,h=elements[key]['rect']
   assert w>0 and h>0 and x>=0 and x+w<=width and y>=0 and y+h<=height,(base.name,key)
  suggestion=elements['suggest-0']['rect']
  assert suggestion[1]+suggestion[3]<=elements['ask-pill']['rect'][1],base.name
  if width==390:
   assert elements['ask-input']['rect'][2]>=92.0,base.name
  assert elements['meeting-source-pt-0-1']['opacity']==int(name.endswith('-time')),base.name
  assert elements['help-hint-model-tip']['opacity']==int(name.endswith('-hint')),base.name
  assert ('meeting-engine-status' in elements)==name.endswith('-status'),base.name
 if name.startswith('meeting-refine-p6-') and name.endswith('-after'):
  rects={e['key']:e['rect'] for e in tree['elements'] if e.get('key')}
  if width>=1334:
   strip,main,tasks=rects['meeting-grid-strip'],rects['meeting-main'],rects['meeting-tasks-column']
   numbers=dict(strip=strip[2],column=main[2],gap=tasks[0]-(main[0]+main[2]),tasks=tasks[2],header=rects['meeting-header'][2],ask=rects['ask-pill'][2],fade=rects['meeting-scroll-fade'][3]+rects['ask-overlay'][3])
   print(base.name,numbers,flush=True)
   assert numbers==dict(strip=44,column=640,gap=40,tasks=300,header=640,ask=640,fade=150),(base.name,numbers)
  else:
   # 1024 keeps the strip and the notes column; the tasks sit under the notes, not beside them.
   assert ('meeting-grid' in rects)==(width==1024) and 'meeting-tasks-column' not in rects,base.name
  for key,(x,y,w,h) in rects.items():
   if key.startswith('meeting-'):assert x>=0 and x+w<=width,(base.name,key,x,w)
 if name.startswith('meeting-refine-p8-') and name.endswith('-after'):
  rects={e['key']:e['rect'] for e in tree['elements'] if e.get('key')}
  # Nothing of the meeting page runs past the window, on either side.
  for key,(x,y,w,h) in rects.items():
   if key.startswith(('meeting-','more-meeting-')):assert x>=0 and x+w<=width,(base.name,key,x,w)
  facts=('meeting-view','meeting-facts','meeting-people','meeting-title')
  assert all(key in rects for key in facts),(base.name,[key for key in facts if key not in rects])
  assert any(key.startswith('more-meeting-') and key.endswith('-button') for key in rects),base.name
  numbers=dict(title=rects['meeting-title'][:4],header=rects['meeting-header'][:4],view=rects['meeting-view'][:4])
  if width==390:
   # Phone: People is in the facts row above the view switch (was below it before S4).
   assert rects['meeting-people'][1]<rects['meeting-view'][1],(base.name,numbers)
   assert 'meeting-sections-slot' in rects,base.name
   print(base.name,numbers,flush=True)
  if width==1024:
   main,strip=rects['meeting-main'],rects['meeting-grid-strip']
   numbers=dict(strip=strip[2],column=main[2],header=(rects['meeting-header'][0]-main[0],rects['meeting-header'][2]))
   print(base.name,numbers,flush=True)
   assert numbers==dict(strip=44,column=640,header=(0,640)),(base.name,numbers)
   assert 'meeting-tasks-column' not in rects,base.name
 if scene.startswith('settings-engine'):
  keys={n.get('key') for n in tree['elements']}
  need={'engine-unused','engine-worker-threads-input','engine-transcription-save','engine-model-te-input','engine-model-hi-value','engine-notes-writer-status','engine-notes-writer-change','engine-notes-model-status','engine-step-models-save','engine-step-tasks-switch','engine-step-comparison-switch','engine-import-button','engine-prompt-notes-input','engine-prompt-notes-reset','engine-prompt-translation-reset'}
  assert 'engine-model-hi-input' not in keys,'A routed language must be read only'
  text=[n.get('text','') for n in tree['elements']]
  assert text.count('These saved choices do not affect the built-in engine yet.')==1
  assert not any('Default value' in t or ' ? valid' in t or ' ? not set' in t for t in text)
  paths={n.get('key'):n.get('path','') for n in tree['elements']}
  rects={n.get('key'):n.get('rect') for n in tree['elements']}
  end=rects['engine-import'];assert end[1]+end[3]<=height,(base.name,'The Engine tab runs past the render',end,height)
  assert 'notes-model-card' in paths.get('engine-notes-model-status',''),'The Summary model status must sit inside its card'
  if scene=='settings-engine-notes-error':assert 'notes-model-card' in paths.get('engine-notes-error',''),'A failed Summary model save must show its error in the card'
  assert need<=keys,(base.name,sorted(need-keys))
 if scene=='people-suggestion':
  elements=tree['elements']
  assert any(n.get('text')=='Suggested: Our team' for n in elements), 'Suggestion missing'
  assert any(n.get('key')=='person-confirm-jamie' for n in elements), 'Confirm missing'
 print(base.name,flush=True)
 return os.path.relpath(base.with_suffix('.png'), ROOT)

def tap(key,t=.3):return dict(type='tap',key=key,t=t)

def meeting_lift(phase, output, exe):
 global OUT, STATE, EXE
 OUT=output.resolve()
 OUT.mkdir(parents=True,exist_ok=True)
 if exe:
  EXE=exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-meeting-render-') as state:
  STATE=Path(state)
  for width in (1334,390):
   for theme in ('light','dark'):
    render((f'meeting-lift-{phase}','screens-summary',width,theme,900,None))
  if phase=='after':
   for suffix,event in (
    ('status',tap('meeting-status')),
    ('time',dict(type='move',key='meeting-point-text-pt-0-1',t=.3)),
    ('hint',dict(type='move',key='help-hint-model',t=.3)),
   ):
    render((f'meeting-lift-{phase}-{suffix}','screens-summary',1334,'dark',900,[event]))


def refine_p1(phase, output, exe):
 global OUT, STATE, EXE
 OUT = output.resolve()
 OUT.mkdir(parents=True, exist_ok=True)
 if exe:
  EXE = exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-refine-render-') as state:
  STATE = Path(state)
  for width, theme in ((1334, 'dark'), (1334, 'light'), (390, 'dark')):
   events = None
   if width == 1334 and theme == 'dark':
    events = [dict(type='move', key='meeting-point-text-pt-0-1', t=.3)]
   render((f'meeting-refine-{phase}', 'screens-summary', width, theme, 900, events))
  for name, scene in (('tasks', 'tasks'), ('memory', 'memory'), ('diary', 'diary'), ('general', 'settings-general')):
   for width in (1280, 390):
    render((f'refine-{phase}-{name}', scene, width, 'dark', 900, None))


def refine_p3(phase, output, exe):
 global OUT, STATE, EXE
 OUT = output.resolve()
 OUT.mkdir(parents=True, exist_ok=True)
 if exe:
  EXE = exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-meta-render-') as state:
  STATE = Path(state)
  for width, theme in ((1334, 'dark'), (1334, 'light'), (390, 'dark')):
   render((f'meeting-refine-p3-meta-{phase}', 'screens-summary', width, theme, 900, None))
   render((f'meeting-refine-p3-popover-{phase}', 'screens-summary', width, theme, 900, [tap('meeting-status')]))
  if phase == 'after':
   for project in ('confirmed', 'unassigned'):
    render((f'meeting-refine-p3-{project}-{phase}', 'screens-summary', 1334, 'dark', 900, None))


def refine_p4(phase, output, exe):
 global OUT, STATE, EXE
 OUT = output.resolve()
 OUT.mkdir(parents=True, exist_ok=True)
 if exe:
  EXE = exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-notes-render-') as state:
  STATE = Path(state)
  for width, theme in ((1334, 'dark'), (1334, 'light'), (390, 'dark')):
   render((f'meeting-refine-p4-notes-{phase}', 'screens-summary', width, theme, 900, None))
  hover = [dict(type='move', key='meeting-point-text-pt-0-1', t=.3)]
  render((f'meeting-refine-p4-hover-{phase}', 'screens-summary', 1334, 'dark', 900, hover))


def refine_p5(phase, output, exe):
 global OUT, STATE, EXE
 OUT = output.resolve()
 OUT.mkdir(parents=True, exist_ok=True)
 if exe:
  EXE = exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-tasks-render-') as state:
  STATE = Path(state)
  for width, theme in ((1334, 'dark'), (1334, 'light'), (390, 'dark')):
   # Normal, then with the failed-sort notice under the Tasks label.
   for scene in ('clean', 'retry'):
    # At 390 the tasks sit below the notes, so the picture is taller.
    render((f'meeting-refine-p5-{scene}-{phase}', 'screens-summary', width, theme, 2400 if width == 390 else 900, None))


def refine_p6(phase, output, exe):
 global OUT, STATE, EXE
 OUT = output.resolve()
 OUT.mkdir(parents=True, exist_ok=True)
 if exe:
  EXE = exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-grid-render-') as state:
  STATE = Path(state)
  # The grid at three desktop and phone sizes, and 1024 where the tasks sit under the notes.
  # "banner" is a bordered notes banner under the last row. Tall renders show the stacked tasks.
  for scene in ('clean', 'banner'):
   for width, theme, height in ((1334, 'dark', 900), (1334, 'light', 900), (390, 'dark', 2400), (1024, 'dark', 1600)):
    render((f'meeting-refine-p6-{scene}-{phase}', 'screens-summary', width, theme, height, None))


def refine_p7(phase, output, exe):
 global OUT, STATE, EXE
 OUT = output.resolve()
 OUT.mkdir(parents=True, exist_ok=True)
 if exe:
  EXE = exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-recording-render-') as state:
  STATE = Path(state)
  for width, theme in ((1334, 'dark'), (1334, 'light'), (390, 'dark')):
   # The meeting open while it records, then after a pause.
   for scene in ('recording', 'paused'):
    render((f'meeting-refine-p7-{scene}-{phase}', f'screens-{scene}', width, theme, 900, None))


def refine_p8(phase, output, exe):
 global OUT, STATE, EXE
 OUT = output.resolve()
 OUT.mkdir(parents=True, exist_ok=True)
 if exe:
  EXE = exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-narrow-render-') as state:
  STATE = Path(state)
  # The narrow layouts: the phone, then 1024 where the tasks sit under the notes. "clean" has no
  # failed job, "retry" has the tasks notice, "recording-clean" is the meeting while it records.
  for scene, kind in (('clean', 'screens-summary'), ('retry', 'screens-summary'), ('recording-clean', 'screens-recording')):
   for width, theme, height in ((390, 'dark', 2400), (390, 'light', 2400), (1024, 'dark', 1600)):
    render((f'meeting-refine-p8-{scene}-{phase}', kind, width, theme, height, None))


def refine_p8_pages(output, exe):
 global OUT, STATE, EXE
 OUT = output.resolve()
 OUT.mkdir(parents=True, exist_ok=True)
 if exe:
  EXE = exe.resolve()
 with tempfile.TemporaryDirectory(prefix='sidenote-pages-render-') as state:
  STATE = Path(state)
  # Pages that share the type roles and the switch the meeting page work changed, and the older
  # tabbed meeting layout, which has its own header.
  for width in (1280, 390):
   for name, scene in (('people-team', 'people-our-team'), ('people-external', 'people-external'), ('companies', 'company-overview'), ('board', 'tasks-v2-board')):
    render((f'refine-p8-{name}', scene, width, 'dark', 900, None))
   for scene in ('clean', 'retry'):
    render((f'meeting-refine-p8-tabs-{scene}-pages', 'screens-summary', width, 'dark', 1600 if width == 390 else 900, None))


def main():
 global OUT, EXE
 parser=argparse.ArgumentParser();parser.add_argument('--cloud-ask',action='store_true');parser.add_argument('--quick',action='store_true');parser.add_argument('--only');parser.add_argument('--search-keys',action='store_true')
 parser.add_argument('--channel', choices=('stable','dev'), default='dev')
 modes=parser.add_mutually_exclusive_group()
 modes.add_argument('--meeting-lift',choices=['before','after'])
 modes.add_argument('--refine-p1',choices=['before','after'])
 modes.add_argument('--refine-p3',choices=['before','after'])
 modes.add_argument('--refine-p4',choices=['before','after'])
 modes.add_argument('--refine-p5',choices=['before','after'])
 modes.add_argument('--refine-p6',choices=['before','after'])
 modes.add_argument('--refine-p7',choices=['before','after'])
 modes.add_argument('--refine-p8',choices=['before','after'])
 modes.add_argument('--refine-p8-pages',action='store_true')
 parser.add_argument('--output',type=Path,default=OUT)
 parser.add_argument('--exe',type=Path)
 args=parser.parse_args()
 if args.meeting_lift:
  return meeting_lift(args.meeting_lift,args.output,args.exe)
 if args.refine_p1:
  return refine_p1(args.refine_p1,args.output,args.exe)
 if args.refine_p3:
  return refine_p3(args.refine_p3,args.output,args.exe)
 if args.refine_p4:
  return refine_p4(args.refine_p4,args.output,args.exe)
 if args.refine_p5:
  return refine_p5(args.refine_p5,args.output,args.exe)
 if args.refine_p6:
  return refine_p6(args.refine_p6,args.output,args.exe)
 if args.refine_p7:
  return refine_p7(args.refine_p7,args.output,args.exe)
 if args.refine_p8:
  return refine_p8(args.refine_p8,args.output,args.exe)
 if args.refine_p8_pages:
  return refine_p8_pages(args.output,args.exe)
 OUT=args.output.resolve()
 if args.exe:
  EXE=args.exe.resolve()
 OUT.mkdir(parents=True,exist_ok=True)
 jobs=[]
 setup_scenes=[('hex-setup','settings-hex-setup'),('hex-installing','settings-hex-installing'),('hex-install-failed','settings-hex-install-failed'),('hex-installed','settings-hex-installed'),('hex-meetings-off','settings-hex-meetings-off'),('hex-meetings-enable','settings-hex-meetings-enable')]
 for theme in ('dark','light'):
  for width in (1280,390):
   for name,scene in setup_scenes:
    jobs.append((name,scene,width,theme,900,None))
   for name,scene,height in PAGES:
    if args.channel=='stable' and name in ('sync','connected-apps'):continue
    # The narrow Engine tab is longer than the wide one; the render checks that nothing is cut off.
    jobs.append((name,scene,width,theme,7600 if width==390 and scene.startswith('settings-engine') else height,None))
   for name,scene,events in [('updates-available','updates-ready',None),('updates-checking','updates-checking',None),('hex-scrolled','settings-hex-expanded',[tap('settings-jump-5')]),('recording-expanded','settings-recording-expanded',None),('summaries-expanded','settings-summaries-expanded',None),('helpers-expanded','settings-helpers-expanded',None),('speech-expanded','settings-speech-expanded',None),('privacy-expanded','settings-privacy',[tap('notes-privacy-details')]),('diagnostics-expanded','settings-diagnostics',None),('sync-expanded','settings-sync-expanded',None),('people-editor','people-team-editor',[tap('team-advanced')])]:
    if args.channel=='stable' and name=='sync-expanded':continue
    jobs.append((name,scene,width,theme,2200 if 'expanded' in name or 'editor' in name else 900,events))
 jobs.append(('people-suggestion','people-suggestion',1280,'dark',1800,None))
 # Existing legacy search proof keeps its original 800px viewport and exact interaction.
 events=json.loads((ROOT/'tools/ui-scripts/settings-search.json').read_text())['events'][:-1]
 jobs.append(('search','settings-search',1280,'light',800,events))
 mcp_scene='settings-general' if args.channel=='stable' else 'settings-mcp-mixed'
 for query in ('MCP','connect','Claude','Codex','Cursor','OpenCode'):
  jobs.append(('mcp-search-'+query,mcp_scene,1280,'light',1200,[tap('hex-filter',.2),dict(type='type',text=query,t=.4)]))
 if args.channel=='stable':
  for query in ('Sync','Connected apps'):
   typed=dict(type='type',text=query,t=.4)
   jobs.append(('hidden-search-'+query,'settings-general',1280,'light',900,[tap('hex-filter',.2),typed]))
   jobs.append(('hidden-finder-'+query,'settings-general',1280,'light',900,[dict(type='key',name='k',cmd=True,t=.2),typed]))
 if args.cloud_ask:
  jobs=[('cloud-ask','settings-cloud-ask',1280,'dark',1400,None),('cloud-ask-label','cloud-ask-answer',1280,'dark',900,[tap('ask-endpoint-details')])]
 if args.search_keys:
  mapping=json.loads((ROOT/'tools/ui-fixtures/settings-search-map.json').read_text(encoding='utf-8'))
  jobs=[('legacy-search-'+str(i).zfill(2),'settings-search',1280,'light',900,[tap('hex-filter',.2),dict(type='type',text=item['key'],t=.4),dict(type='key',name='Enter',t=1.)]) for i,item in enumerate(mapping)]
 if args.quick:jobs=[j for j in jobs if j[0] in ('updates','hex-dictation','meetings-recording') and j[2]==1280 and j[3]=='dark']
 if args.only:jobs=[j for j in jobs if j[0] in args.only.split(',')]
 with ThreadPoolExecutor(max_workers=2) as pool:paths=list(pool.map(render,jobs))
 for name in ('updates','hex-dictation','meetings-recording'):
  source=OUT/f'settings-{name}-1280-dark.png'
  if source.exists():shutil.copyfile(source,OUT/f'{name}-1280-dark.png')
 (OUT/('cloud-ask-render-manifest.json' if args.cloud_ask else 'search-render-manifest.json' if args.search_keys else 'render-manifest.json')).write_text(json.dumps(paths,indent=2)+'\n',encoding='utf-8')
 if args.cloud_ask:
  for name,required in [('cloud-ask',('ask-connection-url','ask-connection-model','ask-connection-key','ask-connection-consent')),('cloud-ask-label',('ask-endpoint-label','ask-endpoint-timing'))]:
   source=(OUT/f'settings-{name}-1280-dark.json').read_text(encoding='utf-8-sig')
   assert 'synthetic-render-only-key' not in source,'Secure key appeared in render tree'
   elements=json.loads(source)['elements']
   keys={e.get('key') for e in elements}
   assert set(required)<=keys,(name,set(required)-keys)
  print('PASS: Ask connection controls, endpoint/model label, timing and key redaction in isolated dark renders',flush=True)
 if args.search_keys:
  for i,item in enumerate(mapping):
   tree=json.loads((OUT/f'settings-legacy-search-{i:02}-1280-light.json').read_text(encoding='utf-8-sig'))
   elements=tree['elements']
   title=next(n for n in elements if n.get('key')=='settings-title')
   assert item['home'].startswith(title['text']),(item['key'],title.get('text'),item['home'])
   target=next((n for n in elements if n.get('key')==item['target']),None)
   assert target is not None,(item['key'],item['target'])
   assert target['rect'][1]<900 and target['rect'][1]+target['rect'][3]>0,(item['key'],target['rect'])
  print('PASS: all 49 legacy keys search to their approved page and visible destination',flush=True)
 print(f'PASS: {len(paths)} isolated renders',flush=True)
if __name__=='__main__':main()
