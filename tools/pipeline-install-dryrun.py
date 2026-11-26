"""Exercise the shipped installer/entrypoint using synthetic files in a private profile."""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile
import time
import uuid
import winreg
from contextlib import contextmanager

REPO = Path(__file__).resolve().parents[1]
SOURCE = REPO / 'scripts/meeting-pipeline'
# Match the actual embedded payload rather than accidentally packaging a new helper.
NAMES = re.findall(r'include_str!\("../scripts/meeting-pipeline/([^"/]+)"\)',
                   (REPO / 'src/updates_windows.rs').read_text(encoding='utf-8'))
LEGACY_NAMES = [name for name in NAMES if name != 'job-contract.json']
SETTINGS = ('HEX_MEETING_PROJECT', 'HEX_MEETING_PROJECT_PATTERN',
            'HEX_MEETING_PROJECT_CONTEXT_PATTERN', 'HEX_MEETING_KB_REPO',
            'HEX_MEETING_OWNER_NAME', 'HEX_MEETING_COMPANY_NAME',
            'HEX_MEETING_PRODUCT_NAME', 'HEX_TE_ASR_MODEL')
FLAGS = getattr(subprocess, 'CREATE_NO_WINDOW', 0) | getattr(subprocess, 'BELOW_NORMAL_PRIORITY_CLASS', 0)
REGISTRY_PRELUDE = r'''
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices;
public class PipelineTestRegistry {
    [DllImport("advapi32.dll")] public static extern int RegOverridePredefKey(IntPtr key, IntPtr replacement);
}'
$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($env:SIDENOTE_TEST_HKCU)
if (-not $key -or [PipelineTestRegistry]::RegOverridePredefKey([IntPtr](-2147483647), $key.Handle.DangerousGetHandle()) -ne 0) {
    throw 'Test registry isolation failed'
}
'''


@contextmanager
def user_settings(env, settings):
    # Redirect HKCU only in the child: never inspect or alter the owner's Environment key.
    hive = r'Software\SidenotePipelineTest-' + uuid.uuid4().hex
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, hive + r'\Environment') as key:
        for name, value in settings.items():
            winreg.SetValueEx(key, name, 0, winreg.REG_SZ, value)
    try:
        yield dict(env, SIDENOTE_TEST_HKCU=hive)
    finally:
        winreg.DeleteKey(winreg.HKEY_CURRENT_USER, hive + r'\Environment')
        winreg.DeleteKey(winreg.HKEY_CURRENT_USER, hive)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def environment(profile, configured=True):
    env = os.environ.copy()
    for key in SETTINGS:
        env.pop(key, None)
    env.update(USERPROFILE=str(profile / 'home'), HOME=str(profile / 'home'),
               APPDATA=str(profile / 'appdata'), LOCALAPPDATA=str(profile / 'localappdata'),
               TEMP=str(profile / 'temp'), TMP=str(profile / 'temp'))
    env['SIDENOTE_DATA_DIR'] = str(profile / 'data')
    for key in ('USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'TEMP'):
        Path(env[key]).mkdir(parents=True, exist_ok=True)
    if configured:
        env.update(dict.fromkeys(SETTINGS, 'synthetic-proof'))
        env['HEX_MEETING_KB_REPO'] = str(profile / 'synthetic-kb')
        Path(env['HEX_MEETING_KB_REPO']).mkdir(exist_ok=True)
    return env


def bundle(stub=True):
    files = {name: (SOURCE / name).read_text(encoding='utf-8') for name in NAMES}
    if stub:
        files['hex-meeting-watch.ps1'] = "Write-Output 'stub-watcher-started'\n"
    entry = (REPO / 'src/pipeline_entry.ps1').read_text(encoding='utf-8')
    return dict(version='synthetic-proof', files=files,
                hashes={n: digest(s.encode()) for n, s in files.items()},
                entry=entry, entry_hash=digest(entry.encode()))


def powershell(args, env, payload=None):
    require(args[0] == '-File', 'expected script invocation')
    command = '& ' + ' '.join("'" + arg.replace("'", "''") + "'" for arg in args[1:])
    return subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive',
                           '-ExecutionPolicy', 'Bypass', '-Command',
                           REGISTRY_PRELUDE + command + '; exit $LASTEXITCODE'], env=env,
                          input=json.dumps(payload) if payload else None,
                          text=True, capture_output=True, timeout=60, creationflags=FLAGS)


def snapshot(root):
    return {str(p.relative_to(root)): digest(p.read_bytes())
            for p in root.rglob('*') if p.is_file()}


def legacy_files():
    files = dict.fromkeys(LEGACY_NAMES, '# synthetic legacy file\n')
    files['hex-meeting-watch.ps1'] = "Write-Output 'legacy-watcher-started'\n"
    files['hex-meeting-compare-granola.py'] = "SCRIPTS = Path(__file__).resolve().parent\n"
    return files


def dispatch(root, env, expected):
    result = powershell(['-File', str(root / 'hex-meeting-watch.ps1')], env)
    require(result.returncode == 0 and expected in result.stdout, 'dispatcher did not launch expected watcher')


def versioned_upgrade(profile):
    env = environment(profile)
    root = Path(env['USERPROFILE']) / '.config/hex/scripts'
    generation = root / '.pipeline-generations' / ('generation-' + uuid.uuid4().hex)
    generation.mkdir(parents=True)
    files = legacy_files()
    for name, text in files.items():
        (generation / name).write_text(text, encoding='utf-8')
    seal = generation / '.generation.json'
    seal.write_text(json.dumps(dict(schema='sidenote.pipeline/1', version='old-14',
                                   files={name: digest((generation / name).read_bytes()) for name in files})))
    selection = dict(generation=generation.name, manifest_sha256=digest(seal.read_bytes()))
    pointer = root / '.pipeline-current.json'
    pointer.write_text(json.dumps(dict(schema='sidenote.pipeline/1', current=selection, previous=None)))
    old_entry = subprocess.check_output(['git', '-C', str(REPO), 'show', '8407e2d:src/pipeline_entry.ps1'])
    (root / 'hex-meeting-watch.ps1').write_bytes(old_entry)
    (root / 'custom-owner-file.txt').write_bytes(b'synthetic owner bytes\x00\xff')
    old_generation = snapshot(generation)
    old_pointer = pointer.read_bytes()
    with user_settings(env, {}) as isolated:
        dispatch(root, isolated, 'legacy-watcher-started')
        audio = generation / 'meeting-audio.py'
        saved = audio.read_bytes()
        audio.write_bytes(b'tampered synthetic bytes')
        result = powershell(['-File', str(root / 'hex-meeting-watch.ps1')], isolated)
        require(result.returncode != 0 and 'legacy-watcher-started' not in result.stdout, 'corrupt generation dispatched')
        result = powershell(['-File', str(REPO / 'src/pipeline_install.ps1')], isolated, bundle())
        require(result.returncode != 0 and pointer.read_bytes() == old_pointer, 'corrupt prior generation accepted')
        audio.write_bytes(saved)
        result = powershell(['-File', str(REPO / 'src/pipeline_install.ps1')], isolated, bundle())
        require(result.returncode == 0, 'versioned 14-file upgrade failed')
        selected = json.loads(pointer.read_text())
        require(selected['previous'] == selection, 'prior selection lost')
        require(snapshot(generation) == old_generation, 'prior generation bytes changed')
        require(any(p.read_bytes() == old_entry for p in (root / '.pipeline-backups').rglob('hex-meeting-watch.ps1')), 'old dispatcher backup lost')
        require((root / 'custom-owner-file.txt').read_bytes() == b'synthetic owner bytes\x00\xff', 'custom file changed')
        dispatch(root, isolated, 'stub-watcher-started')
        current = root / '.pipeline-generations' / selected['current']['generation'] / '.generation.json'
        manifest = json.loads(current.read_text())
        require(set(manifest['files']) == set(NAMES) and len(NAMES) == 15, 'new membership incorrect')
        # Re-seal a same-size manifest with an unknown JSON name: reject membership, not just its hash.
        original_seal, original_pointer = current.read_bytes(), pointer.read_bytes()
        (current.parent / 'unexpected.json').write_bytes((current.parent / 'job-contract.json').read_bytes())
        manifest['files']['unexpected.json'] = manifest['files'].pop('job-contract.json')
        current.write_text(json.dumps(manifest))
        selected['current']['manifest_sha256'] = digest(current.read_bytes())
        pointer.write_text(json.dumps(selected))
        result = powershell(['-File', str(root / 'hex-meeting-watch.ps1')], isolated)
        require(result.returncode != 0 and 'stub-watcher-started' not in result.stdout, 'unknown member dispatched')
        current.write_bytes(original_seal)
        pointer.write_bytes(old_pointer)
        dispatch(root, isolated, 'legacy-watcher-started')
        pointer.write_bytes(original_pointer)
    print('PASS versioned 14 -> 15: old bytes/rollback/custom file preserved; corrupt hashes and unknown names rejected')


def dryrun(profile):
    env = environment(profile)
    root = Path(env['USERPROFILE']) / '.config/hex/scripts'
    root.mkdir(parents=True)
    with user_settings(env, {}) as isolated:
        fresh = powershell(['-File', str(REPO / 'src/pipeline_install.ps1')], isolated, bundle())
    require(fresh.returncode == 3, 'fresh profile silently claimed scripts were installed')
    require(not list(root.iterdir()), 'fresh failure published incomplete scripts')
    print('PASS fresh profile: missing script setup fails visibly without publishing files')
    for name, text in legacy_files().items():
        (root / name).write_text(text, encoding='utf-8')
    originals = {name: (root / name).read_bytes() for name in LEGACY_NAMES}
    install = REPO / 'src/pipeline_install.ps1'
    payload = bundle()
    before = snapshot(root)
    missing = environment(profile, configured=False)
    with user_settings(missing, {}) as isolated:
        result = powershell(['-File', str(install)], isolated, payload)
    require(result.returncode == 1, 'missing settings accepted')
    require(snapshot(root) == before, 'missing settings changed legacy bytes')
    with user_settings(missing, {}) as isolated:
        result = powershell(['-File', str(root / 'hex-meeting-watch.ps1')], isolated)
    require(result.returncode == 0 and 'legacy-watcher-started' in result.stdout, 'legacy watcher failed')
    print('PASS missing settings: all 14 legacy files unchanged and legacy watcher runs')
    for absent in SETTINGS:
        partial = dict.fromkeys(SETTINGS, 'synthetic-user-setting')
        del partial[absent]
        with user_settings(missing, partial) as isolated:
            result = powershell(['-File', str(install)], isolated, payload)
        require(result.returncode != 0, 'partial user settings accepted')
        require(snapshot(root) == before, 'partial settings changed legacy bytes')
    print('PASS partial settings: each of 8 missing User settings blocks publication')
    with user_settings(missing, dict.fromkeys(SETTINGS, 'synthetic-user-setting')) as isolated:
        result = powershell(['-File', str(install)], isolated, payload)
        require(result.returncode == 0, 'User-only settings install failed')
        require(not result.stdout and not result.stderr, 'installer printed private output')
        installed = snapshot(root)
        result = powershell(['-File', str(install)], isolated, payload)
        require(snapshot(root) == installed, 'identical retry changed installed files')
    require(result.returncode == 0, 'User-only settings install failed')
    require(not result.stdout and not result.stderr, 'installer printed private output')
    print('PASS User-only settings: isolated HKCU install and identical retry succeed unchanged')
    pointer = root / '.pipeline-current.json'
    selected = json.loads(pointer.read_text())
    generation = root / '.pipeline-generations' / selected['current']['generation']
    seal = generation / '.generation.json'
    require(digest(seal.read_bytes()) == selected['current']['manifest_sha256'], 'invalid generation seal')
    manifest = json.loads(seal.read_text())
    require(len(manifest['files']) == len(NAMES) == 15, 'expected 15 generation files')
    for name in NAMES:
        require(digest((generation / name).read_bytes()) == manifest['files'][name], 'generation hash mismatch')
    require(len(list((root / '.pipeline-backups').rglob('*.*'))) >= 14, 'legacy backup incomplete')
    for name, data in originals.items():
        require(any(p.read_bytes() == data for p in (root / '.pipeline-backups').rglob(name)), 'original backup bytes lost')
    previous = root / '.pipeline-generations' / selected['previous']['generation'] / '.generation.json'
    require(set(json.loads(previous.read_text())['files']) == set(LEGACY_NAMES), 'legacy generation membership changed')
    with user_settings(env, {}) as isolated:
        result = powershell(['-File', str(root / 'hex-meeting-watch.ps1')], isolated)
    require(result.returncode == 0 and 'stub-watcher-started' in result.stdout, 'stub watcher failed')
    print('PASS flat 14 -> 15: sealed files, pointer, exact original backups, real entry launches stub watcher')
    with user_settings(env, {}) as isolated:
        interrupt_install(profile, isolated, root, payload, 'stub-watcher-started')
    versioned_upgrade(profile / 'versioned')


def interrupt_install(profile, env, root, payload, expected=None):
    install = REPO / 'src/pipeline_install.ps1'
    pointer = root / '.pipeline-current.json'
    old = pointer.read_bytes()
    signal = profile / 'before-pointer.txt'
    # A debugger breakpoint pauses the real script after sealing, before publication.
    line = next(i for i, s in enumerate(install.read_text().splitlines(), 1)
                if s.strip() == 'Publish-Pointer $next $current')
    quote = lambda p: "'" + str(p).replace("'", "''") + "'"
    command = (REGISTRY_PRELUDE + f'Set-PSBreakpoint -Script {quote(install)} -Line {line} -Action '
               '{ [IO.File]::WriteAllText(' + quote(signal) + ", 'sealed'); Start-Sleep -Seconds 60 } | Out-Null; "
               f'& {quote(install)}')
    payload['version'] = 'synthetic-interrupted'
    child = subprocess.Popen(['powershell.exe', '-NoProfile', '-NonInteractive',
                              '-ExecutionPolicy', 'Bypass', '-Command', command],
                             env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, creationflags=FLAGS)
    try:
        child.stdin.write(json.dumps(payload))
        child.stdin.close()
        deadline = time.monotonic() + 30
        while not signal.exists() and child.poll() is None and time.monotonic() < deadline:
            time.sleep(.05)
        require(signal.exists(), 'did not reach sealed/pre-pointer breakpoint')
        seals = list((root / '.pipeline-generations').glob('generation-*/.generation.json'))
        require(len(seals) == 2, 'interrupted generation was not sealed')
        child.kill()
        child.wait(timeout=10)
    finally:
        if child.poll() is None:
            child.kill()
            child.wait(timeout=10)
    require(pointer.read_bytes() == old, 'kill changed selection')
    result = powershell(['-File', str(root / 'hex-meeting-watch.ps1'),
                         '-MeetingFolders', 'pipeline-proof-no-meeting'], env)
    require(result.returncode == 0, 'old watcher failed after interruption')
    if expected:
        require(expected in result.stdout, 'old watcher output missing after interruption')
    print('PASS killed install: new generation sealed; pointer unchanged; old watcher still runs')

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--profile', type=Path)
    args = parser.parse_args()
    if args.profile:
        dryrun(args.profile.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='hex-pipeline-proof-') as folder:
            dryrun(Path(folder))


if __name__ == '__main__':
    main()
