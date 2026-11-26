#!/usr/bin/env python3
"""Safe direct entry: CASE|all|HEAD CHECKOUT_RECEIPTS [TEST_SOURCE [PRODUCER_SOURCE]]. All test homes are private /tmp directories."""
import sys; sys.dont_write_bytecode = True
import configparser, hashlib, json, os, re, shutil, stat, subprocess, tarfile, unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO / 'tools'))
from pack_linux import copy, directory, new_file, put
MODE, RECEIPTS = sys.argv[1], Path(sys.argv[2]).absolute()
RUST = Path(os.environ.get('SIDENOTE_RUST_BIN', Path(os.environ.get('RUSTUP_HOME', Path.home() / '.rustup')) / 'toolchains/stable-x86_64-unknown-linux-gnu/bin')).resolve()
MIRROR = os.environ.get('SIDENOTE_GIT_MIRROR')
CODE = (Path(sys.argv[4]) if len(sys.argv) > 4 else REPO / 'tools/pack_linux.py').read_bytes()
TEST_CODE = Path(sys.argv[3]).read_bytes() if len(sys.argv) > 3 else Path(__file__).read_bytes()
SAFE = dict(PATH=str(RUST) + ':/usr/bin:/bin', GIT_NO_REPLACE_OBJECTS='1', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1', GIT_OPTIONAL_LOCKS='0')

def run(args, **kw):
    if 'cargo' in args: subprocess.run(['timeout', '180s', 'bash', '-c', 'while pgrep -x cargo >/dev/null; do sleep 1; done'], check=True)
    return subprocess.run(list(map(str, args)), capture_output=True, timeout=3500, **kw)

def git(p, *args):
    return run(['git', '--no-replace-objects', '-C', p, '-c', 'user.name=Synthetic', '-c', 'user.email=fixture@example.invalid', *args], env=dict(os.environ, GIT_NO_REPLACE_OBJECTS='1', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1'), check=True).stdout.decode().strip()

def receipt(name, data):
    if Path(name).name != name: raise ValueError('receipt basename required')
    put(name, data, parent=RECEIPT_FD)

def remove(path):
    fd = directory(path.parent)
    try:
        if not stat.S_ISREG(os.stat(path.name, dir_fd=fd, follow_symlinks=False).st_mode): raise ValueError('only owned regular fixture files may be replaced')
        os.unlink(path.name, dir_fd=fd)
    finally: os.close(fd)

def fixture(name):
    p = ROOT / name
    os.close(directory(p, fresh=True))
    for name in ['tools/linux_test_env.sh', 'tools/linux_test_runner.py', 'tools/license_choices.json', 'LICENSE', 'THIRD_PARTY.md', 'assets/fonts/Inter-LICENSE.txt', 'assets/licenses/webpki-roots-CDLA-Permissive-2.0.txt', 'assets/licenses/lucide-LICENSE.txt', 'assets/licenses/solar-LICENSE.txt', 'assets/icon/png/256.png']:
        copy(REPO / name, p / name)
    put((p / 'tools/pack_linux.py'), CODE)
    put((p / 'tests/linux_package.py'), TEST_CODE)
    for license in (REPO / 'tools/licenses').glob('*'): copy(license, p / 'tools/licenses' / license.name)
    put((p / '.gitignore'), '/target/\n')
    put((p / 'Cargo.toml'), '[workspace]\n[package]\nname="sidenote"\nversion="0.0.1"\nedition="2024"\n[features]\ndesktop=[]\nasr-runtime=[]\n')
    put((p / 'Cargo.lock'), 'version=4\n[[package]]\nname="sidenote"\nversion="0.0.1"\n')
    put((p / 'src/main.rs'), '#[cfg(debug_assertions)] compile_error!("AMBIENT_FLAGS");\nfn main(){if std::env::args().any(|a|a=="--payload"){println!("SIDENOTE_PAYLOAD_A")}else{println!("Sidenote 0.0.1")}}\n')
    git(p, 'init', '-q')
    return p
def commit(p):
    git(p, 'add', '.')
    git(p, 'commit', '-qm', 'synthetic fixture')
    return git(p, 'rev-parse', 'HEAD')

def cli(p, sha=None, output=None, env=None):
    output = output or p / 'target' / ('out-' + os.urandom(6).hex())
    r = run(['timeout', '--kill-after=10s', '3400s', sys.executable, p / 'tools/pack_linux.py', sha or git(p, 'rev-parse', 'HEAD'), output], env=env)
    receipt(p.name + '-' + output.name + '.log', r.stdout + r.stderr)
    for artifact in output.glob('*.tar.gz'):
        receipt(p.name + '-' + output.name + '.tar.gz', artifact.read_bytes())
        unpack = output / 'unpack'; os.close(directory(unpack, fresh=True))
        with tarfile.open(artifact) as tar:
            for item in tar:
                if Path(item.name).is_absolute() or '..' in Path(item.name).parts: raise ValueError('invalid archive path')
                destination = unpack / item.name
                if item.isdir(): os.close(directory(destination))
                elif item.isfile():
                    with new_file(destination, item.mode) as stream: shutil.copyfileobj(tar.extractfile(item), stream)
                else: raise ValueError('archive links refused')
    return r, output
def binary(out):
    return next((out / 'unpack').glob('*/bin/sidenote'))

class Packaging(unittest.TestCase):
    def test_guards(self):
        p = fixture('guards'); old = commit(p)
        for kind in ('unstaged', 'staged', 'untracked'):
            target = p / ('new.rs' if kind == 'untracked' else 'src/main.rs')
            if kind != 'untracked': remove(target)
            put(target, 'synthetic dirty input')
            if kind == 'staged': git(p, 'add', '.')
            r, out = cli(p)
            self.assertIn(b'source tree must be clean', r.stderr); self.assertFalse(out.exists())
            if kind == 'untracked': remove(target)
            git(p, 'reset', '--hard', 'HEAD')
        put((p / 'README'), 'new commit'); commit(p)
        r, out = cli(p, old)
        self.assertIn(b'not checked-out HEAD', r.stderr); self.assertFalse(out.exists())
    def test_replace(self):
        p = fixture('replace'); original = commit(p)
        main = p / 'src/main.rs'
        data = main.read_text().replace('PAYLOAD_A', 'PAYLOAD_B'); remove(main); put(main, data); replacement = commit(p)
        git(p, 'replace', original, replacement); git(p, 'update-ref', 'HEAD', original)
        r, out = cli(p, original)
        if r.returncode:
            self.assertTrue(b'replacement refs refused' in r.stderr or b'source tree must be clean' in r.stderr)
        else: self.assertEqual(run([binary(out), '--payload'], check=True).stdout.strip(), b'SIDENOTE_PAYLOAD_A')
    def test_output_link(self):
        p = fixture('output-link'); commit(p)
        os.close(directory(p / 'target/real', fresh=True)); (p / 'target/link').symlink_to(p / 'target/real', target_is_directory=True)
        r, out = cli(p, output=p / 'target/link/result')
        self.assertIn(b'symlink', r.stderr); self.assertFalse(out.exists()); self.assertEqual(list((p / 'target/real').iterdir()), [])
    def test_old_work_link(self):
        p = fixture('work-link'); commit(p)
        outside = ROOT / 'outside'; os.close(directory(outside / 'source', fresh=True))
        sentinel = outside / 'source/keep'; put(sentinel, b'public sentinel')
        os.close(directory(p / 'target', fresh=True)); (p / 'target/linux-package').symlink_to(outside, target_is_directory=True)
        r, _ = cli(p)
        self.assertTrue(sentinel.exists(), 'producer deleted through ignored work symlink')
        self.assertEqual(sentinel.read_bytes(), b'public sentinel'); self.assertFalse((outside / 'lock').exists())
        self.assertEqual(r.returncode, 0, r.stderr.decode())
    def test_fresh(self):
        p = fixture('fresh'); sha = commit(p)
        first, out = cli(p); self.assertEqual(first.returncode, 0, first.stderr.decode())
        cached = p / f'target/linux-package/build/{sha}/x86_64-unknown-linux-gnu/release/sidenote'
        if not cached.exists(): copy(binary(out), cached, 0o755)
        before = cached.stat(); data = cached.read_bytes(); self.assertIn(b'SIDENOTE_PAYLOAD_A', data)
        remove(cached)
        with new_file(cached, 0o755) as stream: stream.write(data.replace(b'SIDENOTE_PAYLOAD_A', b'SIDENOTE_PAYLOAD_B')); os.utime(stream.fileno(), ns=(before.st_atime_ns, before.st_mtime_ns))
        second, out = cli(p); self.assertEqual(second.returncode, 0, second.stderr.decode())
        self.assertEqual(run([binary(out), '--payload'], check=True).stdout.strip(), b'SIDENOTE_PAYLOAD_A')
        paths = [re.search(rb'Workspace: (/tmp/sidenote-release\.\w+)', r.stderr).group(1) for r in (first, second)]
        self.assertNotEqual(*paths)
        for path in paths: self.assertFalse(Path(os.fsdecode(path)).exists())
    def test_environment(self):
        p = fixture('environment'); commit(p)
        put(Path.home() / '.cargo/config.toml', '[build]\nrustflags=["-Cdebug-assertions=yes"]\n')
        put((Path(os.environ['CARGO_HOME']) / 'config.toml'), '[build]\nrustflags=["-Cdebug-assertions=yes"]\n')
        put(ROOT / '.cargo/config.toml', '[build]\nrustflags=["-Cdebug-assertions=yes"]\n')
        env = dict(os.environ, CARGO_ENCODED_RUSTFLAGS='-Cdebug-assertions=yes', RUSTFLAGS='-Cdebug-assertions=yes', RUSTC='/nonexistent', RUSTC_WRAPPER='/nonexistent', CARGO_TARGET_DIR=str(ROOT / 'escaped'), CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS='true')
        r, _ = cli(p, env=env); self.assertEqual(r.returncode, 0, r.stderr.decode()); self.assertFalse((ROOT / 'escaped').exists())
    def test_entry(self):
        p = fixture('entry'); commit(p)
        owner = ROOT / 'synthetic-owner'; config = owner / 'config.toml'; put(config, b'SYNTHETIC_OWNER_CONFIG')
        r = run([sys.executable, p / 'tests/linux_package.py', 'environment', p / 'target/entry-receipts'], env=dict(os.environ, CARGO_HOME=str(owner)))
        self.assertEqual(config.read_bytes(), b'SYNTHETIC_OWNER_CONFIG', 'test entry overwrote caller Cargo config')
        self.assertEqual(r.returncode, 0, (r.stdout + r.stderr).decode())
    def test_receipts(self):
        p = fixture('receipts'); commit(p)
        owner = ROOT / 'receipt-caller/config.toml'; put(owner, b'SYNTHETIC_CALLER_CONFIG')
        root = p / 'target/receipts'; os.close(directory(root, fresh=True))
        (root / 'output-link-result.log').symlink_to(owner)
        r = run([sys.executable, p / 'tests/linux_package.py', 'output_link', root], env=dict(os.environ, CARGO_HOME=str(owner.parent)))
        self.assertEqual(owner.read_bytes(), b'SYNTHETIC_CALLER_CONFIG', 'receipt symlink overwrote caller Cargo config')
        self.assertNotEqual(r.returncode, 0); self.assertIn(b'FileExistsError', r.stderr)
        os.symlink(owner, 'planted.log', dir_fd=RECEIPT_FD)
        with self.assertRaises(FileExistsError): receipt('planted.log', b'overwrite')
        os.unlink('planted.log', dir_fd=RECEIPT_FD)
        receipt('existing.log', b'first')
        with self.assertRaises(FileExistsError): receipt('existing.log', b'second')
        self.assertEqual(owner.read_bytes(), b'SYNTHETIC_CALLER_CONFIG')
        self.assertEqual((RECEIPTS / 'existing.log').read_bytes(), b'first')
        (p / 'target/link').symlink_to(root, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'): directory(p / 'target/link/new', fresh=True)
        self.assertFalse((root / 'new').exists())
    def test_cleanup(self):
        outside = ROOT / 'cleanup-outside'; sentinel = outside / 'keep'; put(sentinel, b'public sentinel')
        for failed in (False, True):
            p = fixture('cleanup-' + str(failed))
            put((p / 'build.rs'), 'fn main(){std::os::unix::fs::symlink("' + str(outside) + '",std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("escape")).unwrap();}')
            if failed: remove(p / 'src/main.rs'); put(p / 'src/main.rs', 'compile_error!("EXPECTED_BUILD_FAILURE");fn main(){}')
            commit(p); r, _ = cli(p)
            self.assertEqual(r.returncode == 0, not failed, r.stderr.decode())
            workspace = re.search(rb'Workspace: (/tmp/sidenote-release\.\w+)', r.stderr)
            self.assertIsNotNone(workspace, 'release did not use a disposable /tmp workspace')
            self.assertFalse(Path(os.fsdecode(workspace[1])).exists()); self.assertEqual(sentinel.read_bytes(), b'public sentinel')
    def test_notice_collision(self):
        p = fixture('notice-collision')
        mirror = ROOT / 'collision-mirror'
        local = p / 'deps/local'
        for dependency, notice in ((mirror, b'Mirror license\n'), (local, b'Local license\n')):
            put(dependency / 'Cargo.toml', '[package]\nname="duplicate"\nversion="0.1.0"\nedition="2024"\nlicense-file="LICENSE"\n')
            put(dependency / 'src/lib.rs', 'pub fn fixture() {}\n')
            put(dependency / 'LICENSE', notice)
        git(mirror, 'init', '-q')
        revision = commit(mirror)
        url = 'https://example.invalid/sidenote-notice-fixture'
        source = f'git+{url}?rev={revision}#{revision}'
        manifest = p / 'Cargo.toml'
        data = manifest.read_text().replace('[workspace]\n', '[workspace]\nexclude=["deps/local"]\n')
        remove(manifest)
        put(manifest, data + f'\n[dependencies]\nlocal = {{ package="duplicate", path="deps/local" }}\n'
                         f'mirror = {{ package="duplicate", git="{url}", rev="{revision}" }}\n')
        lock = p / 'Cargo.lock'
        remove(lock)
        put(lock, 'version=4\n\n[[package]]\nname="duplicate"\nversion="0.1.0"\n'
                  f'\n[[package]]\nname="duplicate"\nversion="0.1.0"\nsource="{source}"\n'
                  '\n[[package]]\nname="sidenote"\nversion="0.0.1"\n'
                  f'dependencies=["duplicate 0.1.0", "duplicate 0.1.0 ({source})"]\n')
        commit(p)
        r, out = cli(p, env=dict(os.environ, SIDENOTE_GIT_MIRROR=str(mirror)))
        self.assertEqual(r.returncode, 0, r.stderr.decode())
        stage = binary(out).parents[1]
        coverage = json.loads((stage / 'DEPENDENCY_NOTICE_COVERAGE.json').read_text())
        rows = {row['source']: row for row in coverage if row['name'] == 'duplicate'}
        self.assertEqual(set(rows), {None, source})
        paths = []
        for identity, expected in ((None, b'Local license\n'), (source, b'Mirror license\n')):
            row = rows[identity]
            self.assertEqual(row['notice_files'], len(row['notice_paths']))
            matches = [stage / name for name in row['notice_paths'] if Path(name).name == 'LICENSE']
            self.assertEqual(len(matches), 1)
            self.assertEqual(matches[0].read_bytes(), expected)
            paths.extend(matches)
        self.assertNotEqual(*paths)

    def test_notice_dedupe(self):
        p = fixture('notice-dedupe')
        shared = b'Shared license\r\nSecond line'
        different = b'Different license\n'
        packages = {
            'first': {'LICENSE': shared, 'NOTICE': shared},
            'second': {'COPYING': shared},
            'third': {'LICENSE': different},
        }
        root = p / 'Cargo.toml'
        excluded = json.dumps([f'deps/{name}' for name in packages])
        manifest = root.read_text().replace('[workspace]\n', f'[workspace]\nexclude={excluded}\n')
        remove(root)
        put(root, manifest + '\n[dependencies]\n' + ''.join(f'{name}={{path="deps/{name}"}}\n' for name in packages))
        lock = p / 'Cargo.lock'
        remove(lock)
        put(lock, 'version=4\n' + ''.join(f'\n[[package]]\nname="{name}"\nversion="0.1.0"\n' for name in packages)
                  + f'\n[[package]]\nname="sidenote"\nversion="0.0.1"\ndependencies={json.dumps(list(packages))}\n')
        for name, notices in packages.items():
            dependency = p / 'deps' / name
            put(dependency / 'Cargo.toml', f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2024"\n')
            put(dependency / 'src/lib.rs', 'pub fn fixture() {}\n')
            for filename, data in notices.items():
                put(dependency / filename, data)

        commit(p)
        r, out = cli(p)
        self.assertEqual(r.returncode, 0, r.stderr.decode())
        stage = binary(out).parents[1]
        coverage = json.loads((stage / 'DEPENDENCY_NOTICE_COVERAGE.json').read_text())
        self.assertCountEqual([row['name'] for row in coverage], list(packages))
        referenced = set()
        for row in coverage:
            paths = row['notice_paths']
            expected = packages[row['name']].values()
            self.assertEqual(row['kind'], 'upstream')
            self.assertEqual(row['notice_files'], len(expected))
            self.assertCountEqual([(stage / path).read_bytes() for path in paths], expected)
            referenced.update(stage / path for path in paths)
        stored = {path for path in (stage / 'dependency-notices').rglob('*') if path.is_file()}
        self.assertEqual(stored, referenced)
        self.assertCountEqual([path.read_bytes() for path in stored], [shared, different])

    def test_llvm_exception(self):
        p = fixture('llvm-exception')
        term = 'Apache-2.0 WITH LLVM-exception'
        packages = {'exact': (term, {'Apache-2.0', 'LLVM-exception'}),
                    'alternatives': (term + ' OR Apache-2.0 OR MIT', {'Apache-2.0', 'LLVM-exception', 'MIT'})}
        root = p / 'Cargo.toml'
        workspace = root.read_text().replace('[workspace]\n', '[workspace]\nexclude=["deps/exact", "deps/alternatives"]\n')
        remove(root)
        put(root, workspace + '\n[dependencies]\nexact={path="deps/exact"}\nalternatives={path="deps/alternatives"}\n')
        for name, (expression, _) in packages.items():
            put(p / f'deps/{name}/Cargo.toml',
                f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2024"\nlicense="{expression}"\nauthors=["Fixture"]\n')
            put(p / f'deps/{name}/src/lib.rs', 'pub fn fixture() {}\n')
        lock = p / 'Cargo.lock'
        remove(lock)
        put(lock, 'version=4\n\n[[package]]\nname="alternatives"\nversion="0.1.0"\n'
                  '\n[[package]]\nname="exact"\nversion="0.1.0"\n'
                  '\n[[package]]\nname="sidenote"\nversion="0.0.1"\ndependencies=["alternatives", "exact"]\n')
        commit(p)
        r, out = cli(p)
        self.assertEqual(r.returncode, 0, r.stderr.decode())
        stage = binary(out).parents[1]
        rows = {row['name']: row for row in json.loads((stage / 'DEPENDENCY_NOTICE_COVERAGE.json').read_text())}
        for name, (expression, ids) in packages.items():
            row = rows[name]
            self.assertEqual(row['license'], expression)
            self.assertEqual(len(row['notice_paths']), 1)
            self.assertIn(f'Included: {expression}', (stage / row['notice_paths'][0]).read_text())
            self.assertEqual({Path(path).stem for path in row['standard_license_paths']}, ids)
            for path in row['standard_license_paths']:
                self.assertEqual((stage / path).read_bytes(), (p / 'tools/licenses' / Path(path).name).read_bytes())

        manifest = p / 'deps/exact/Cargo.toml'
        for expression in ('MIT WITH LLVM-exception', 'Apache-2.0 WITH Unknown-exception', term + ' AND MIT'):
            text = manifest.read_text()
            remove(manifest)
            put(manifest, re.sub(r'^license=.*$', f'license="{expression}"', text, flags=re.MULTILINE))
            commit(p)
            rejected, out = cli(p)
            self.assertIn(b'uncovered licence', rejected.stderr)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertFalse(out.exists())

    def test_archive(self):
        p = REPO if MODE == 'HEAD' else fixture('archive')
        sha = git(p, 'rev-parse', 'HEAD') if MODE == 'HEAD' else commit(p)
        r, out = cli(p, sha); self.assertEqual(r.returncode, 0, r.stderr.decode())
        artifact, = out.glob('*.tar.gz'); stage = binary(out).parents[1]
        sums = dict(line.split('  ', 1)[::-1] for line in (stage / 'SHA256SUMS').read_text().splitlines())
        files = {f.relative_to(stage).as_posix(): f for f in stage.rglob('*') if f.is_file()}
        self.assertEqual(set(sums), set(files) - {'SHA256SUMS'})
        for name, value in sums.items(): self.assertEqual(hashlib.sha256(files[name].read_bytes()).hexdigest(), value)
        self.assertEqual((out / (artifact.name + '.sha256')).read_text().split()[0], hashlib.sha256(artifact.read_bytes()).hexdigest())
        source = json.loads((stage / 'SOURCE.json').read_text()); self.assertEqual(source['sha'], sha); self.assertEqual(source['tree'], git(p, 'rev-parse', sha + '^{tree}'))
        self.assertEqual(source['source_sha256'], hashlib.sha256(run(['git', '--no-replace-objects', '-C', p, 'archive', '--format=tar', sha], env=SAFE, check=True).stdout).hexdigest())
        entry = configparser.ConfigParser(interpolation=None, strict=True); entry.read(stage / 'share/applications/sidenote.desktop')
        self.assertEqual(dict(entry['Desktop Entry']), dict(type='Application', name='Sidenote', exec='sidenote', icon='sidenote', terminal='false', categories='Office;AudioVideo;'))
        if shutil.which('desktop-file-validate'): run(['desktop-file-validate', stage / 'share/applications/sidenote.desktop'], check=True)
        self.assertTrue((stage / 'share/icons/hicolor/256x256/apps/sidenote.png').read_bytes().startswith(b'\x89PNG\r\n\x1a\n'))
        for name in ('LICENSE', 'THIRD_PARTY.md', 'DEPENDENCY_NOTICE_COVERAGE.json', 'RUNTIME.txt'): self.assertTrue((stage / name).stat().st_size)
        if MODE == 'HEAD': receipt('artifact.json', json.dumps(dict(path=str(artifact), sha256=hashlib.sha256(artifact.read_bytes()).hexdigest(), bytes=artifact.stat().st_size, head=sha)) + '\n')

if __name__ == '__main__':
    if not RECEIPTS.resolve().is_relative_to(REPO / 'target'): raise ValueError('receipts must be inside checkout target')
    RECEIPT_FD = directory(RECEIPTS, fresh=True)
    if not shutil.rmtree.avoids_symlink_attacks: raise ValueError('safe cleanup required')
    ROOT = Path(subprocess.check_output(['mktemp', '-d', '/tmp/sidenote-tests.XXXXXXXX'], env=SAFE).decode().strip())
    parent = os.open('/tmp', os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for name in ('home', 'cargo', 'rustup', 'tmp'): os.close(directory(ROOT / name, fresh=True))
        os.environ.clear(); os.environ.update(SAFE, HOME=str(ROOT / 'home'), CARGO_HOME=str(ROOT / 'cargo'), RUSTUP_HOME=str(ROOT / 'rustup'), TMPDIR=str(ROOT / 'tmp'), SIDENOTE_RUST_BIN=str(RUST), CARGO_BUILD_JOBS='1')
        if MIRROR: os.environ['SIDENOTE_GIT_MIRROR'] = MIRROR
        selected = ['Packaging.test_' + MODE] if MODE not in ('all', 'HEAD') else []
        unittest.main(argv=[sys.argv[0], *selected], verbosity=2, failfast=True)
    finally:
        if stat.S_ISLNK(os.stat(ROOT.name, dir_fd=parent, follow_symlinks=False).st_mode): os.unlink(ROOT.name, dir_fd=parent)
        else: shutil.rmtree(ROOT.name, dir_fd=parent)
        os.close(parent); os.close(RECEIPT_FD)
