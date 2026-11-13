"""Generate a GNU import library (.a) for a Windows system DLL.

Parses the DLL's PE export table (stdlib only), writes a .def file, then runs
llvm-dlltool to produce lib<name>.a. Used to fill gaps in rustup's
self-contained MinGW library set (e.g. shlwapi).

Usage: py gen_implib.py <dll-name> <out-dir> <llvm-dlltool-path>
"""

import struct
import subprocess
import sys
from pathlib import Path


def named_exports(path: Path) -> list[str]:
    data = path.read_bytes()
    assert data[:2] == b"MZ", "not a PE file"
    e_lfanew = struct.unpack_from("<I", data, 0x3C)[0]
    assert data[e_lfanew : e_lfanew + 4] == b"PE\0\0"
    coff = e_lfanew + 4
    (num_sections,) = struct.unpack_from("<H", data, coff + 2)
    (opt_size,) = struct.unpack_from("<H", data, coff + 16)
    opt = coff + 20
    (magic,) = struct.unpack_from("<H", data, opt)
    assert magic == 0x20B, "PE32+ (x64) expected"
    exp_rva, _exp_size = struct.unpack_from("<II", data, opt + 112)
    assert exp_rva, "no export table"

    sections = []
    sec0 = opt + opt_size
    for i in range(num_sections):
        s = sec0 + 40 * i
        vsize, vaddr, rsize, roff = struct.unpack_from("<IIII", data, s + 8)
        sections.append((vaddr, max(vsize, rsize), roff))

    def off(rva: int) -> int:
        for vaddr, size, roff in sections:
            if vaddr <= rva < vaddr + size:
                return roff + (rva - vaddr)
        raise ValueError(f"rva {rva:#x} not in any section")

    e = off(exp_rva)
    (n_names,) = struct.unpack_from("<I", data, e + 24)
    (names_rva,) = struct.unpack_from("<I", data, e + 32)
    arr = off(names_rva)
    names = []
    for i in range(n_names):
        (srva,) = struct.unpack_from("<I", data, arr + 4 * i)
        so = off(srva)
        names.append(data[so : data.index(b"\0", so)].decode("ascii"))
    return names


def main() -> None:
    dll_name, out_dir, dlltool = sys.argv[1], Path(sys.argv[2]), sys.argv[3]
    dll_path = Path(r"C:\Windows\System32") / f"{dll_name}.dll"
    names = named_exports(dll_path)
    print(f"{dll_path}: {len(names)} named exports")

    out_dir.mkdir(parents=True, exist_ok=True)
    def_path = out_dir / f"{dll_name}.def"
    def_path.write_text(
        f"LIBRARY {dll_name}.dll\nEXPORTS\n" + "\n".join(names) + "\n",
        encoding="ascii",
    )
    lib_path = out_dir / f"lib{dll_name}.a"
    subprocess.check_call(
        [
            dlltool,
            "-m", "i386:x86-64",
            "-d", str(def_path),
            "-D", f"{dll_name}.dll",
            "-l", str(lib_path),
        ]
    )
    print(f"wrote {lib_path} ({lib_path.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
