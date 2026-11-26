#!/usr/bin/env python3
"""Unsigned desktop package: python3 tools/pack_linux.py HEAD_SHA NEW_OUTPUT_FOLDER."""
import sys; sys.dont_write_bytecode = True
import gzip, hashlib, io, json, os, re, shutil, stat, subprocess, tarfile, tomllib
from pathlib import Path
REPO = Path(__file__).resolve().parents[1]
TARGET, FEATURES = 'x86_64-unknown-linux-gnu', 'desktop,asr-runtime'
GIT_ENV = dict(PATH='/usr/bin:/bin', GIT_NO_REPLACE_OBJECTS='1', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1', GIT_OPTIONAL_LOCKS='0', GIT_ALLOW_PROTOCOL='file')
NOTICES = ['LICENSE', 'THIRD_PARTY.md', 'assets/fonts/Inter-LICENSE.txt', 'assets/licenses/webpki-roots-CDLA-Permissive-2.0.txt', 'assets/licenses/lucide-LICENSE.txt', 'assets/licenses/solar-LICENSE.txt']

def run(args, **kw):
    if 'cargo' in args: subprocess.run(['timeout', '--kill-after=10s', '2700s', 'bash', '-c', 'while pgrep -x cargo >/dev/null; do sleep 1; done'], env=GIT_ENV, check=True)
    return subprocess.run(['timeout', '--kill-after=10s', '2700s', *map(str, args)], env=GIT_ENV, check=True, **kw)

def git(root, *args):
    return run(['git', '--no-replace-objects', '-C', root, '-c', 'core.fsmonitor=false', *args], capture_output=True).stdout

def no_links(path):
    for p in (path, *path.parents):
        if p.is_symlink(): raise ValueError('symlinked path or ancestor refused')
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def directory(path, fresh=False):
    path = Path(path).absolute()
    no_links(path); path = Path(os.path.abspath(path))
    fd = os.open('/', os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for i, part in enumerate(path.parts[1:], 1):
            try: os.mkdir(part, dir_fd=fd)
            except FileExistsError:
                if fresh and i == len(path.parts) - 1: raise
            child = os.open(part, os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd); fd = child
        return os.dup(fd)
    finally: os.close(fd)

def new_file(path, mode=0o644, parent=None):
    fd = directory(Path(path).parent) if parent is None else os.dup(parent)
    try: return os.fdopen(os.open(Path(path).name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode, dir_fd=fd), 'wb')
    finally: os.close(fd)
def put(path, data, mode=0o644, parent=None):
    with new_file(path, mode, parent) as stream: stream.write(data.encode() if isinstance(data, str) else data)

def copy(src, dst, mode=0o644):
    with src.open('rb') as source, new_file(dst, mode) as target: shutil.copyfileobj(source, target)

def notices(source, stage, cache, metadata, archive, git_archives):
    locked = {(p['name'], p['version'], p.get('source')): p for p in tomllib.loads((source / 'Cargo.lock').read_text())['package']}
    choices = json.loads((source / 'tools/license_choices.json').read_text())
    resolved = {p['id'] for p in metadata['resolve']['nodes']}
    coverage, standards = [], set()
    upstream_paths = {}
    notice_root = stage / 'dependency-notices'
    standards_dir = notice_root / 'standard-licenses'
    for p in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'], p['source'] or '')):
        if p['id'] in metadata['workspace_members'] or p['id'] not in resolved: continue
        lock = locked[(p['name'], p['version'], p['source'])]
        package_name = f"{p['name']}-{p['version']}"
        source_key = 'path' if p['source'] is None else hashlib.sha256(p['source'].encode()).hexdigest()
        destination = notice_root / 'sources' / source_key / package_name
        if p['source'] in git_archives:
            data, prefix = git_archives[p['source']], ''
        elif p['source']:
            crate, = cache.glob(f"registry/cache/*/{p['name']}-{p['version']}.crate")
            if digest(crate) != lock['checksum']: raise ValueError('crate checksum mismatch')
            data, prefix = crate.read_bytes(), f"{p['name']}-{p['version']}/"
        else:
            data, prefix = archive.read_bytes(), Path(p['manifest_path']).parent.relative_to(source).as_posix() + '/'
        manifest_dir = Path(p['manifest_path']).parent
        archive_root = manifest_dir
        if p['source'] in git_archives and p['license_file']:
            archive_root = Path(git(manifest_dir, 'rev-parse', '--show-toplevel').decode().strip())
        declared = (manifest_dir / p['license_file']).resolve().relative_to(archive_root.resolve()).as_posix() if p['license_file'] else None
        files = {}
        file_paths, parent_paths = set(), set()
        with tarfile.open(fileobj=io.BytesIO(data)) as tar:
            for item in tar:
                name = item.name.removeprefix(prefix)
                if item.isfile() and item.name.startswith(prefix) and (name == declared or re.search(r'(?i)(^|/)(licen[cs]es?|copying|copyright|notices?)([^/]*$|/)', name)):
                    path = Path(name)
                    if path.is_absolute() or '..' in path.parts: raise ValueError('invalid notice path')
                    if path in file_paths or path in parent_paths or not file_paths.isdisjoint(path.parents):
                        raise FileExistsError(f'conflicting notice path: {name}')
                    file_paths.add(path)
                    parent_paths.update(path.parents)
                    contents = tar.extractfile(item).read()
                    # Coverage rows share the first stored path for identical upstream bytes.
                    if contents not in upstream_paths:
                        target = destination / path
                        put(target, contents)
                        upstream_paths[contents] = target.relative_to(stage).as_posix()
                    files[name] = upstream_paths[contents]
        if declared and declared not in files: raise ValueError('declared notice absent from immutable source')
        kind = 'upstream' if files else 'metadata'
        standard_paths = []
        if not files:
            expression = p['license'] or ''
            selected = next((c for c in choices if (c['name'], c['version']) == (p['name'], p['version'])), None)
            if selected and (selected['declared'] != expression or selected['selected'] not in expression.split(' OR ')): raise ValueError('invalid licence choice')
            effective = selected['selected'] if selected else expression
            exception = 'Apache-2.0 WITH LLVM-exception'
            if effective == exception:
                ids = {'Apache-2.0', 'LLVM-exception'}
            elif effective == f'{exception} OR Apache-2.0 OR MIT':
                ids = {'Apache-2.0', 'LLVM-exception', 'MIT'}
            else:
                tokens = re.findall(r'[A-Za-z0-9.+-]+|[()]', effective)
                ids = set(tokens) - {'AND', 'OR', '(', ')'}
                if ''.join(tokens) != re.sub(r'\s', '', effective) or not ids or not ids <= {'MIT', 'Apache-2.0', 'BSL-1.0', 'Zlib'}: raise ValueError('uncovered licence')
            holder = '; '.join(p['authors']) or p['repository']
            if not holder: raise ValueError('missing attribution')
            relative_standards = os.path.relpath(standards_dir, destination)
            notice = destination / 'NOTICE.txt'
            put(notice, f"{p['name']} {p['version']}\nCopyright holder (Cargo metadata): {holder}\nDeclared: {expression}\nIncluded: {effective}\nStandard texts: {relative_standards}/\n")
            files['NOTICE.txt'] = notice.relative_to(stage).as_posix()
            for identifier in sorted(ids):
                target = standards_dir / f'{identifier}.txt'
                if identifier not in standards:
                    copy(source / f'tools/licenses/{identifier}.txt', target)
                    standards.add(identifier)
                standard_paths.append(target.relative_to(stage).as_posix())
        coverage.append(dict(
            name=p['name'], version=p['version'], source=p['source'], license=p['license'], kind=kind,
            notice_files=len(files), notice_paths=list(files.values()),
            standard_license_paths=standard_paths,
        ))
    put((stage / 'DEPENDENCY_NOTICE_COVERAGE.json'), json.dumps(coverage, indent=2) + '\n')

def git_source(source, work):
    sources = {p['source'] for p in tomllib.loads((source / 'Cargo.lock').read_text())['package'] if p.get('source', '').startswith('git+')}
    if not sources: return [], {}, {}
    if len(sources) != 1 or 'SIDENOTE_GIT_MIRROR' not in os.environ: raise ValueError('one verified local Git mirror is required; no Git network fetch is allowed')
    identity, = sources
    location, _, revision = identity.rpartition('#')
    url = location[4:].split('?')[0]
    if identity != f'git+{url}?rev={revision}#{revision}' or not re.fullmatch(r'[0-9a-f]{40}', revision): raise ValueError('Git dependency must be pinned by full revision')
    mirror = Path(os.environ['SIDENOTE_GIT_MIRROR']).absolute()
    no_links(mirror)
    tree = git(mirror, 'rev-parse', revision + '^{tree}').decode().strip()
    clone = source / '.git-source'
    git(work, 'clone', '--no-local', '--mirror', '--', mirror, clone)
    if git(clone, 'rev-parse', revision + '^{commit}').decode().strip() != revision or git(clone, 'rev-parse', revision + '^{tree}').decode().strip() != tree: raise ValueError('Git dependency clone mismatch')
    git(clone, 'update-ref', 'refs/heads/release', revision)
    git(clone, 'symbolic-ref', 'HEAD', 'refs/heads/release')
    config = work / 'sources.toml'
    put(config, '[source.locked]\ngit=' + json.dumps(url) + '\nrev=' + json.dumps(revision) + '\nreplace-with="local"\n[source.local]\ngit=' + json.dumps(clone.as_uri()) + '\nrev=' + json.dumps(revision) + '\n')
    return ['--config', str(config)], {identity: git(clone, 'archive', '--format=tar', revision)}, {identity: dict(sha=revision, tree=tree)}

def build(sha, tree, output, work, rust_bin):
    source = work / 'source'
    git(work, 'clone', '--no-local', '--no-checkout', '--', str(REPO), str(source))
    git(source, 'checkout', '--detach', sha)
    if git(source, 'rev-parse', 'HEAD').decode().strip() != sha or git(source, 'rev-parse', 'HEAD^{tree}').decode().strip() != tree: raise ValueError('cloned commit/tree mismatch')
    archive = work / 'source.tar'
    put(archive, git(source, 'archive', '--format=tar', sha))
    version = tomllib.loads((source / 'Cargo.toml').read_text())['package']['version']
    if not re.fullmatch(r'[A-Za-z0-9.+-]+', version): raise ValueError('invalid version')
    source_args, git_archives, git_ids = git_source(source, work)
    for parent in work.parents:
        if any((parent / '.cargo' / n).exists() for n in ('config', 'config.toml')): raise ValueError('external Cargo ancestor config refused')
    env = dict(GIT_ENV, PATH=str(rust_bin) + ':/usr/bin:/bin', HOME=str(work / 'home'), CARGO_HOME=str(work / 'cargo'), CARGO_TARGET_DIR=str(work / 'target'), TMPDIR=str(work / 'tmp'), CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', CARGO_NET_GIT_FETCH_WITH_CLI='true', CARGO_REGISTRIES_CRATES_IO_PROTOCOL='sparse', RUSTFLAGS=f'--remap-path-prefix={work}=/usr/src/sidenote')
    for name in ('home', 'cargo', 'target', 'tmp'): os.close(directory(work / name, fresh=True))
    command = ['env', '-i', *(f'{k}={v}' for k, v in env.items()), 'bash', source / 'tools/linux_test_env.sh', 'cargo', *source_args]
    common = ['--locked', '--no-default-features', '--features', FEATURES]
    run([*command, 'build', '--release', '--bin', 'sidenote', '--target', TARGET, *common], cwd=source)
    metadata = json.loads(run([*command, 'metadata', '--filter-platform', TARGET, '--format-version=1', *common], cwd=source, capture_output=True).stdout)
    name = f'sidenote-{version}-linux-x86_64-{sha}'
    stage = work / name
    os.close(directory(stage, fresh=True))
    binary = work / f'target/{TARGET}/release/sidenote'
    copy(binary, stage / 'bin/sidenote', 0o755)
    if run(['env', '-i', *(f'{k}={v}' for k, v in env.items()), stage / 'bin/sidenote', '--version'], capture_output=True).stdout.decode().strip() != f'Sidenote {version}': raise ValueError('binary version mismatch')
    for notice in NOTICES: copy(source / notice, stage / notice)
    notices(source, stage, work / 'cargo', metadata, archive, git_archives)
    copy(source / 'assets/icon/png/256.png', stage / 'share/icons/hicolor/256x256/apps/sidenote.png')
    put(stage / 'share/applications/sidenote.desktop', '[Desktop Entry]\nType=Application\nName=Sidenote\nExec=sidenote\nIcon=sidenote\nTerminal=false\nCategories=Office;AudioVideo;\n')
    linked = run(['readelf', '-d', binary], capture_output=True).stdout.decode()
    versions = run(['readelf', '--version-info', binary], capture_output=True).stdout.decode()
    glibc = max(re.findall(r'GLIBC_([0-9.]+)', versions), key=lambda v: tuple(map(int, v.split('.'))))
    dependencies = sorted(re.findall(r'Shared library: \[(.*?)\]', linked))
    put((stage / 'RUNTIME.txt'), f'x86_64 GNU/Linux; glibc >= {glibc}\nLinked: ' + ', '.join(dependencies) + '\nDesktop: X11 or Wayland; libX11.so.6, libX11-xcb.so.1, libXi.so.6, libXcursor.so.1, libXrandr.so.2, libXfixes.so.3, libXrender.so.1, libxcb.so.1; libwayland-client.so.0, libwayland-cursor.so.0, libwayland-egl.so.1; libxkbcommon.so.0 and libxkbcommon-x11.so.0; libvulkan.so.1 and a Vulkan ICD/driver.\nSession D-Bus for tray; xdg-open for reveal.\nUnsigned manual package; installation acceptance is layer 16. Linux inference stays refused. No runtime/models, credentials or user configuration bundled.\n')
    copy(archive, stage / 'source.tar')
    put((stage / 'SOURCE.json'), json.dumps(dict(sha=sha, tree=tree, git_sources=git_ids, version=version, target=TARGET, features=FEATURES, binary_sha256=digest(binary), source_sha256=digest(archive)), indent=2) + '\n')
    put((stage / 'SHA256SUMS'), ''.join(f'{digest(p)}  {p.relative_to(stage).as_posix()}\n' for p in sorted(stage.rglob('*')) if p.is_file()))
    artifact = work / (name + '.tar.gz')
    epoch = int(git(source, 'show', '-s', '--format=%ct', sha))
    with new_file(artifact) as raw, gzip.GzipFile(filename='', fileobj=raw, mode='wb', mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode='w') as tar:
            for p in [stage, *sorted(stage.rglob('*'))]:
                info = tar.gettarinfo(p, arcname=p.relative_to(work).as_posix())
                info.uid = info.gid = 0
                info.uname = info.gname = ''
                info.mtime = epoch
                info.mode = 0o755 if p.is_dir() or p == stage / 'bin/sidenote' else 0o644
                if p.is_file():
                    with p.open('rb') as stream: tar.addfile(info, stream)
                else: tar.addfile(info)
    os.close(directory(output, fresh=True))
    for p in sorted(stage.rglob('*')):
        destination = output / name / p.relative_to(stage)
        if p.is_dir(): os.close(directory(destination))
        else: copy(p, destination, 0o755 if p == stage / 'bin/sidenote' else 0o644)
    copy(artifact, output / artifact.name)
    put(output / (artifact.name + '.sha256'), f'{digest(artifact)}  {artifact.name}\n')
    print(output / artifact.name)

def package(sha, output):
    output = Path(output).absolute()
    no_links(output)
    if not output.resolve().is_relative_to(REPO) or output.resolve() == REPO or output.exists(): raise ValueError('output must be a new folder inside checkout')
    if sys.platform != 'linux' or os.uname().machine != 'x86_64': raise ValueError('native x86_64 Linux required')
    if not re.fullmatch(r'[0-9a-fA-F]{7,40}', sha): raise ValueError('expected commit SHA')
    sha = git(REPO, 'rev-parse', sha + '^{commit}').decode().strip()
    if sha != git(REPO, 'rev-parse', 'HEAD').decode().strip(): raise ValueError('requested SHA is not checked-out HEAD')
    if git(REPO, 'for-each-ref', 'refs/replace'): raise ValueError('replacement refs refused')
    if git(REPO, 'status', '--porcelain', '--untracked-files=all'): raise ValueError('source tree must be clean')
    tree = git(REPO, 'rev-parse', 'HEAD^{tree}').decode().strip()
    rust_bin = Path(os.environ.get('SIDENOTE_RUST_BIN', Path(os.environ.get('RUSTUP_HOME', Path.home() / '.rustup')) / f'toolchains/stable-{TARGET}/bin')).resolve()
    if not (rust_bin / 'cargo').is_file() or not (rust_bin / 'rustc').is_file(): raise ValueError('set SIDENOTE_RUST_BIN to the installed stable toolchain bin')
    no_links(Path('/tmp'))
    if not shutil.rmtree.avoids_symlink_attacks: raise ValueError('fd-safe cleanup required')
    parent = os.open('/tmp', os.O_DIRECTORY | os.O_NOFOLLOW)
    work = Path(run(['mktemp', '-d', '/tmp/sidenote-release.XXXXXXXX'], capture_output=True).stdout.decode().strip())
    try:
        no_links(work)
        if work.is_relative_to(Path.home().resolve()): raise ValueError('workspace must be outside caller HOME')
        print(f'Workspace: {work}', file=sys.stderr, flush=True)
        build(sha, tree, output, work, rust_bin)
    finally:
        if stat.S_ISLNK(os.stat(work.name, dir_fd=parent, follow_symlinks=False).st_mode): os.unlink(work.name, dir_fd=parent)
        else: shutil.rmtree(work.name, dir_fd=parent)
        os.close(parent)

if __name__ == '__main__':
    package(*sys.argv[1:])
