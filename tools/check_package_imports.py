"""Check the native imports in a Velopack release artifact without running it."""

import argparse
from contextlib import redirect_stderr, redirect_stdout
import io
from pathlib import Path, PurePosixPath
import struct
import sys
import tempfile
from xml.etree import ElementTree
from zipfile import BadZipFile, ZIP_DEFLATED, ZipFile
import zlib


PE_SUFFIXES = {".dll", ".exe"}
WINDOWS_DLLS = frozenset("""
    advapi32.dll bcrypt.dll bcryptprimitives.dll comctl32.dll comdlg32.dll
    crypt32.dll dbghelp.dll dwmapi.dll dwrite.dll dxgi.dll gdi32.dll imm32.dll
    iphlpapi.dll kernel32.dll msvcrt.dll ntdll.dll ole32.dll oleaut32.dll
    propsys.dll psapi.dll setupapi.dll shell32.dll uiautomationcore.dll user32.dll userenv.dll
    uxtheme.dll winhttp.dll ws2_32.dll
""".split())
VC143_X64_DLLS = {
    "concrt140.dll", "msvcp140.dll", "msvcp140_1.dll", "vcruntime140.dll", "vcruntime140_1.dll",
}


def pe_imports(data):
    if data[:2] != b"MZ":
        raise ValueError("missing DOS signature")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError("missing PE signature")
    machine, section_count, _, _, _, optional_size, _ = struct.unpack_from("<HHIIIHH", data, pe + 4)
    optional = pe + 24
    sections_offset = optional + optional_size
    magic = struct.unpack_from("<H", data, optional)[0]
    if magic == 0x10B:
        image_base = struct.unpack_from("<I", data, optional + 28)[0]
        directory_offset = optional + 96
    elif magic == 0x20B:
        image_base = struct.unpack_from("<Q", data, optional + 24)[0]
        directory_offset = optional + 112
    else:
        raise ValueError(f"unknown PE optional header {magic:#x}")
    if directory_offset > sections_offset:
        raise ValueError("truncated PE optional header")
    directory_count = struct.unpack_from("<I", data, directory_offset - 4)[0]
    if directory_offset + 8 * directory_count > sections_offset:
        raise ValueError("truncated PE data directories")
    headers_size = struct.unpack_from("<I", data, optional + 60)[0]
    sections = [struct.unpack_from("<III", data, sections_offset + i * 40 + 12)
                for i in range(section_count)]

    def file_span(rva):
        if 0 < rva < headers_size <= len(data):
            return rva, headers_size
        for address, raw_size, raw_start in sections:
            if address <= rva < address + raw_size and raw_start + raw_size <= len(data):
                return raw_start + rva - address, raw_start + raw_size
        raise ValueError(f"unmapped PE address {rva:#x}")

    imports = set()
    for index, words, name_field in ((1, 5, 3), (13, 8, 1)):
        if directory_count <= index:
            continue
        rva, size = struct.unpack_from("<II", data, directory_offset + index * 8)
        if not rva and not size:
            continue
        if not rva:
            raise ValueError("missing PE import directory address")
        start, end = file_span(rva)
        if start + size > end:
            raise ValueError("truncated PE import directory")
        width = words * 4
        for offset in range(start, start + size - width + 1, width):
            descriptor = struct.unpack_from(f"<{words}I", data, offset)
            if not any(descriptor):
                break
            name_rva = descriptor[name_field]
            if index == 13 and not descriptor[0] & 1:
                name_rva -= image_base
            name_start, name_limit = file_span(name_rva)
            name_end = data.find(b"\0", name_start, name_limit)
            if name_end <= name_start:
                raise ValueError("empty or unterminated PE import name")
            imports.add(data[name_start:name_end].decode("ascii").casefold())
        else:
            raise ValueError("unterminated PE import directory")
    return machine, imports


def check_binaries(binaries, runtimes):
    parsed = {}
    shipped = set()
    errors = []
    for path, data in binaries:
        if path.is_absolute() or any(
            part.endswith((".", " ")) or any(char in part for char in '\\:<>"|?*')
            for part in path.parts
        ):
            raise ValueError(f"cannot resolve Windows binary path {path}")
        parent, name = path.parent.as_posix().lower(), path.name.lower()
        if (parent, name) in parsed:
            raise ValueError(f"ambiguous Windows binary path {path}")
        machine, imports = None, ()
        try:
            machine, imports = pe_imports(data)
        except (ValueError, struct.error) as error:
            errors.append(f"{path}: invalid PE ({error})")
        parsed[parent, name] = path, machine, imports
        if machine is not None and path.suffix.casefold() == ".dll":
            shipped.add((parent, name, machine))

    for (parent, _), (path, machine, imports) in parsed.items():
        allowed = WINDOWS_DLLS
        if machine == 0x8664 and "vcredist143-x64" in runtimes:
            allowed |= VC143_X64_DLLS
        for dll in sorted(imports):
            if (dll not in allowed and not dll.startswith(("api-ms-win-", "ext-ms-"))
                    and (parent, dll, machine) not in shipped):
                errors.append(f"{path}: missing imported DLL {dll}")
    return errors


def check_release(directory):
    packages = sorted(directory.glob("*-full.nupkg"))
    if len(packages) != 1:
        raise ValueError("expected exactly one Velopack full package")
    with ZipFile(packages[0]) as package:
        members = {entry: PurePosixPath(entry.filename) for entry in package.infolist()
                   if not entry.is_dir()}
        manifests = [entry for entry, path in members.items()
                     if path.parent == PurePosixPath(".") and path.suffix == ".nuspec"]
        if len(manifests) != 1:
            raise ValueError("expected exactly one package manifest")
        metadata = ElementTree.fromstring(package.read(manifests[0]))
        # Velopack writes the pack --framework argument into runtimeDependencies.
        runtime_list = metadata.findtext("{*}metadata/{*}runtimeDependencies", default="")
        runtimes = {runtime.strip().casefold() for runtime in runtime_list.split(",")}
        binaries = {entry: path for entry, path in members.items() if path.suffix.casefold() in PE_SUFFIXES}
        if not binaries:
            raise ValueError("full package contains no Windows binaries")
        errors = check_binaries(((path, package.read(entry)) for entry, path in binaries.items()), runtimes)

    # The setup executable must load before it can install any prerequisites.
    outer = sorted(path for path in directory.rglob("*") if path.is_file() and path.suffix.casefold() in PE_SUFFIXES)
    errors.extend(check_binaries(((path.relative_to(directory), path.read_bytes()) for path in outer), ()))
    return errors, len(binaries) + len(outer)


def fixture_pe(dll=None, *, delay=False, machine=0x8664):
    data = bytearray(0x400)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", data, 0x84, machine, 1, 0, 0, 0, 240, 0x2000)
    optional = 0x98
    struct.pack_into("<H", data, optional, 0x20B)
    struct.pack_into("<I", data, optional + 60, 0x200)
    struct.pack_into("<I", data, optional + 108, 16)
    section = optional + 240
    struct.pack_into("<8sIIII", data, section, b".rdata", 0x200, 0x1000, 0x200, 0x200)
    if dll:
        index, size = (13, 64) if delay else (1, 40)
        struct.pack_into("<II", data, optional + 112 + index * 8, 0x1000, size)
        if delay:
            struct.pack_into("<II", data, 0x200, 1, 0x1100)
        else:
            struct.pack_into("<I", data, 0x20C, 0x1100)
        data[0x300:0x300 + len(dll)] = dll.encode("ascii")
    return data


def assert_check(root, label, diagnostic=None):
    output = io.StringIO()
    with redirect_stderr(output), redirect_stdout(output):
        result = main([str(root)])
    expected = int(diagnostic is not None)
    if result != expected or (diagnostic and diagnostic not in output.getvalue()):
        raise AssertionError(f"{label}: expected exit {expected}, got {result}\n{output.getvalue()}")


def write_fixture(archive, files, runtimes=""):
    manifest = (
        '<package xmlns="http://schemas.microsoft.com/packaging/2010/07/nuspec.xsd">'
        f"<metadata><runtimeDependencies>{runtimes}</runtimeDependencies></metadata></package>"
    )
    with ZipFile(archive, "w", compression=ZIP_DEFLATED) as package:
        package.writestr("fixture.nuspec", manifest)
        for path, data in files:
            package.writestr(path, data)


def self_test():
    cases = (
        ("VCRUNTIME140.dll", False, "", True),
        ("VCRUNTIME140.dll", False, "vcredist143-x64", False),
        ("VCRUNTIME140.dll", True, "vcredist143-x86", True),
        ("VCRUNTIME140.dll", True, "webview2,VCREDIST143-X64", False),
        ("concrt140.dll", False, "vcredist143-x64", False),
        ("unknown.dll", False, "vcredist143-x64", True),
        ("unknown.dll", True, "", True),
        ("Kernel32.DLL", False, "", False),
        ("api-ms-win-crt-runtime-l1-1-0.dll", False, "", False),
        ("ext-ms-win-test-l1-1-0.dll", True, "", False),
    )
    layouts = (
        ("subfolder copy", "plugins/UNKNOWN.DLL", 0x8664, True),
        ("wrong machine", "UNKNOWN.DLL", 0xAA64, True),
        ("matching copy", "UNKNOWN.DLL", 0x8664, False),
    )
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        archive = root / "fixture-full.nupkg"
        importer = "lib/app/fixture.DLL"
        for dll, delay, runtimes, missing in cases:
            write_fixture(archive, [(importer, fixture_pe(dll, delay=delay))], runtimes)
            diagnostic = f"{importer}: missing imported DLL {dll.casefold()}" if missing else None
            assert_check(root, f"{dll}, delay={delay}, runtimes={runtimes!r}", diagnostic)

        for label, copy, machine, missing in layouts:
            write_fixture(archive, [(importer, fixture_pe("unknown.dll", delay=True)),
                                    (f"lib/app/{copy}", fixture_pe(machine=machine))])
            diagnostic = f"{importer}: missing imported DLL unknown.dll" if missing else None
            assert_check(root, f"package {label}", diagnostic)

        write_fixture(archive, [(importer, fixture_pe())])
        for label, copy, machine, missing in layouts:
            setup, dll = root / "Setup.exe", root / copy
            setup.write_bytes(fixture_pe("unknown.dll"))
            dll.parent.mkdir(parents=True, exist_ok=True)
            dll.write_bytes(fixture_pe(machine=machine))
            diagnostic = "Setup.exe: missing imported DLL unknown.dll" if missing else None
            assert_check(root, f"outer {label}", diagnostic)
            setup.unlink()
            dll.unlink()

        with ZipFile(archive) as package:
            entry = package.getinfo(importer)
        with archive.open("r+b") as stream:
            # ZIP's fixed local header is 30 bytes. This fixture has no extra fields.
            stream.seek(entry.header_offset + 30 + len(importer))
            stream.write(b"\x07")  # Reserved DEFLATE block type.
        assert_check(root, "corrupt deflate member", "Package import check failed: Error")
    print(f"Package import self-test passed ({len(cases) + 2 * len(layouts) + 1} cases).")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("package_directory", type=Path, nargs="?")
    mode.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        self_test()
        return 0
    try:
        errors, count = check_release(args.package_directory)
    except (BadZipFile, ElementTree.ParseError, OSError, ValueError, zlib.error) as error:
        print(f"Package import check failed: {error}", file=sys.stderr)
        return 1
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Checked imports in {count} packaged Windows binaries.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
