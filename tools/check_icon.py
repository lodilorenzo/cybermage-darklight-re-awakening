"""Verify every source ICO frame is embedded in a Windows executable (stdlib only).
Run: python tools/check_icon.py [PATH_TO_EXE]
"""
import ctypes
from ctypes import wintypes
from pathlib import Path
import struct
import sys

PROJECT = Path(__file__).resolve().parents[1]


def check_icon(exe):
    ico = (PROJECT / "assets/cybermage.ico").read_bytes()
    assert struct.unpack_from("<HH", ico) == (0, 1)
    count = struct.unpack_from("<H", ico, 4)[0]
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, ctypes.c_void_p, wintypes.DWORD]
    api.LoadLibraryExW.restype = ctypes.c_void_p
    api.FindResourceW.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    api.FindResourceW.restype = ctypes.c_void_p
    api.LoadResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    api.LoadResource.restype = ctypes.c_void_p
    api.LockResource.argtypes = [ctypes.c_void_p]
    api.LockResource.restype = ctypes.c_void_p
    api.SizeofResource.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    api.SizeofResource.restype = wintypes.DWORD
    api.FreeLibrary.argtypes = [ctypes.c_void_p]
    module = api.LoadLibraryExW(str(Path(exe).resolve()), None, 0x22)  # data/image resources only
    assert module, ctypes.WinError(ctypes.get_last_error())
    try:
        def resource(kind, ident):
            found = api.FindResourceW(module, ident, kind)
            assert found, f"Missing icon resource {kind}/{ident} in {exe}"
            pointer = api.LockResource(api.LoadResource(module, found))
            assert pointer, "Cannot load icon resource"
            return ctypes.string_at(pointer, api.SizeofResource(module, found))
        group = resource(14, 1)  # RT_GROUP_ICON
        assert struct.unpack_from("<HHH", group) == (0, 1, count)
        sizes = set()
        for i in range(count):
            entry = ico[6+i*16:6+i*16+16]
            embedded = group[6+i*14:6+i*14+14]
            # rc.exe normalizes PNG frames' unspecified planes (0) to one.
            assert entry[:4] == embedded[:4] and entry[6:12] == embedded[6:12], "Icon frame metadata differs"
            assert struct.unpack_from("<H", embedded, 4)[0] == 1
            size, offset = struct.unpack_from("<II", entry, 8)
            ident = struct.unpack_from("<H", embedded, 12)[0]
            assert resource(3, ident) == ico[offset:offset+size], "Embedded icon pixels differ"
            sizes.add((entry[0] or 256, entry[1] or 256))
        assert sizes == {(n,n) for n in (16,24,32,48,64,128,256)}
    finally:
        api.FreeLibrary(module)
    print("Embedded Darklight icon: all seven sizes match the source ICO.")


if __name__ == "__main__":
    check_icon(sys.argv[1] if len(sys.argv) > 1 else PROJECT / "target/x86_64-pc-windows-msvc/release/cybermage.exe")
