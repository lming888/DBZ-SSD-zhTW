#!/usr/bin/env python3
"""Scenario tests for installer/dbz-zhtw.nsi: builds the setup exe and runs it (and the uninstaller it writes) under Wine,
then checks files, registry and exit codes from the macOS side.

    run_wine_tests.py [--only TEXT] [--ref GIT_REF | --nsi FILE] [--auto-update] [--gui [--shots DIR] [--dpi N]] [--keep-work]

Defaults match the tools under ~/Tools (override with flags or env):
  --wine      ~/Tools/wine/11.0_1/wine/bin/wine          (env DBZ_WINE)
  --prefix    ~/Tools/wine/prefix-dbz-installer          (env DBZ_WINEPREFIX; a throwaway prefix, it gets reset)
  --makensis  ~/Tools/nsis/3.13/bin/makensis             (env DBZ_MAKENSIS; NSISDIR = <makensis>/../../share/nsis)
  --old-setups ~/Tools/dbz-old-setups                    (env DBZ_OLD_SETUPS; the released RC1, RC2, RC3, v1.0 installers that the
               upgrade scenarios v01-v03 install first, SHA-256 checked; missing ones fail those scenarios, they are not skipped:
               gh release download rc1 -R lming888/DBZ-SSD-zhTW -p '*setup.exe' -D <folder>, and rc2, rc3, v1.0)

--ref builds installer/ from another commit (git archive). A commit without /DELETEDATA support
gets two edits so its "delete player data" section can run silently at all: the section is selected by default and the
${Silent} early return is removed. Nothing else changes, so the old data logic itself is what gets tested — but with
that edit the old uninstaller deletes player data even without /DELETEDATA, so its u01 failure comes from the edit and
is no evidence about the old version.

--nsi builds from another copy of dbz-zhtw.nsi (it must sit in an installer/ folder next to launcher/, like the repo's;
used for mutation checks).

Not testable under Wine (listed in installer/README.md as Windows-only checks): an exe locked because it is running
(tested here with a file held open without delete sharing, which Wine does enforce), NTFS junctions / symbolic links
(Wine has no NTFS reparse points), volume serial numbers (Wine reports 0 or a per-drive value), 8.3 short names on a real volume.
Exit 0 when every scenario passes."""
import argparse
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
INSTALLER_DIR = HERE.parent
REPO = INSTALLER_DIR.parent

APP_ID = "DBZ-SSD-zhTW"
EXE_NAME = "DBZ_超賽亞傳說_繁中.exe"
BLOCKED_TEST_DIR = "C:\\windows\\dbz-blocked-test"   # g09: an install inside the Windows folder
LAUNCHER_CALLS = "C:\\dbztest\\logs\\launcher-calls.txt"   # Build.recorder: what the finish page started
EXE_STEM = "DBZ_超賽亞傳說_繁中"
UNINST = "uninstall-dbz-zhtw.exe"
PRODUCT = "七龍珠Z 超賽亞傳說 繁中版"
HDPACK_LNK = "下載 HD 美術包"
MARKER = ".dbz-ssd-zhtw-install"
SYNC_MARKER = ".launcher-sync.toml"
UNINST_ROOT = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall"
UNINST_KEY = UNINST_ROOT + "\\" + APP_ID
OWN_KEY = "HKCU\\Software\\" + APP_ID
# What the installer itself puts in the install folder (dbz-zhtw.nsi: the marker first, then the launcher and the
# uninstaller). Anything else at the top of an install folder is the player's (save\, hd-pack.zip) or left over.
PROGRAM_FILES = {EXE_NAME, UNINST, MARKER}
# The upgrade scenarios (v01-v04): the installer under test is built a second time from a copy whose
# launcher/RELEASE_NAME is NEXT_NAME and RELEASE is NEXT_RELEASE, so "the new version" is told apart from the old
# installers, which are the released ones (RC1, RC2, RC3, v1.0), checked against the SHA-256 of their Release's
# SHA256SUMS_public.txt. They are not in the repo: --old-setups names the folder that holds them.
NEXT_NAME = "v1.0.1"
NEXT_RELEASE = "51"
OLD_SETUPS = {
    "RC1": ("DBZ-SSD-zh-TW-RC1-setup.exe", "8b83a3d9c5181d911280f3c9a9c64d93d51e5deee9376c0fd52a3b31a9119bca", "45"),
    "RC2": ("DBZ-SSD-zh-TW-RC2-setup.exe", "7f05ad0ff00b528a137f45ed2401055e212860b8d8dc65f490e2f12a1d66a871", "46"),
    "RC3": ("DBZ-SSD-zh-TW-RC3-setup.exe", "2037f62b49cf28c4b277526a1c785c146e17eb6cf84a0daf7478712290c29529", "47"),
    "v1.0": ("DBZ-SSD-zh-TW-v1.0-setup.exe", "5a056bc6d90be93d7603acc67a2ae51bfef916e47feaeb5d0f3e44af6d965f37", "50"),
}

HOME = Path.home()


# ----------------------------------------------------------------------------------------------------------------------
# Wine helpers
# ----------------------------------------------------------------------------------------------------------------------

class Wine:
    def __init__(self, wine: Path, prefix: Path):
        self.wine = wine
        self.wineserver = wine.parent / "wineserver"
        self.prefix = prefix
        self.c = prefix / "drive_c"
        users = [p for p in (self.c / "users").iterdir() if p.is_dir() and p.name.lower() != "public"]
        if len(users) != 1:
            raise SystemExit(f"prefix 裡應該只有一個使用者資料夾：{[p.name for p in users]}")
        self.user = users[0]
        self.win_user = "C:\\users\\" + self.user.name
        self.env = dict(os.environ)
        self.env.update({
            "WINEPREFIX": str(prefix),
            "WINEDEBUG": "-all",
            # winemenubuilder would mirror Start-menu / desktop shortcuts onto the Mac (~/.local/share, ~/Desktop)
            "WINEDLLOVERRIDES": "winemenubuilder.exe=d;mscoree,mshtml=",
            "MVK_CONFIG_LOG_LEVEL": "0",
        })

    def guard_host_isolation(self):
        """Refuse to run when the prefix's user folders still point at the Mac home (shortcuts would land there)."""
        for name in ("Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos"):
            p = self.user / name
            if p.is_symlink():
                raise SystemExit(f"{p} 是連到 Mac 的連結（{os.readlink(p)}）：先換成 prefix 內的一般資料夾再跑")

    def start_server(self):
        """One persistent wineserver for the whole run: a cold start per step (and waiting for it to shut down
        afterwards) costs several seconds each. `wine x.exe` itself returns when x.exe exits, with its exit code."""
        subprocess.run([str(self.wineserver), "-p"], env=self.env, timeout=60)

    def stop_server(self):
        subprocess.run([str(self.wineserver), "-k"], env=self.env, timeout=60)

    def run(self, exe: Path, args: list[str], timeout=180) -> int:
        r = subprocess.run([str(self.wine), str(exe), *args], env=self.env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=timeout)
        return r.returncode

    def popen(self, exe: Path, args: list[str]) -> subprocess.Popen:
        return subprocess.Popen([str(self.wine), str(exe), *args], env=self.env, stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL)

    def reg(self, *args) -> subprocess.CompletedProcess:
        # Output goes to a file, not a pipe: Wine's background processes (services.exe…) inherit the pipe and keep it
        # open, so reading it to EOF would hang
        with tempfile.TemporaryFile() as f:
            r = subprocess.run([str(self.wine), "reg", *args], env=self.env, stdout=f, stderr=subprocess.DEVNULL,
                               timeout=120)
            f.seek(0)
            r.stdout = f.read()
        return r

    def ensure_cjk_font(self):
        """A fresh prefix has no CJK font, so the wizard's Chinese renders as boxes. Copy macOS's Heiti TC into the
        prefix, link it behind the dialog fonts (what winetricks cjkfonts does) and stand it in for 新細明體. Windows
        itself draws 新細明體 (PMingLiU), so screenshots from Wine show a different typeface than a real Windows."""
        fonts = self.c / "windows" / "Fonts"
        for src, dst in (("STHeiti Light.ttc", "stheitil.ttc"), ("STHeiti Medium.ttc", "stheitim.ttc")):
            if not (fonts / dst).exists():
                shutil.copyfile(Path("/System/Library/Fonts") / src, fonts / dst)
        key = r"HKLM\Software\Microsoft\Windows NT\CurrentVersion\FontLink\SystemLink"
        for face in ("Tahoma", "MS Shell Dlg", "MS Shell Dlg 2", "Microsoft Sans Serif", "Segoe UI"):
            self.reg("add", key, "/v", face, "/t", "REG_MULTI_SZ", "/d", "stheitil.ttc,Heiti TC", "/f")
        # The wizard asks for 新細明體 9pt (NSIS's Traditional Chinese language file). Windows has it; without it Wine falls
        # back to a linked font and measures line widths with a different font than it draws with, so a line that
        # fits on Windows runs past the right edge. Give Wine a real font under that name, as Windows has
        replacements = r"HKCU\Software\Wine\Fonts\Replacements"
        for face in ("新細明體", "PMingLiU"):
            self.reg("add", replacements, "/v", face, "/t", "REG_SZ", "/d", "Heiti TC", "/f")

    DPI_KEYS = (r"HKCU\Control Panel\Desktop", r"HKCC\Software\Fonts")

    def dpi_values(self) -> dict:
        """LogPixels under both keys Wine reads it from (None = not set)."""
        values = {}
        for key in self.DPI_KEYS:
            r = self.reg("query", key, "/v", "LogPixels")
            m = re.search(r"LogPixels\s+REG_DWORD\s+(0x[0-9a-fA-F]+)", r.stdout.decode("utf-8", "replace"))
            values[key] = int(m.group(1), 16) if r.returncode == 0 and m else None
        return values

    def set_dpi(self, dpi: int):
        """Display scaling for the run (read dpi_values() first, for restore_dpi). Restarts the server to apply it."""
        for key in self.DPI_KEYS:
            self.reg("add", key, "/v", "LogPixels", "/t", "REG_DWORD", "/d", str(dpi), "/f")
        self.stop_server()
        self.start_server()

    def restore_dpi(self, before: dict):
        for key, value in before.items():
            if value is None:
                self.reg("delete", key, "/v", "LogPixels", "/f")
            else:
                self.reg("add", key, "/v", "LogPixels", "/t", "REG_DWORD", "/d", str(value), "/f")

    def reg_get(self, value: str):
        r = self.reg("query", UNINST_KEY, "/v", value)
        if r.returncode != 0:
            return None
        text = r.stdout.decode("utf-8", "replace")
        m = re.search(rf"^\s*{re.escape(value)}\s+REG_\w+\s+(.*?)\s*$", text, re.M)
        return m.group(1) if m else None

    def attrib(self, win: str) -> str:
        """The DOS attribute letters of `win` from Wine's attrib (e.g. "A  H")."""
        with tempfile.TemporaryFile() as f:
            subprocess.run([str(self.wine), "cmd", "/c", "attrib", win], env=self.env, stdout=f,
                           stderr=subprocess.DEVNULL, timeout=120)
            f.seek(0)
            line = f.read().decode("utf-8", "replace").strip().splitlines()
        return line[0][:10] if line else ""

    def own_get(self, value: str):
        r = self.reg("query", OWN_KEY, "/v", value)
        if r.returncode != 0:
            return None
        m = re.search(rf"^\s*{re.escape(value)}\s+REG_\w+\s+(.*?)\s*$", r.stdout.decode("utf-8", "replace"), re.M)
        return m.group(1) if m else None

    def reg_key_exists(self) -> bool:
        return self.reg("query", UNINST_KEY).returncode == 0

    def subkeys(self, key: str) -> list[str]:
        """The names of `key`'s direct subkeys (ASCII names; Wine's reg prints others as ?)."""
        r = self.reg("query", key)
        if r.returncode != 0:
            return []
        prefix = key.split("\\", 1)[1].lower()
        names = []
        for line in r.stdout.decode("utf-8", "replace").splitlines():
            line = line.strip()
            head, _, rest = line.partition("\\")
            if head.upper() in ("HKEY_CURRENT_USER", "HKCU") and rest.lower().startswith(prefix + "\\"):
                names.append(rest[len(prefix) + 1:])
        return names

    def reg_set(self, value: str, data: str):
        self.reg("add", UNINST_KEY, "/v", value, "/t", "REG_SZ", "/d", data, "/f")

    def reg_delete(self):
        self.reg("delete", UNINST_KEY, "/f")
        self.reg("delete", OWN_KEY, "/f")

    def u(self, win: str) -> Path:
        """Windows path on C: or X: (the test subst drive) -> macOS path."""
        m = re.match(r"^([A-Za-z]):\\?(.*)$", win)
        if not m:
            raise ValueError(win)
        drive, rest = m.group(1).lower(), m.group(2)
        base = (self.prefix / "dosdevices" / f"{drive}:").resolve()
        return base.joinpath(*[p for p in rest.split("\\") if p])

    # well-known folders
    @property
    def localappdata(self) -> str:
        return self.win_user + "\\AppData\\Local"

    @property
    def default_dir(self) -> str:
        return self.localappdata + "\\Programs\\" + APP_ID

    @property
    def startmenu_dir(self) -> Path:
        return self.user / "AppData/Roaming/Microsoft/Windows/Start Menu/Programs" / PRODUCT

    @property
    def desktop_lnk(self) -> Path:
        return self.user / "Desktop" / f"{PRODUCT}.lnk"


def force_rmtree(p: Path, root: Path):
    """Delete a test tree inside the Wine prefix (refuses anything outside `root`)."""
    p = Path(p)
    if not p.exists() and not p.is_symlink():
        return
    if root.resolve() not in p.resolve().parents and p.resolve() != root.resolve():
        if not p.is_symlink():
            raise RuntimeError(f"拒絕刪除 prefix 以外的路徑：{p}")
    if p.is_symlink() or p.is_file():
        p.unlink()
        return
    for d, dirs, files in os.walk(p):
        os.chmod(d, 0o755)
        for f in files:
            fp = Path(d) / f
            if not fp.is_symlink():
                os.chmod(fp, 0o644)
    shutil.rmtree(p)


# ----------------------------------------------------------------------------------------------------------------------
# Build
# ----------------------------------------------------------------------------------------------------------------------

LEGACY_PATCHES = [
    ('Section /o "un.同時刪除存檔、ROM 設定、HD 美術包與遊戲設定" SEC_UN_DATA',
     'Section "un.同時刪除存檔、ROM 設定、HD 美術包與遊戲設定" SEC_UN_DATA'),
    ('  ${If} ${Silent}\n  ${OrIf} $UnDataBlocked != ""\n', '  ${If} $UnDataBlocked != ""\n'),
]


class Build:
    def __init__(self, makensis: Path, work: Path):
        self.makensis = makensis
        self.nsisdir = makensis.parent.parent / "share" / "nsis"
        self.work = work
        self.uncopies = 0  # uninstaller copies made so far (each run gets its own folder)
        self.drives = 0  # guidrive args files written so far
        self.old_setups = None  # --old-setups: the folder of the released RC1, RC2, RC3, v1.0 installers (upgrade scenarios)

    def compile(self, nsi: Path, defines: dict, strict=True) -> None:
        cmd = [str(self.makensis), "-V2"] + (["-WX"] if strict else [])
        cmd += [f"-D{k}={v}" if v is not None else f"-D{k}" for k, v in defines.items()]
        cmd.append(str(nsi))
        env = dict(os.environ, NSISDIR=str(self.nsisdir))
        r = subprocess.run(cmd, env=env, capture_output=True, cwd=nsi.parent)
        if r.returncode != 0:
            out = (r.stdout + r.stderr).decode("utf-8", "replace")
            raise SystemExit(f"makensis 失敗（{nsi}）：\n{out[-3000:]}")

    def standin(self) -> Path:
        out = self.work / "standin.exe"
        nsi = self.work / "standin.nsi"
        nsi.write_text(f'Unicode true\nOutFile "{out}"\nSilentInstall silent\nRequestExecutionLevel user\n'
                       'Section\nSectionEnd\n', encoding="utf-8")
        self.compile(nsi, {}, strict=False)
        return out

    def recorder(self) -> Path:
        """Stand-in launcher that appends `[<its parameters>]` to C:\\dbztest\\logs\\launcher-calls.txt: what the
        finish page started (the game, `--hd-pack`) and in which order."""
        out = self.work / "recorder.exe"
        if not out.exists():
            nsi = self.work / "recorder.nsi"
            nsi.write_text(f'Unicode true\nOutFile "{out}"\nSilentInstall silent\nRequestExecutionLevel user\n'
                           '!include "FileFunc.nsh"\nSection\n  ${GetParameters} $0\n'
                           f'  FileOpen $9 "{LAUNCHER_CALLS}" a\n  FileSeek $9 0 END\n'
                           '  FileWrite $9 "[$0]$\\r$\\n"\n  FileClose $9\nSectionEnd\n', encoding="utf-8")
            self.compile(nsi, {}, strict=False)
        return out

    def recording_setup(self, nsi: Path) -> Path:
        """The setup exe with the recorder as its launcher (finish page scenarios)."""
        out = self.work / "DBZ-SSD-zh-TW-RC1-recorder-test.exe"
        if not out.exists():
            self.compile(nsi, {"LAUNCHER_EXE": self.recorder(), "RELEASE": "45", "OUTFILE": out})
        return out

    def portable_launcher(self) -> Path:
        """Stand-in for the portable 7z release's exe: carries the launcher's version fields (launcher/build.rs)."""
        out = self.work / "portable-launcher.exe"
        if not out.exists():
            nsi = self.work / "portable.nsi"
            nsi.write_text(f'Unicode true\nOutFile "{out}"\nSilentInstall silent\nRequestExecutionLevel user\n'
                           'VIProductVersion "44.0.0.0"\n'
                           'VIAddVersionKey /LANG=1028 "ProductName" "DBZ 超賽亞傳說 繁中版"\n'
                           'VIAddVersionKey /LANG=1028 "FileVersion" "44.0.0"\n'
                           'VIAddVersionKey /LANG=1028 "FileDescription" "test"\n'
                           'VIAddVersionKey /LANG=1028 "InternalName" "dbz-zhtw-launcher"\n'
                           f'VIAddVersionKey /LANG=1028 "OriginalFilename" "{EXE_NAME}"\n'
                           'Section\nSectionEnd\n', encoding="utf-8")
            self.compile(nsi, {}, strict=False)
        return out

    def old_uninstaller(self) -> Path:
        """Stand-in for an earlier test build's uninstall.exe: carries this program's ProductName, as those did."""
        out = self.work / "old-uninstall.exe"
        if not out.exists():
            nsi = self.work / "old-uninstall.nsi"
            nsi.write_text(f'Unicode true\nOutFile "{out}"\nSilentInstall silent\nRequestExecutionLevel user\n'
                           'VIProductVersion "44.0.0.0"\n'
                           f'VIAddVersionKey /LANG=1028 "ProductName" "{PRODUCT}"\n'
                           'VIAddVersionKey /LANG=1028 "FileVersion" "v44"\n'
                           f'VIAddVersionKey /LANG=1028 "FileDescription" "{PRODUCT} 安裝程式"\n'
                           'Section\nSectionEnd\n', encoding="utf-8")
            self.compile(nsi, {}, strict=False)
        return out

    def source_tree(self, ref):
        """installer/ to build from: the working tree, or `ref` extracted (and patched when it predates /DELETEDATA)."""
        if ref is None:
            return INSTALLER_DIR / "dbz-zhtw.nsi", []
        dest = self.work / "ref"
        dest.mkdir()
        paths = ["installer", "launcher/icon"]
        # an older installer/ runs launcher/publish_check.py (which imports attach_overlay.py) when it compiles:
        # taken along when the ref has them
        for extra in ("launcher/publish_check.py", "launcher/attach_overlay.py"):
            if subprocess.run(["git", "-C", str(REPO), "cat-file", "-e", f"{ref}:{extra}"],
                              capture_output=True).returncode == 0:
                paths.append(extra)
        arc = subprocess.run(["git", "-C", str(REPO), "archive", ref, *paths], capture_output=True, check=True)
        subprocess.run(["tar", "-x", "-C", str(dest)], input=arc.stdout, check=True)
        nsi = dest / "installer" / "dbz-zhtw.nsi"
        text = nsi.read_text(encoding="utf-8")
        notes = []
        if "/DELETEDATA" not in text:
            for old, new in LEGACY_PATCHES:
                if text.count(old) != 1:
                    raise SystemExit(f"{ref} 的 dbz-zhtw.nsi 找不到要修改的片段：{old!r}")
                text = text.replace(old, new)
            nsi.write_text(text, encoding="utf-8")
            notes.append(f"{ref} 沒有 /DELETEDATA：已改成刪資料區段預設勾選、拿掉 ${{Silent}} 防線（其餘不動）")
        return nsi, notes

    def setup(self, nsi: Path, auto_update: bool) -> Path:
        launcher = self.standin()
        out = self.work / "DBZ-SSD-zh-TW-RC1-test.exe"  # "setup.exe" trips makensis warning 9000 (compat shims)
        d = {"LAUNCHER_EXE": launcher, "RELEASE": "45", "OUTFILE": out}
        if auto_update:
            d["AUTO_UPDATE"] = None
        self.compile(nsi, d)
        return out

    def next_setup(self, nsi: Path) -> Path:
        """The installer under test (`nsi`) as the next release: built from a copy of its installer/ folder whose
        launcher/RELEASE_NAME is NEXT_NAME, with RELEASE=NEXT_RELEASE and the stand-in launcher (the real launcher needs
        the Windows game build, which this Mac cannot make). Only the name, the number and the launcher differ from the
        installer the other scenarios run."""
        out = self.work / "DBZ-SSD-zh-TW-next-test.exe"
        if not out.exists():
            root = self.work / "next"
            shutil.copytree(nsi.parent, root / "installer", ignore=shutil.ignore_patterns("tests"))
            shutil.copytree(nsi.parent.parent / "launcher" / "icon", root / "launcher" / "icon")
            (root / "launcher" / "RELEASE_NAME").write_text(NEXT_NAME + "\n", encoding="utf-8")
            self.compile(root / "installer" / nsi.name,
                         {"LAUNCHER_EXE": self.standin(), "RELEASE": NEXT_RELEASE, "OUTFILE": out})
        return out

    def old_setup(self, name: str) -> Path:
        """The released installer `name` (RC1, RC2, RC3, v1.0) from --old-setups, its SHA-256 checked against the Release's."""
        file, sha, _ = OLD_SETUPS[name]
        where = self.old_setups / file if self.old_setups else None
        if where is None or not where.is_file():
            raise RuntimeError(f"找不到 {file}（--old-setups／DBZ_OLD_SETUPS：{self.old_setups}）。取得方式："
                               f"gh release download {name.lower()} -R lming888/DBZ-SSD-zhTW -p '*setup.exe' -D <資料夾>")
        import hashlib
        got = hashlib.sha256(where.read_bytes()).hexdigest()
        if got != sha:
            raise RuntimeError(f"{where} 的 SHA-256 是 {got}，不是 Release {name} 的 {sha}")
        return where

    def winlist(self) -> Path:
        out = self.work / "winlist"
        if not out.exists():
            r = subprocess.run(["swiftc", "-O", "-o", str(out), str(HERE / "winlist.swift")], capture_output=True)
            if r.returncode != 0:
                raise SystemExit("swiftc 失敗：\n" + (r.stdout + r.stderr).decode("utf-8", "replace")[-2000:])
        return out

    def guidrive(self) -> Path:
        out = self.work / "guidrive.exe"
        if not out.exists():
            self.compile(HERE / "guidrive.nsi", {"OUTFILE": out})
        return out

    def probe(self, nsi: Path, lines: list[str], out_win: str) -> Path:
        """A small exe built from the FileId / NormPath / SamePath macros exactly as written in `nsi`."""
        text = nsi.read_text(encoding="utf-8")
        macros = []
        for name in ("FILE_ID_FUNC", "NORM_PATH_FUNC", "SAME_PATH_FUNC"):
            m = re.search(rf"^!macro {name} UN\n.*?^!macroend\n", text, re.S | re.M)
            if not m:
                raise SystemExit(f"{nsi} 裡找不到 !macro {name}")
            macros.append(m.group(0) + f'!insertmacro {name} ""\n')
        body = []
        for ln in lines:
            kind, *paths = ln.split("\t")
            if kind == "fileid":
                body.append(f'  Push "{paths[0]}"\n  Call FileId\n  Pop $0\n'
                            f'  FileWriteUTF16LE $9 "fileid$\\t{paths[0]}$\\t$0$\\r$\\n"\n')
            else:
                body.append(f'  Push "{paths[0]}"\n  Push "{paths[1]}"\n  Call SamePath\n  Pop $0\n'
                            f'  FileWriteUTF16LE $9 "same$\\t{paths[0]}$\\t{paths[1]}$\\t$0$\\r$\\n"\n')
        out = self.work / "probe.exe"
        src = self.work / "probe.nsi"
        src.write_text(
            f'Unicode true\nOutFile "{out}"\nSilentInstall silent\nRequestExecutionLevel user\n'
            '!include "LogicLib.nsh"\n' + "".join(macros) +
            f'Section\n  FileOpen $9 "{out_win}" w\n  FileWriteWord $9 0xFEFF\n' + "".join(body) +
            '  FileClose $9\nSectionEnd\n', encoding="utf-8")
        self.compile(src, {}, strict=False)
        return out


# ----------------------------------------------------------------------------------------------------------------------
# Scenario framework
# ----------------------------------------------------------------------------------------------------------------------

SCENARIOS = []


def scenario(title, gui=False):
    def deco(fn):
        SCENARIOS.append((fn.__name__, title, fn, gui))
        return fn
    return deco


class Ctx:
    T = "C:\\dbztest"

    def __init__(self, wine: Wine, build: Build, setup_exe: Path, nsi: Path, shots: Path = None):
        self.w = wine
        self.build = build
        self.setup_exe = setup_exe
        self.nsi = nsi
        self.shots = shots
        self.checks = []

    # -- checks --
    def check(self, ok, what, detail=""):
        self.checks.append((bool(ok), what, detail))
        return ok

    def eq(self, got, want, what):
        return self.check(got == want, what, f"得到 {got!r}，應為 {want!r}")

    def exists(self, win, what=None, want=True):
        p = self.w.u(win)
        there = p.exists() or p.is_symlink()
        return self.check(there == want, what or (f"存在：{win}" if want else f"不存在：{win}"),
                          "實際存在" if there else "實際不存在")

    def log_has(self, log_win, text, what=None, want=True):
        content = self.read_log(log_win)
        return self.check((text in content) == want, what or f"紀錄{'有' if want else '沒有'}「{text}」",
                          content[-600:] if content else "（紀錄是空的或不存在）")

    def read_log(self, log_win) -> str:
        p = self.w.u(log_win)
        if not p.exists():
            return ""
        return p.read_bytes().decode("utf-16", "replace")

    # -- fixtures --
    def reset(self):
        w = self.w
        try:
            os.unlink(w.prefix / "dosdevices" / "x:")
        except FileNotFoundError:
            pass
        for win in (self.T, "C:\\" + APP_ID, BLOCKED_TEST_DIR, w.default_dir, w.localappdata + "\\dbz-ssd-zhtw",
                    w.localappdata + "\\dbz-ssd-recompiled"):
            force_rmtree(w.u(win), w.c)
        force_rmtree(w.startmenu_dir, w.c)
        for p in (w.desktop_lnk, w.user / "AppData/Roaming/Microsoft/Windows/Start Menu/Programs" / f"{PRODUCT}.lnk"):
            if p.exists():
                p.unlink()
        w.reg_delete()
        # a mutant with another APP_ID (the upgrade scenarios' mutation check) leaves its own key and default folder
        for root in (UNINST_ROOT, "HKCU\\Software"):
            for name in w.subkeys(root):
                if name.startswith(APP_ID) and name != APP_ID:
                    w.reg("delete", root + "\\" + name, "/f")
        programs = w.u(w.localappdata + "\\Programs")
        if programs.is_dir():
            for p in programs.iterdir():
                if p.name.startswith(APP_ID):
                    force_rmtree(p, w.c)
        w.u(self.T + "\\logs").mkdir(parents=True)
        w.u(self.T + "\\src").mkdir(parents=True)

    def write(self, win, data: bytes = b"x"):
        p = self.w.u(win)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        return p

    def install(self, d=None, *extra, log="C:\\dbztest\\logs\\setup.log", exe=None) -> int:
        """Silent install with this run's setup exe (or `exe`); without `d`, where the installer itself picks."""
        args = ["/S", f"/LOG={log}", *extra]
        if d is not None:
            args.append(f"/D={d}")
        return self.w.run(exe or self.setup_exe, args)

    def snapshot(self, win) -> dict:
        """{relative path: bytes (None for a folder)} of everything under `win`, to prove nothing changed."""
        root = self.w.u(win)
        snap = {}
        for d, dirs, files in os.walk(root):
            for n in dirs:
                snap[os.path.relpath(os.path.join(d, n), root)] = None
            for n in files:
                fp = os.path.join(d, n)
                snap[os.path.relpath(fp, root)] = Path(fp).read_bytes()
        return snap

    def unchanged(self, win, before, what, ignore=()):
        after = {k: v for k, v in self.snapshot(win).items() if not any(k == i or k.startswith(i + os.sep) for i in ignore)}
        diff = sorted(set(before) ^ set(after)) + sorted(k for k in before if k in after and before[k] != after[k])
        return self.check(not diff, what, f"有變動：{diff[:8]}")

    def hold(self, win, ms, share=1) -> subprocess.Popen:
        """Keep `win` open for `ms` ms with this share mode (1: others may only read; 3: read and write, never delete)."""
        self.build.drives += 1
        args = self.T + f"\\gui\\args{self.build.drives}.txt"
        self.write(args, b"\xff\xfe" + f"CMD=hold\r\nOUT={win}\r\nTEXT={ms}\r\nMODE={share}\r\n".encode("utf-16-le"))
        p = self.w.popen(self.build.guidrive(), [args])
        time.sleep(2)
        return p

    def marker(self, d) -> str:
        p = self.w.u(d + "\\" + MARKER)
        return p.read_text("utf-8", "replace") if p.exists() else ""

    def install_raw(self, tail: str) -> int:
        """Run the setup exe with `tail` as its command line exactly as written (guidrive exec: no re-quoting)."""
        exe = self.w.u(self.T + "\\bin\\DBZ-SSD-zh-TW-RC1-test.exe")
        exe.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(self.setup_exe, exe)
        return self.drive("exec", LINE='"' + self.T + '\\bin\\DBZ-SSD-zh-TW-RC1-test.exe" ' + tail)

    def drive(self, cmd, timeout=60, **kv) -> int:
        """guidrive.exe with CMD=cmd and KEY=value lines (TITLE, KIND, ID, TEXT, OUT, LINE) in its args file."""
        self.build.drives += 1
        args = self.T + f"\\gui\\args{self.build.drives}.txt"
        lines = [f"CMD={cmd}"] + [f"{k}={v}" for k, v in kv.items() if v is not None]
        self.write(args, b"\xff\xfe" + "\r\n".join(lines).encode("utf-16-le") + "\r\n".encode("utf-16-le"))
        return self.w.run(self.build.guidrive(), [args], timeout=timeout)

    def uninstall(self, d, *extra, log="C:\\dbztest\\logs\\un.log", exe=None) -> int:
        """Run the uninstaller the way NSIS itself does (a copy elsewhere with _?=<dir>), so it can delete itself."""
        src = exe or self.w.u(d + "\\" + UNINST)
        self.build.uncopies += 1
        copy = self.build.work / f"un{self.build.uncopies}" / "Au_.exe"
        copy.parent.mkdir()
        shutil.copyfile(src, copy)
        return self.w.run(copy, ["/S", f"/LOG={log}", *extra, f"_?={d}"])

    def uninstall_self(self, d, *extra, log="C:\\dbztest\\logs\\un.log") -> int:
        """Run <d>\\uninstall-dbz-zhtw.exe itself with _?=<d> (as a script would, to get the exit code): it cannot delete itself."""
        return self.w.run(self.w.u(d + "\\" + UNINST), ["/S", f"/LOG={log}", *extra, f"_?={d}"])

    def populate_player_data(self, d):
        """What the launcher leaves behind after being used: save tree, HD pack files, its own folder."""
        self.write(d + "\\save\\" + SYNC_MARKER, b"sync")
        self.write(d + "\\save\\.launcher-art", b"art notes")
        self.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
        self.write(d + "\\save\\cache\\rom\\canonical.sfc", b"rom")
        self.write(d + "\\save\\art-override\\a.png", b"png")
        for name in ("hd-pack.zip", "hd-pack.zip.part", "hd-pack.zip.old", "hd-pack.zip.old2", "hd-pack.zip.old99"):
            self.write(d + "\\" + name, b"zip")
        la = self.w.localappdata
        self.write(la + "\\dbz-ssd-zhtw\\game\\data.bin", b"game")
        self.write(la + "\\dbz-ssd-zhtw\\launcher.log", b"log")
        self.write(la + "\\dbz-ssd-recompiled\\upstream.cfg", b"upstream")


# ----------------------------------------------------------------------------------------------------------------------
# Install scenarios
# ----------------------------------------------------------------------------------------------------------------------

def nothing_installed(c: Ctx, what):
    """Neither the default folder nor the registry got anything (NSIS falls back to the default on a bad /D=)."""
    c.exists(c.w.default_dir, f"{what}：預設位置沒有被裝進去", want=False)
    c.check(not c.w.reg_key_exists(), f"{what}：沒有寫登錄")


def marker_state(c: Ctx, d):
    m = re.search(r"^state=(\w+)$", c.marker(d), re.M)
    return m.group(1) if m else None


def entry_ok(c: Ctx, what=""):
    """③ the「新增或移除程式」entry, when there is one, points at an installed folder whose uninstaller is there."""
    if not c.w.reg_key_exists():
        return c.check(True, f"{what}登錄項目：沒有")
    loc = c.w.reg_get("InstallLocation") or ""
    un = (c.w.reg_get("UninstallString") or "").strip('"')
    ok = bool(loc) and marker_state(c, loc) == "installed" and c.w.u(un).exists()
    return c.check(ok, f"{what}登錄項目指向可以解除安裝的安裝", f"InstallLocation={loc} UninstallString={un}")


def roundtrip(c: Ctx, d, what=""):
    """③ not stuck: installs again in place (no sub-folder) and uninstalls again."""
    c.eq(c.install(d, log="C:\\dbztest\\logs\\rt-setup.log"), 0, f"{what}再裝一次 結束碼")
    c.exists(d + "\\" + APP_ID, f"{what}再裝一次沒有建子資料夾", want=False)
    c.eq(marker_state(c, d), "installed", f"{what}再裝一次後標記檔 state=installed")
    c.eq(c.uninstall(d, log="C:\\dbztest\\logs\\rt-un.log"), 0, f"{what}再解除一次 結束碼")


def plant_others(c: Ctx, d):
    """Files that are not on the program's list: in the install folder and in save\\'s root and a sub-folder."""
    c.write(d + "\\notes-mine.txt", b"MINE")
    c.write(d + "\\save\\my-notes.txt", b"MINE2")
    c.write(d + "\\save\\other\\keep.txt", b"KEEP")
    return {k: c.w.u(d + "\\" + k).read_bytes() for k in ("notes-mine.txt", "save\\my-notes.txt", "save\\other\\keep.txt")}


def others_unchanged(c: Ctx, d, others, what=""):
    """① files outside the list not changed by a byte."""
    bad = [k for k, v in others.items() if not c.w.u(d + "\\" + k).exists() or c.w.u(d + "\\" + k).read_bytes() != v]
    return c.check(not bad, f"{what}清單外的檔一個位元組都沒變", f"變了：{bad}")


def saves_in_place(c: Ctx, d, what=""):
    """④ the player's save still where the launcher reads it (<install>\\save\\saves\\ssd.sav)."""
    p = c.w.u(d + "\\save\\saves\\ssd.sav")
    return c.check(p.exists() and p.read_bytes() == b"BATTERY", f"{what}存檔還在 {d}\\save\\saves\\ssd.sav")


@scenario("預設位置靜默安裝：檔案、標記（state=installed）、解除安裝程式、登錄、上次位置、捷徑")
def s01_default(c: Ctx):
    rc = c.install()
    d = c.w.default_dir
    c.eq(rc, 0, "結束碼")
    c.exists(d + "\\" + EXE_NAME)
    c.exists(d + "\\" + UNINST)
    c.exists(d + "\\uninstall.exe", "不再寫 uninstall.exe", want=False)
    text = c.marker(d)
    c.check("app=DBZ-SSD-zhTW" in text and "version=v45" in text and "installed=" in text, "標記檔內容", text)
    c.eq(marker_state(c, d), "installed", "標記檔 state=installed")
    c.check("uninstaller=" not in text and "save=" not in text, "標記檔沒有舊欄位", text)
    c.eq(c.w.reg_get("InstallLocation"), d, "登錄 InstallLocation")
    c.eq(c.w.reg_get("UninstallString"), f'"{d}\\{UNINST}"', "登錄 UninstallString 指向新檔名")
    c.eq(c.w.own_get("InstallDir"), d, "本程式機碼記下上次的位置")
    c.check((c.w.startmenu_dir / f"{PRODUCT}.lnk").exists(), "開始功能表遊戲捷徑")
    c.check((c.w.startmenu_dir / f"{HDPACK_LNK}.lnk").exists(), "開始功能表美術包捷徑")
    c.check(c.w.desktop_lnk.exists(), "桌面捷徑")
    c.log_has("C:\\dbztest\\logs\\setup.log", "安裝完成")
    loc = c.w.reg_get("InstallLocation") or ""
    c.check(not loc.endswith("\\"), "InstallLocation 沒有結尾的 \\", loc)
    for v in ("InstallLocation", "UninstallString", "QuietUninstallString", "DisplayIcon"):
        val = c.w.reg_get(v) or ""
        c.check("\\\\" not in val, f"登錄 {v} 沒有連續兩個反斜線", val)
    for lnk in (c.w.startmenu_dir / f"{PRODUCT}.lnk", c.w.startmenu_dir / f"{HDPACK_LNK}.lnk", c.w.desktop_lnk):
        data = lnk.read_bytes() if lnk.exists() else b""
        c.check(b"DBZ-SSD-zhTW" in data.replace(b"\x00", b"") and b"\\\x00\\\x00" not in data and b"\\\\" not in data,
                f"捷徑 {lnk.name} 的目標沒有連續兩個反斜線")


@scenario("資料夾裡只有別的程式的 uninstall.exe：不是本程式的資料夾，結束碼 12、對方的檔不動")
def s02_foreign(c: Ctx):
    d = c.T + "\\other"
    c.write(d + "\\uninstall.exe", b"OTHER-UNINSTALLER")
    before = c.snapshot(d)
    c.eq(c.install(d), 12, "結束碼")
    c.unchanged(d, before, "對方的資料夾一個位元組都沒變")
    nothing_installed(c, "別人的資料夾")


@scenario("資料夾已有其他檔案、不是本程式的：靜默時結束碼 12、一個位元組都沒變、也沒建子資料夾")
def s03_nonempty(c: Ctx):
    d = c.T + "\\busy"
    c.write(d + "\\notes.txt", b"mine")
    c.write(d + "\\save\\other\\keep.txt", b"KEEP")
    before = c.snapshot(d)
    c.eq(c.install(d), 12, "結束碼")
    c.unchanged(d, before, "資料夾內容一個位元組都沒變（含沒有建 DBZ-SSD-zhTW 子資料夾）")
    nothing_installed(c, "已有其他檔案")
    e = c.T + "\\only-desktop-ini"
    c.write(e + "\\desktop.ini", b"[.ShellClassInfo]")
    c.write(e + "\\Thumbs.db", b"x")
    c.eq(c.install(e), 0, "只有 desktop.ini、Thumbs.db 的資料夾算空的：結束碼 0")
    c.exists(e + "\\desktop.ini", "desktop.ini 保留")
    f = c.T + "\\empty-existing"
    c.w.u(f).mkdir()
    c.eq(c.install(f), 0, "已存在的空資料夾：結束碼 0")


@scenario("磁碟根目錄、Windows 資料夾、路徑是檔案、寫不進去：結束碼 10，預設位置與登錄都沒被寫入")
def s04_bad_dirs(c: Ctx):
    c.eq(c.install("C:\\"), 10, "根目錄 C:\\ 結束碼")
    c.check(not (c.w.c / MARKER).exists() and not (c.w.c / UNINST).exists(), "根目錄沒有寫入標記或解除安裝程式")
    nothing_installed(c, "根目錄")
    c.eq(c.install("C:\\windows\\dbz"), 10, "Windows 資料夾底下 結束碼")
    c.exists("C:\\windows\\dbz", want=False)
    nothing_installed(c, "Windows 資料夾底下")
    c.eq(c.install("C:\\WINDOWS"), 10, "Windows 資料夾本身（大寫）結束碼")
    nothing_installed(c, "Windows 資料夾本身")
    f = c.write(c.T + "\\afile", b"FILE")
    c.eq(c.install(c.T + "\\afile"), 10, "路徑是檔案 結束碼")
    c.eq(f.read_bytes(), b"FILE", "那個檔案內容不變")
    nothing_installed(c, "路徑是檔案")
    ro = c.w.u(c.T + "\\ro")
    ro.mkdir()
    os.chmod(ro, 0o555)
    try:
        c.eq(c.install(c.T + "\\ro\\inst"), 10, "寫不進去 結束碼")
        c.exists(c.T + "\\ro\\inst", want=False)
    finally:
        os.chmod(ro, 0o755)
    nothing_installed(c, "寫不進去")
    c.log_has("C:\\dbztest\\logs\\setup.log", "不能用這個安裝位置：C:\\")


def check_rom_copied(c: Ctx, d, log):
    p = c.w.u(d + "\\save\\setup\\rom.sfc")
    c.eq(p.read_bytes() if p.exists() else None, b"ROMDATA" * 100, "rom.sfc 內容與來源相同")
    p = c.w.u(d + "\\save\\setup\\dbz102.ips")
    c.eq(p.read_bytes() if p.exists() else None, b"PATCH" * 10, "dbz102.ips 內容與來源相同")
    c.check(c.w.u(c.T + "\\src\\my game\\dbz.sfc").exists() and c.w.u(c.T + "\\src\\dbz102.ips").exists(),
            "來源檔仍在")
    c.log_has(log, "ROM（copied）")


@scenario("/ROM、/IPS 複製到 save\\setup\\、原檔不動；路徑含空白、整個參數加引號（PowerShell 的傳法）")
def s05_rom_copy(c: Ctx):
    c.write(c.T + "\\src\\my game\\dbz.sfc", b"ROMDATA" * 100)
    c.write(c.T + "\\src\\dbz102.ips", b"PATCH" * 10)
    d = c.T + "\\inst"
    log = "C:\\dbztest\\logs\\my log.txt"
    # Wine, like PowerShell, quotes a whole argument that contains a space: "/ROM=C:\...\my game\dbz.sfc"
    rc = c.install(d, "/ROM=C:\\dbztest\\src\\my game\\dbz.sfc", "/IPS=C:\\dbztest\\src\\dbz102.ips", log=log)
    c.eq(rc, 0, "結束碼")
    check_rom_copied(c, d, log)


@scenario("/ROM=\"…\" 值加引號（cmd.exe 的寫法，命令列原樣傳入）")
def s05b_rom_cmd_quoting(c: Ctx):
    c.write(c.T + "\\src\\my game\\dbz.sfc", b"ROMDATA" * 100)
    c.write(c.T + "\\src\\dbz102.ips", b"PATCH" * 10)
    d = c.T + "\\inst"
    log = "C:\\dbztest\\logs\\setup.log"
    rc = c.install_raw('/S /LOG=' + log + ' /ROM="C:\\dbztest\\src\\my game\\dbz.sfc" '
                       '/IPS=C:\\dbztest\\src\\dbz102.ips /D=' + d)
    c.eq(rc, 0, "結束碼")
    check_rom_copied(c, d, log)


@scenario("/ROM 是唯讀 .smc：複本是 rom.smc 且不是唯讀")
def s06_rom_readonly(c: Ctx):
    rom = c.write(c.T + "\\src\\dbz.smc", b"SMC" * 50)
    os.chmod(rom, 0o444)
    d = c.T + "\\inst"
    c.eq(c.install(d, "/ROM=C:\\dbztest\\src\\dbz.smc"), 0, "結束碼")
    p = c.w.u(d + "\\save\\setup\\rom.smc")
    c.check(p.exists(), "rom.smc 存在")
    c.check(p.exists() and os.access(p, os.W_OK), "rom.smc 可寫（不是唯讀）")
    os.chmod(rom, 0o644)


@scenario("/ROM、/IPS 輕量檢查擋下：結束碼 13、不安裝")
def s07_rom_rejects(c: Ctx):
    c.write(c.T + "\\src\\game.bin", b"BIN")
    c.write(c.T + "\\src\\empty.sfc", b"")
    c.write(c.T + "\\src\\ok.sfc", b"OK")
    c.write(c.T + "\\src\\patch.txt", b"TXT")
    cases = [
        ("副檔名不是 .sfc／.smc", ["/ROM=C:\\dbztest\\src\\game.bin"]),
        ("ROM 不存在", ["/ROM=C:\\dbztest\\src\\nope.sfc"]),
        ("ROM 是 0 位元組", ["/ROM=C:\\dbztest\\src\\empty.sfc"]),
        ("只有 /IPS 沒有 /ROM", ["/IPS=C:\\dbztest\\src\\ok.sfc"]),
        ("IPS 副檔名不對", ["/ROM=C:\\dbztest\\src\\ok.sfc", "/IPS=C:\\dbztest\\src\\patch.txt"]),
    ]
    for i, (what, args) in enumerate(cases):
        d = c.T + f"\\r{i}"
        c.eq(c.install(d, *args), 13, f"{what}：結束碼")
        c.exists(d + "\\" + MARKER, f"{what}：沒有安裝", want=False)


@scenario("重裝換 ROM：.sfc 換成 .smc 且沒帶 IPS，舊的 rom.sfc 與 IPS 被移除")
def s08_rom_replace(c: Ctx):
    c.write(c.T + "\\src\\a.sfc", b"A")
    c.write(c.T + "\\src\\a.ips", b"P")
    c.write(c.T + "\\src\\b.smc", b"B")
    d = c.T + "\\inst"
    c.eq(c.install(d, "/ROM=C:\\dbztest\\src\\a.sfc", "/IPS=C:\\dbztest\\src\\a.ips"), 0, "第一次 結束碼")
    c.eq(c.install(d, "/ROM=C:\\dbztest\\src\\b.smc"), 0, "第二次 結束碼")
    c.exists(d + "\\save\\setup\\rom.smc")
    c.exists(d + "\\save\\setup\\rom.sfc", want=False)
    c.exists(d + "\\save\\setup\\dbz102.ips", want=False)


@scenario("/ROM 指向 save\\setup\\ 裡那份本身（換大小寫）：判成原地、檔案不被刪")
def s09_rom_inplace(c: Ctx):
    c.write(c.T + "\\src\\a.sfc", b"ORIGINAL")
    d = c.T + "\\inst"
    c.eq(c.install(d, "/ROM=C:\\dbztest\\src\\a.sfc"), 0, "第一次 結束碼")
    c.eq(c.install(d, "/ROM=C:\\DBZTEST\\INST\\SAVE\\SETUP\\ROM.SFC"), 0, "第二次 結束碼")
    p = c.w.u(d + "\\save\\setup\\rom.sfc")
    c.eq(p.read_bytes() if p.exists() else None, b"ORIGINAL", "rom.sfc 仍在且內容不變")
    c.log_has("C:\\dbztest\\logs\\setup.log", "ROM（inplace）")


@scenario("已設定過 ROM（有 canonical.sfc）：/ROM 略過並寫紀錄")
def s10_rom_already(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "第一次 結束碼")
    c.write(d + "\\save\\cache\\rom\\canonical.sfc", b"CANON")
    c.write(c.T + "\\src\\a.sfc", b"A")
    c.eq(c.install(d, "/ROM=C:\\dbztest\\src\\a.sfc"), 0, "第二次 結束碼")
    c.exists(d + "\\save\\setup", want=False)
    c.log_has("C:\\dbztest\\logs\\setup.log", "已經設定過 ROM，略過")


@scenario("重裝到本程式先前的位置：save\\ 與美術包保留，標記檔寫回 state=installed；連跑兩次結果相同")
def s11_reinstall_owned(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "第一次 結束碼")
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    c.write(d + "\\hd-pack.zip", b"ZIP")
    c.eq(c.install(d), 0, "第二次 結束碼")
    c.eq(c.install(d), 0, "第三次 結束碼")
    saves_in_place(c, d)
    c.exists(d + "\\hd-pack.zip")
    c.exists(d + "\\" + APP_ID, "沒有建子資料夾", want=False)
    c.eq(marker_state(c, d), "installed", "標記檔 state=installed")
    c.eq(c.w.reg_get("InstallLocation"), d, "登錄 InstallLocation")


@scenario("免安裝 7z 原地升級：本專案啟動器（版本資源相符）→ 裝進去、save\\ 不動；只有同名檔不算")
def s12_portable_dir(c: Ctx):
    d = c.T + "\\portable"
    shutil.copyfile(c.build.portable_launcher(), c.write(d + "\\" + EXE_NAME))
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    c.write(d + "\\save\\other\\keep.txt", b"KEEP")
    c.write(d + "\\dbz.sfc", b"ROM")
    save_before = c.snapshot(d + "\\save")
    c.eq(c.install(d), 0, "原地升級 結束碼")
    c.eq(marker_state(c, d), "installed", "標記檔 state=installed")
    c.unchanged(d + "\\save", save_before, "save\\ 一個位元組都沒變")
    c.exists(d + "\\dbz.sfc", "玩家放在旁邊的檔保留")
    c.log_has("C:\\dbztest\\logs\\setup.log", "從免安裝版原地升級")
    decoy = c.T + "\\decoy"
    c.write(decoy + "\\" + EXE_NAME, b"NOT THE LAUNCHER")
    c.write(decoy + "\\save\\other\\keep.txt", b"KEEP")
    before = c.snapshot(decoy)
    c.eq(c.install(decoy), 12, "只有同名 exe（沒有版本資源）：結束碼 12")
    c.unchanged(decoy, before, "同名 exe 的資料夾一個位元組都沒變")


@scenario("標記檔被刪掉的安裝：歸屬只看標記檔，不再認得 → 結束碼 12、不寫入")
def s13_marker_deleted(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "第一次 結束碼")
    c.w.u(d + "\\" + MARKER).unlink()
    before = c.snapshot(d)
    c.eq(c.install(d), 12, "標記檔不見後重裝 結束碼")
    c.unchanged(d, before, "資料夾一個位元組都沒變")


@scenario("subst 磁碟（X: 對應到 C:\\dbztest）：用 X:\\… 重裝同一個資料夾，認得標記檔")
def s14_subst_same(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "第一次 結束碼")
    os.symlink("../drive_c/dbztest", c.w.prefix / "dosdevices" / "x:")
    c.eq(c.install("X:\\inst"), 0, "用 X:\\inst 重裝 結束碼")
    c.exists(d + "\\" + APP_ID, "沒有建子資料夾", want=False)
    c.eq(c.w.reg_get("InstallLocation"), "X:\\inst", "登錄改記 X:\\inst")
    c.exists(c.w.default_dir, "預設位置沒有被裝進去", want=False)


@scenario("試寫暫存檔：分類時忽略它；它刪不掉（被開著）就擋下、結束碼 10")
def s20_write_test_file(c: Ctx):
    d = c.T + "\\had-tmp"
    c.write(d + "\\dbz-zhtw-write-test.tmp", b"")
    c.eq(c.install(d), 0, "只有上次留下的試寫暫存檔：算空的，結束碼 0")
    c.exists(d + "\\dbz-zhtw-write-test.tmp", "暫存檔已刪", want=False)
    e = c.T + "\\locked-tmp"
    c.write(e + "\\dbz-zhtw-write-test.tmp", b"")
    h = c.hold(e + "\\dbz-zhtw-write-test.tmp", 12000, share=3)   # writable, never deletable
    try:
        rc = c.install(e)
    finally:
        h.wait(timeout=60)
    c.eq(rc, 10, "刪不掉的暫存檔 結束碼")
    c.log_has("C:\\dbztest\\logs\\setup.log", "試寫的暫存檔刪不掉")
    c.exists(e + "\\" + MARKER, "沒有安裝", want=False)


@scenario("資料夾列不出內容（沒有讀取權限）：不當成空的，結束碼 12、不寫入")
def s21_unlistable_folder(c: Ctx):
    d = c.T + "\\no-list"
    c.write(d + "\\inside.txt", b"x")
    p = c.w.u(d)
    os.chmod(p, 0o000)
    try:
        rc = c.install(d)
    finally:
        os.chmod(p, 0o755)
    c.eq(rc, 12, "結束碼")
    c.eq(sorted(x.name for x in p.iterdir()), ["inside.txt"], "資料夾內容不變")
    nothing_installed(c, "列不出內容")
    q = c.T + "\\no-list-portable"
    shutil.copyfile(c.build.portable_launcher(), c.write(q + "\\" + EXE_NAME))
    qp = c.w.u(q)
    os.chmod(qp, 0o333)   # can enter, cannot list
    try:
        rc = c.install(q, log="C:\\dbztest\\logs\\setup2.log")
    finally:
        os.chmod(qp, 0o755)
    c.eq(rc, 12, "列不出內容、裡面有免安裝版 exe：仍不算免安裝版，結束碼 12")
    c.exists(q + "\\" + MARKER, "沒有原地升級", want=False)


@scenario("寫不進啟動器 exe（被開著）：結束碼 14，不接著寫「新增或移除程式」的項目，靜默時不卡住")
def s22_exe_write_fails(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "先安裝")
    c.w.reg_set("DisplayName", "SENTINEL")
    h = c.hold(d + "\\" + EXE_NAME, 15000, share=1)
    try:
        rc = c.install(d)
    finally:
        h.wait(timeout=60)
    c.eq(rc, 14, "結束碼")
    c.eq(c.w.reg_get("DisplayName"), "SENTINEL", "項目沒有被改寫")
    c.log_has("C:\\dbztest\\logs\\setup.log", "寫不進")
    c.eq(c.install(d), 0, "放開後再裝 結束碼")
    # `wine reg query` prints Chinese through the console code page (as "???"): check it was rewritten, not its text
    c.check(c.w.reg_get("DisplayName") not in (None, "SENTINEL"), "這次才寫項目", str(c.w.reg_get("DisplayName")))


@scenario("標記檔是隱藏的；改寫照常成功；使用者刪掉它之後重裝會被當成別人的資料夾")
def s23_marker_hidden(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.check("H" in c.w.attrib(d + "\\" + MARKER), "安裝後標記檔是隱藏的", c.w.attrib(d + "\\" + MARKER))
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    c.eq(c.uninstall(d), 0, "解除（保留存檔）")
    c.eq(marker_state(c, d), "removed", "隱藏的標記檔照常改寫成 removed")
    c.check("H" in c.w.attrib(d + "\\" + MARKER), "改寫後仍是隱藏的")
    c.eq(c.install(d), 0, "重裝 結束碼")
    c.eq(marker_state(c, d), "installed", "隱藏的標記檔照常改寫成 installed")


@scenario("/D= 帶了空值（腳本變數沒設）：結束碼 10，預設位置與登錄都沒被寫入")
def s16_empty_d(c: Ctx):
    c.eq(c.install_raw("/S /LOG=C:\\dbztest\\logs\\setup.log /D="), 10, "結束碼")
    nothing_installed(c, "/D= 空值")
    c.log_has("C:\\dbztest\\logs\\setup.log", "帶了 /D= 但沒有給資料夾")
    c.eq(c.install_raw("/S /LOG=C:\\dbztest\\logs\\setup2.log /D=   "), 10, "/D= 只有空白 結束碼")
    nothing_installed(c, "/D= 只有空白")


@scenario("標記檔內容不對 → 不是本程式的；state=removed 的資料夾裡有別人的 uninstall.exe → 照裝、對方的檔不變")
def s17_marker_content_and_foreign_uninstaller(c: Ctx):
    a = c.T + "\\fake-marker"
    c.write(a + "\\" + MARKER, b"app=something-else\r\nstate=installed\r\n")
    c.write(a + "\\notes.txt", b"mine")
    before = c.snapshot(a)
    c.eq(c.install(a), 12, "標記檔沒有 app=DBZ-SSD-zhTW：結束碼 12")
    c.unchanged(a, before, "那個資料夾一個位元組都沒變")

    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "先正常安裝")
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    c.eq(c.uninstall(d), 0, "解除（保留資料）")
    c.eq(marker_state(c, d), "removed", "標記檔 state=removed")
    c.write(d + "\\uninstall.exe", b"OTHER-UNINSTALLER")
    c.eq(c.install(d), 0, "state=removed＋別人的 uninstall.exe：照裝")
    c.eq(c.w.u(d + "\\uninstall.exe").read_bytes(), b"OTHER-UNINSTALLER", "別人的 uninstall.exe 沒被動")
    c.exists(d + "\\" + UNINST, "本程式的解除安裝程式在")
    saves_in_place(c, d)


@scenario("目標在另一個安裝裡面：上層是 installed → 整棵擋下；上層是 removed → 只擋它的 save\\ 底下，其餘放行")
def s18_inside_other_install(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "先安裝")
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    before = c.snapshot(d)
    c.eq(c.install(d + "\\save"), 12, "裝到 installed 那份的 save\\ 結束碼")
    c.eq(c.install(d + "\\tools"), 12, "裝到 installed 那份的一般子資料夾 結束碼")
    c.unchanged(d, before, "那個安裝一個位元組都沒變")
    c.log_has("C:\\dbztest\\logs\\setup.log", "狀態 installed")
    c.eq(c.uninstall(d), 0, "解除（保留存檔）")
    c.eq(marker_state(c, d), "removed", "上層的標記檔 state=removed")
    before = c.snapshot(d + "\\save")
    c.eq(c.install(d + "\\save\\new", log="C:\\dbztest\\logs\\setup2.log"), 12, "裝到 removed 那份的 save\\ 底下 結束碼")
    c.unchanged(d + "\\save", before, "保留的 save\\ 一個位元組都沒變")
    c.log_has("C:\\dbztest\\logs\\setup2.log", "狀態 removed")
    c.eq(c.install(d + "\\tools"), 0, "裝到 removed 那份的一般子資料夾 結束碼")
    c.eq(marker_state(c, d + "\\tools"), "installed", "子資料夾是新的安裝")


@scenario("之前測試版的安裝（舊欄位標記、uninstall.exe）→ 新版重裝 → 解除並勾刪除")
def s19_legacy_test_build(c: Ctx):
    d = c.T + "\\old-build"
    c.write(d + "\\" + MARKER, b"app=DBZ-SSD-zhTW\r\nversion=v44\r\ninstalled=2026-11-14 10:00:00\r\nuninstaller=0:0:12345\r\nsave=preexisting\r\n")
    c.write(d + "\\" + EXE_NAME, b"OLD LAUNCHER")
    shutil.copyfile(c.build.old_uninstaller(), c.write(d + "\\uninstall.exe"))  # carries this program's ProductName
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    c.w.reg_set("InstallLocation", d)
    c.w.reg_set("UninstallString", f'"{d}\\uninstall.exe"')
    c.w.reg_set("UninstallerId", "0:0:12345")
    c.w.reg_set("SaveOrigin", "preexisting")
    c.eq(c.install(d), 0, "新版重裝 結束碼")
    c.exists(d + "\\uninstall.exe", "舊的 uninstall.exe（本程式的）已刪", want=False)
    c.exists(d + "\\" + UNINST, "新的解除安裝程式")
    text = c.marker(d)
    c.check("uninstaller=" not in text and "save=" not in text and "state=installed" in text, "標記檔改成新格式", text)
    c.eq(c.w.reg_get("UninstallString"), f'"{d}\\{UNINST}"', "登錄改指新的解除安裝程式")
    c.eq(c.w.reg_get("UninstallerId"), None, "舊值 UninstallerId 已刪")
    c.eq(c.w.reg_get("SaveOrigin"), None, "舊值 SaveOrigin 已刪")
    saves_in_place(c, d)
    c.eq(c.uninstall(d, "/DELETEDATA"), 0, "解除並勾刪除 結束碼")
    c.exists(d, "整個資料夾移除", want=False)
    c.check(not c.w.reg_key_exists(), "登錄項目已刪")
    # a folder of ours with someone else's uninstall.exe (no version resource): kept
    e = c.T + "\\ours-with-foreign"
    c.eq(c.install(e), 0, "另一份安裝")
    c.write(e + "\\uninstall.exe", b"OTHER")
    c.eq(c.install(e), 0, "有別人的 uninstall.exe 時重裝")
    c.eq(c.w.u(e + "\\uninstall.exe").read_bytes(), b"OTHER", "別人的 uninstall.exe 沒被動")


@scenario("之前測試版的安裝、啟動器 exe 被開著時重裝：結束碼 14，舊的 uninstall.exe 與舊項目都還在；放開後再裝才刪")
def s24_legacy_kept_when_write_fails(c: Ctx):
    d = c.T + "\\old-build"
    c.write(d + "\\" + MARKER, b"app=DBZ-SSD-zhTW\r\nversion=v44\r\ninstalled=2026-11-14 10:00:00\r\n")
    c.write(d + "\\" + EXE_NAME, b"OLD LAUNCHER")
    shutil.copyfile(c.build.old_uninstaller(), c.write(d + "\\uninstall.exe"))
    c.w.reg_set("InstallLocation", d)
    c.w.reg_set("UninstallString", f'"{d}\\uninstall.exe"')
    h = c.hold(d + "\\" + EXE_NAME, 15000, share=1)
    try:
        rc = c.install(d)
    finally:
        h.wait(timeout=60)
    c.eq(rc, 14, "寫不進啟動器 結束碼")
    c.exists(d + "\\uninstall.exe", "舊的 uninstall.exe 還在")
    c.eq(c.w.reg_get("UninstallString"), f'"{d}\\uninstall.exe"', "舊項目仍指向它")
    c.eq(c.install(d, log="C:\\dbztest\\logs\\setup2.log"), 0, "放開後再裝 結束碼")
    c.exists(d + "\\uninstall.exe", "這次才刪舊的 uninstall.exe", want=False)


@scenario("安裝位置（或上層）的標記檔存在卻打不開：當成那份安裝還在，靜默不裝（結束碼 14、紀錄寫原因），什麼都沒動；放開後照常重裝")
def s25_marker_unreadable(c: Ctx):
    # Limit: "cannot be opened" is guidrive hold (the file opened without sharing for 20 s, started 2 s before the next
    # step): the steps inside must finish within that time, and a real game or antivirus holding it is a hardware item
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "先安裝")
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    before = c.snapshot(d)
    h = c.hold(d + "\\" + MARKER, 20000, share=0)   # exists, cannot be opened (not even for reading)
    try:
        rc = c.install(d)
        rc_sub = c.install(d + "\\sub", log="C:\\dbztest\\logs\\setup-sub.log")
        rc_un = c.uninstall(d, "/DELETEDATA", log="C:\\dbztest\\logs\\un-locked.log")
    finally:
        h.wait(timeout=60)
    c.eq(rc, 14, "標記檔打不開 結束碼")
    c.log_has("C:\\dbztest\\logs\\setup.log", "安裝標記檔打不開")
    c.unchanged(d, before, "安裝資料夾沒有任何變動（沒有改裝到子資料夾）")
    c.eq(rc_sub, 14, "上層的標記檔打不開 結束碼")
    c.log_has("C:\\dbztest\\logs\\setup-sub.log", "安裝標記檔打不開")
    c.exists(d + "\\sub", "上層打不開時沒有建子資料夾", want=False)
    # the uninstaller on its own location: still 21 and nothing deleted, but it says the marker cannot be opened
    c.eq(rc_un, 21, "解除安裝自己位置的標記檔打不開 結束碼")
    c.log_has("C:\\dbztest\\logs\\un-locked.log", "無法確認是不是本程式目前的安裝")
    c.log_has("C:\\dbztest\\logs\\un-locked.log", "打不開")
    c.unchanged(d, before, "解除安裝也沒有動任何東西")
    c.eq(c.install(d, log="C:\\dbztest\\logs\\setup2.log"), 0, "放開後重裝 結束碼")
    c.exists(d + "\\save\\saves\\ssd.sav", "存檔還在")


# ----------------------------------------------------------------------------------------------------------------------
# Upgrade scenarios: a released RC1, RC2, RC3 or v1.0 installer first, then the installer under test as the next release
# (Build.next_setup). What they prove: the installer's own logic (where it installs, what it writes and leaves, the
# registry, the shortcuts). What they do not: the real launcher of the next release (a stand-in is installed), what that
# launcher does on its first run (the unpacked game of the old bundle in %LOCALAPPDATA%\dbz-ssd-zhtw\app is removed by
# the launcher, launcher/src/main.rs clean_up, not by the installer), Windows itself (Explorer's shortcuts, Settings >
# Apps, an exe locked because it runs).
# ----------------------------------------------------------------------------------------------------------------------

UPGRADE_LOG = "C:\\dbztest\\logs\\upgrade.log"


def plant_player_files(c: Ctx, d):
    """What an earlier release's player has in the install folder after playing (installer/README.md's list of the
    launcher's and the game's files), and the launcher's own folder."""
    for rel, data in (("save\\saves\\ssd.sav", b"BATTERY"),
                      ("save\\saves\\ssd.settings", b"ssd-settings\r\naccess.language=zh-tw\r\n"),
                      ("save\\saves\\ssd.options", b"OPTIONS"),
                      ("save\\states\\slot1.ssdstate", b"STATE"),
                      ("save\\cache\\rom\\canonical.sfc", b"ROM"),
                      ("save\\art-override\\a.png", b"PNG"),
                      ("save\\voice\\ssj-transform.wav", b"WAV"),
                      ("save\\" + SYNC_MARKER, b"sync"),
                      ("save\\.launcher-lang", b"zh-tw"),
                      ("save\\.launcher-art", b"art"),
                      ("save\\zh-tw.tsv", b"TSV"),
                      ("save\\fonts\\OFL.txt", b"OFL"),
                      ("hd-pack.zip", b"PK-HD")):
        c.write(d + "\\" + rel, data)
    la = c.w.localappdata
    c.write(la + "\\dbz-ssd-zhtw\\app\\old-bundle\\ssd-recompiled.exe", b"OLD GAME")
    c.write(la + "\\dbz-ssd-zhtw\\last-run.log", b"log")


def old_install(c: Ctx, name, d=None):
    """Install the released `name` silently (where it picks without `d`); return the folder it went to."""
    log = f"C:\\dbztest\\logs\\setup-{name}.log"
    c.eq(c.install(d, log=log, exe=c.build.old_setup(name)), 0, f"{name} 安裝程式 結束碼")
    d = d or c.w.default_dir
    c.eq(marker_state(c, d), "installed", f"{name} 裝好：標記檔 state=installed")
    c.check(f"version=v{OLD_SETUPS[name][2]}" in c.marker(d), f"{name} 裝好：標記檔 version=v{OLD_SETUPS[name][2]}",
            c.marker(d))
    c.eq(c.w.reg_get("DisplayVersion"), name, f"{name} 裝好：登錄 DisplayVersion")
    # the old installer put exactly what the new one puts: nothing it wrote can be left behind as a file of no use
    c.eq(sorted(os.listdir(c.w.u(d))), sorted(PROGRAM_FILES), f"{name} 安裝程式放的檔與新版相同（只有這三個）")
    return d


def upgraded(c: Ctx, d, old_name, uninst_before, what=""):
    """After the next release went over `old_name` in `d`: one install, the new one's files, nothing extra."""
    # left over = at the top of the folder, neither the installer's own files nor the player's (save\, hd-pack.zip)
    extra = sorted(set(os.listdir(c.w.u(d))) - PROGRAM_FILES - {"save", "hd-pack.zip"})
    c.eq(extra, [], f"{what}安裝資料夾沒有多餘的檔（不是程式三個檔、save\\、hd-pack.zip 的東西）")
    exe = c.w.u(d + "\\" + EXE_NAME)
    got = exe.read_bytes() if exe.exists() else b""
    c.check(got == c.build.standin().read_bytes(), f"{what}啟動器 exe 換成新版的",
            f"現在的 exe {len(got)} 位元組，新版的替身 {c.build.standin().stat().st_size} 位元組")
    c.check(c.w.u(d + "\\" + UNINST).read_bytes() != uninst_before, f"{what}解除安裝程式換成新版的")
    text = c.marker(d)
    c.check(f"version=v{NEXT_RELEASE}" in text and "app=" + APP_ID in text, f"{what}標記檔 version=v{NEXT_RELEASE}", text)
    c.eq(marker_state(c, d), "installed", f"{what}標記檔 state=installed")
    c.eq(c.w.subkeys(UNINST_ROOT), [APP_ID], f"{what}「新增或移除程式」只有一個項目")
    c.eq(c.w.reg_get("DisplayVersion"), NEXT_NAME, f"{what}登錄 DisplayVersion 是新版")
    c.eq(c.w.reg_get("InstallLocation"), d, f"{what}登錄 InstallLocation")
    c.eq(c.w.reg_get("UninstallString"), f'"{d}\\{UNINST}"', f"{what}登錄 UninstallString")
    c.eq(c.w.own_get("InstallDir"), d, f"{what}記下的安裝位置")
    c.eq([k for k in c.w.subkeys("HKCU\\Software") if k.startswith(APP_ID)], [APP_ID], f"{what}本程式自己的機碼只有一個")
    sm = sorted(p.name for p in c.w.startmenu_dir.iterdir()) if c.w.startmenu_dir.is_dir() else []
    c.eq(sm, sorted([f"{PRODUCT}.lnk", f"{HDPACK_LNK}.lnk"]), f"{what}開始功能表資料夾只有兩個捷徑")
    desk = sorted(p.name for p in (c.w.user / "Desktop").iterdir() if p.suffix.lower() == ".lnk")
    c.eq(desk, [f"{PRODUCT}.lnk"], f"{what}桌面只有一個捷徑")
    c.check(not (c.w.startmenu_dir.parent / f"{PRODUCT}.lnk").exists(), f"{what}開始功能表最上層沒有舊捷徑")
    c.log_has(UPGRADE_LOG, f"偵測到已安裝的 {old_name}，原地升級為 {NEXT_NAME}：{d}", f"{what}紀錄寫明從 {old_name} 原地升級")
    entry_ok(c, what)


def upgrade_over(c: Ctx, name):
    """`name` installed where it picks (the default folder), played, then the next release installed where it picks."""
    d = old_install(c, name)
    plant_player_files(c, d)
    save_before = c.snapshot(d + "\\save")
    app_before = c.snapshot(c.w.localappdata + "\\dbz-ssd-zhtw")
    uninst_before = c.w.u(d + "\\" + UNINST).read_bytes()
    c.eq(c.install(log=UPGRADE_LOG, exe=c.build.next_setup(c.nsi)), 0, "新版安裝程式（不帶 /D=）結束碼")
    c.exists(d + "\\" + APP_ID, "沒有建子資料夾", want=False)
    upgraded(c, d, name, uninst_before)
    c.unchanged(d + "\\save", save_before, "save\\ 一個位元組都沒變（存檔、設定、語言、ROM 快取、美術覆蓋圖、語音、譯文表）")
    c.eq(c.w.u(d + "\\hd-pack.zip").read_bytes(), b"PK-HD", "hd-pack.zip 保留")
    c.unchanged(c.w.localappdata + "\\dbz-ssd-zhtw", app_before, "%LOCALAPPDATA%\\dbz-ssd-zhtw 安裝程式沒有動")
    # the upgraded install is an ordinary one: uninstalling it (keeping data) works and keeps the save
    c.eq(c.uninstall(d), 0, "升級後解除安裝（保留資料）結束碼")
    saves_in_place(c, d, "升級後解除安裝：")
    c.check(not c.w.reg_key_exists(), "升級後解除安裝：「新增或移除程式」的項目已刪")


@scenario("已裝 RC1（真的 RC1 安裝程式、預設位置、玩過）→ 新版不帶 /D=：原地升級，一個項目、捷徑不重複、沒有多餘的檔、save\\ 與 hd-pack.zip 不變")
def v01_upgrade_rc1(c: Ctx):
    upgrade_over(c, "RC1")


@scenario("已裝 RC2（真的 RC2 安裝程式、預設位置、玩過）→ 新版不帶 /D=：原地升級，一個項目、捷徑不重複、沒有多餘的檔、save\\ 與 hd-pack.zip 不變")
def v02_upgrade_rc2(c: Ctx):
    upgrade_over(c, "RC2")


@scenario("已裝 RC3（真的 RC3 安裝程式、預設位置、玩過）→ 新版不帶 /D=：原地升級，一個項目、捷徑不重複、沒有多餘的檔、save\\ 與 hd-pack.zip 不變")
def upgrade_from_rc3(c: Ctx):
    upgrade_over(c, "RC3")


@scenario("已裝 v1.0（真的 v1.0 安裝程式、預設位置、玩過）→ 新版不帶 /D=：原地升級，一個項目、捷徑不重複、沒有多餘的檔、save\\ 與 hd-pack.zip 不變")
def upgrade_from_v1_0(c: Ctx):
    upgrade_over(c, "v1.0")


@scenario("RC1 裝在 A、RC2 裝在 B（各自玩過）→ 新版不帶 /D=：升級 B（登錄與記下的位置指的那份）；A 原封不動，RC2 時就已不在「新增或移除程式」裡")
def v03_upgrade_two_folders(c: Ctx):
    a, b = c.T + "\\rc1", c.T + "\\rc2"
    old_install(c, "RC1", a)
    plant_player_files(c, a)
    old_install(c, "RC2", b)
    plant_player_files(c, b)
    # before the new release: RC2's install already took the one entry and the remembered folder from A
    c.eq(c.w.subkeys(UNINST_ROOT), [APP_ID], "RC2 裝好後：「新增或移除程式」只有一個項目")
    c.eq(c.w.reg_get("InstallLocation"), b, "RC2 裝好後：項目指向 B（A 已經不在項目裡）")
    c.eq(c.w.own_get("InstallDir"), b, "RC2 裝好後：記下的位置是 B")
    a_before = c.snapshot(a)
    b_save = c.snapshot(b + "\\save")
    uninst_before = c.w.u(b + "\\" + UNINST).read_bytes()
    c.eq(c.install(log=UPGRADE_LOG, exe=c.build.next_setup(c.nsi)), 0, "新版安裝程式（不帶 /D=）結束碼")
    upgraded(c, b, "RC2", uninst_before, "B：")
    c.unchanged(b + "\\save", b_save, "B：save\\ 一個位元組都沒變")
    c.unchanged(a, a_before, "A：整個資料夾一個位元組都沒變（RC1 的程式、標記檔 state=installed、存檔都還在）")
    c.check(a not in c.read_log(UPGRADE_LOG), "新版的紀錄沒有提到 A")
    c.exists(c.w.default_dir, "預設位置沒有被裝進去", want=False)


@scenario("全新安裝（沒有舊版）：程式三個檔、一個項目、紀錄沒有「偵測到已安裝」；同一版再裝一次：紀錄寫「在原位置重新安裝」")
def v04_fresh_then_same(c: Ctx):
    nxt = c.build.next_setup(c.nsi)
    d = c.w.default_dir
    c.eq(c.install(log=UPGRADE_LOG, exe=nxt), 0, "全新安裝 結束碼")
    c.eq(sorted(os.listdir(c.w.u(d))), sorted(PROGRAM_FILES), "安裝資料夾只有程式三個檔")
    c.eq(c.w.subkeys(UNINST_ROOT), [APP_ID], "「新增或移除程式」只有一個項目")
    c.eq(c.w.reg_get("DisplayVersion"), NEXT_NAME, "登錄 DisplayVersion")
    c.log_has(UPGRADE_LOG, "偵測到已安裝", "全新安裝：紀錄沒有「偵測到已安裝」", want=False)
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    log2 = "C:\\dbztest\\logs\\upgrade2.log"
    c.eq(c.install(log=log2, exe=nxt), 0, "同一版再裝一次 結束碼")
    c.log_has(log2, f"偵測到已安裝的 {NEXT_NAME}，在原位置重新安裝：{d}", "同一版再裝：紀錄寫「在原位置重新安裝」")
    c.log_has(log2, "原地升級為", "同一版再裝：紀錄沒有說是升級", want=False)
    saves_in_place(c, d, "同一版再裝：")
    c.eq(c.w.subkeys(UNINST_ROOT), [APP_ID], "同一版再裝：「新增或移除程式」仍只有一個項目")


@scenario("FileId 讀值：檔案編號＝macOS inode、同磁碟序號相同、不存在回空字串；SamePath 各情況")
def s15_fileid_probe(c: Ctx):
    a = c.write(c.T + "\\probe\\a.txt", b"a")
    b = c.write(c.T + "\\probe\\b.txt", b"b")
    os.symlink("../drive_c/dbztest", c.w.prefix / "dosdevices" / "x:")
    out = c.T + "\\logs\\probe.txt"
    lines = [
        "fileid\tC:\\dbztest\\probe\\a.txt",
        "fileid\tC:\\dbztest\\probe\\b.txt",
        "fileid\tC:\\dbztest\\probe",
        "fileid\tC:\\dbztest\\probe\\missing.txt",
        "fileid\tX:\\probe\\a.txt",
        "same\tC:\\dbztest\\probe\\a.txt\tX:\\probe\\a.txt",
        "same\tC:\\dbztest\\probe\\a.txt\tC:\\dbztest\\probe\\b.txt",
        "same\tC:\\dbztest\\probe\\a.txt\tc:\\DBZTEST\\probe\\..\\probe\\A.TXT",
        "same\tC:\\dbztest\\probe\\new\tC:\\DBZTEST\\PROBE\\NEW\\",
        "same\tC:\\dbztest\\probe\\new\tC:\\dbztest\\probe\\other",
        "same\t\tC:\\dbztest\\probe\\a.txt",
    ]
    exe = c.build.probe(c.nsi, lines, out)
    c.w.run(exe, [])
    res = {}
    for ln in c.read_log(out).splitlines():
        parts = ln.split("\t")
        res[tuple(parts[:-1])] = parts[-1]

    def signed(v):
        return v - (1 << 32) if v >= 1 << 31 else v

    def expect_id(p: Path):
        ino = p.stat().st_ino
        return signed(ino >> 32), signed(ino & 0xFFFFFFFF)

    ida = res.get(("fileid", "C:\\dbztest\\probe\\a.txt"), "")
    idb = res.get(("fileid", "C:\\dbztest\\probe\\b.txt"), "")
    idd = res.get(("fileid", "C:\\dbztest\\probe"), "")
    for got, p, what in ((ida, a, "a.txt"), (idb, b, "b.txt"), (idd, a.parent, "資料夾 probe")):
        parts = got.split(":")
        ok = len(parts) == 3 and (int(parts[1]), int(parts[2])) == expect_id(p)
        c.check(ok, f"FileId {what} 的檔案編號＝inode", f"得到 {got!r}，inode {p.stat().st_ino} → {expect_id(p)}")
    # Wine's volume serial is not meaningful (0 or a per-drive value); its real value is a Windows check
    c.check(ida.split(":")[0] == idb.split(":")[0] == idd.split(":")[0] != "",
            "同一磁碟上磁碟序號相同", f"{ida} / {idb} / {idd}")
    c.check(ida != idb, "不同檔案的 FileId 不同", f"{ida} / {idb}")
    c.eq(res.get(("fileid", "C:\\dbztest\\probe\\missing.txt")), "", "不存在的檔回空字串")
    c.eq(res.get(("fileid", "X:\\probe\\a.txt")), ida, "經 X: 開同一個檔，FileId 相同")
    c.eq(res.get(("same", "C:\\dbztest\\probe\\a.txt", "X:\\probe\\a.txt")), "1", "SamePath C: 與 X: 同一檔")
    c.eq(res.get(("same", "C:\\dbztest\\probe\\a.txt", "C:\\dbztest\\probe\\b.txt")), "0", "SamePath 不同檔")
    c.eq(res.get(("same", "C:\\dbztest\\probe\\a.txt", "c:\\DBZTEST\\probe\\..\\probe\\A.TXT")), "1",
         "SamePath 大小寫與 .. 不同")
    c.eq(res.get(("same", "C:\\dbztest\\probe\\new", "C:\\DBZTEST\\PROBE\\NEW\\")), "1",
         "SamePath 都不存在時比正規化路徑（大小寫、結尾反斜線）")
    c.eq(res.get(("same", "C:\\dbztest\\probe\\new", "C:\\dbztest\\probe\\other")), "0", "SamePath 不存在且不同")
    c.eq(res.get(("same", "", "C:\\dbztest\\probe\\a.txt")), "0", "SamePath 空字串")


# ----------------------------------------------------------------------------------------------------------------------
# Uninstall scenarios: each order of the state table, with ①–④ (others unchanged, nothing installed into or over
# someone else's, not stuck, the save still where the launcher reads it)
# ----------------------------------------------------------------------------------------------------------------------

@scenario("預設位置 → 解除（保留資料）→ 重裝同位置：不建子資料夾、save\\ 不變")
def t01_default_keep_reinstall(c: Ctx):
    d = c.w.default_dir
    c.eq(c.install(), 0, "安裝 結束碼")
    c.populate_player_data(d)
    others = plant_others(c, d)
    save_before = c.snapshot(d + "\\save")
    c.eq(c.uninstall(d), 0, "解除（保留）結束碼")
    c.exists(d + "\\" + EXE_NAME, "啟動器已刪", want=False)
    c.exists(d + "\\" + UNINST, "解除安裝程式已刪", want=False)
    c.eq(marker_state(c, d), "removed", "標記檔留著 state=removed")
    c.check(not c.w.reg_key_exists(), "登錄項目已刪")
    c.eq(c.w.own_get("InstallDir"), d, "上次的位置保留")
    c.check(not c.w.desktop_lnk.exists(), "捷徑已刪")
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw\\game\\data.bin", "啟動器資料夾保留")
    c.eq(c.install(), 0, "重裝（不帶 /D=）結束碼")
    c.exists(d + "\\" + APP_ID, "沒有建子資料夾", want=False)
    c.unchanged(d + "\\save", save_before, "save\\ 一個位元組都沒變")
    others_unchanged(c, d, others)
    saves_in_place(c, d)
    entry_ok(c)


@scenario("自訂位置 → 解除（保留）→ 靜默重裝不帶 /D=：裝回自訂位置")
def t02_custom_keep_reinstall(c: Ctx):
    d = c.T + "\\custom"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    others = plant_others(c, d)
    c.eq(c.uninstall(d), 0, "解除（保留）結束碼")
    c.eq(c.install(), 0, "重裝（不帶 /D=）結束碼")
    others_unchanged(c, d, others)
    c.eq(marker_state(c, d), "installed", "裝回自訂位置")
    c.exists(c.w.default_dir, "預設位置沒有被裝進去", want=False)
    c.eq(c.w.reg_get("InstallLocation"), d, "登錄指回自訂位置")
    saves_in_place(c, d)
    entry_ok(c)


@scenario("裝 → 用 _?= 直接執行解除安裝程式（保留資料）→ 重裝：原地；再跑留下的解除安裝程式 → 21")
def t03_self_uninstall_with_data(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    others = plant_others(c, d)
    keep = c.build.work / "self-un.exe"
    shutil.copyfile(c.w.u(d + "\\" + UNINST), keep)
    c.eq(c.uninstall_self(d), 0, "_?= 解除 結束碼")
    c.exists(d + "\\" + UNINST, "正在執行的解除安裝程式刪不掉自己、留著")
    c.eq(marker_state(c, d), "removed", "資料夾還有存檔：標記檔 state=removed")
    c.eq(c.uninstall_self(d, log="C:\\dbztest\\logs\\un2.log"), 21, "再跑留下的解除安裝程式 結束碼")
    c.eq(c.install(d), 0, "重裝 結束碼")
    c.exists(d + "\\" + APP_ID, "沒有建子資料夾", want=False)
    saves_in_place(c, d)
    others_unchanged(c, d, others)
    entry_ok(c)


@scenario("裝（沒有玩家資料）→ 用 _?= 解除：留下解除安裝程式與 state=removed 的標記檔；再跑它 → 21；重裝：原地")
def t03b_self_uninstall_empty(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.eq(c.uninstall_self(d), 0, "_?= 解除 結束碼")
    c.exists(d + "\\" + UNINST, "正在執行的解除安裝程式刪不掉自己、留著")
    c.eq(marker_state(c, d), "removed", "留下的解除安裝程式也是本程式的東西：標記檔保留 state=removed")
    c.eq(sorted(p.name for p in c.w.u(d).iterdir()), sorted([MARKER, UNINST]), "資料夾裡只剩標記檔與解除安裝程式")
    c.check(not c.w.reg_key_exists(), "登錄項目已刪")
    before = c.snapshot(d)
    c.eq(c.uninstall_self(d, log="C:\\dbztest\\logs\\un2.log"), 21, "再跑留下的解除安裝程式 結束碼")
    c.unchanged(d, before, "再跑一次什麼都沒動")
    c.eq(c.install(d), 0, "重裝同位置 結束碼")
    c.exists(d + "\\" + APP_ID, "原地、沒有建子資料夾", want=False)
    c.eq(marker_state(c, d), "installed", "標記檔寫回 state=installed")
    entry_ok(c)


@scenario("殘留後寫回的「新增或移除程式」項目與安裝時寫的完全相同")
def t10_entry_restored_identical(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    names = ("DisplayName", "DisplayVersion", "Publisher", "DisplayIcon", "InstallLocation", "UninstallString",
             "QuietUninstallString", "NoModify", "NoRepair", "EstimatedSize")
    before = {n: c.w.reg_get(n) for n in names}
    c.check(all(before[n] for n in ("DisplayName", "UninstallString", "InstallLocation")), "安裝時寫了項目", str(before))
    h = c.hold(d + "\\" + EXE_NAME, 15000)
    try:
        rc = c.uninstall(d)
    finally:
        h.wait(timeout=60)
    c.eq(rc, 20, "殘留 結束碼")
    after = {n: c.w.reg_get(n) for n in names}
    for n in names:
        c.eq(after[n], before[n], f"寫回的 {n} 與安裝時相同")
    c.check((c.w.startmenu_dir / f"{PRODUCT}.lnk").exists() and (c.w.startmenu_dir / f"{HDPACK_LNK}.lnk").exists()
            and c.w.desktop_lnk.exists(), "捷徑也補回")
    c.log_has("C:\\dbztest\\logs\\un.log", "沒有刪乾淨（結束碼 20）")


@scenario("登錄項目指向的位置讀不到（磁碟沒接上）：當成那一份還在，不動它的項目、捷徑與共用資料夾；讀得到而沒有標記才算失效")
def t11_entry_target_unreachable(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    # Q: is not mapped in the prefix: like a drive that is not plugged in
    c.w.reg_set("InstallLocation", "Q:\\Games\\DBZ")
    c.eq(c.uninstall(d, "/DELETEDATA"), 0, "解除 結束碼")
    c.eq(c.w.reg_get("InstallLocation"), "Q:\\Games\\DBZ", "指向讀不到位置的項目保留")
    c.check((c.w.startmenu_dir / f"{PRODUCT}.lnk").exists() and c.w.desktop_lnk.exists(), "捷徑保留")
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw\\game\\data.bin", "共用資料夾保留")
    c.log_has("C:\\dbztest\\logs\\un.log", "現在讀不到")
    e = c.T + "\\inst2"
    c.eq(c.install(e), 0, "另一份安裝")
    c.populate_player_data(e)
    c.w.u(c.T + "\\gone").mkdir()
    c.w.reg_set("InstallLocation", c.T + "\\gone")   # readable, no marker: stale
    c.eq(c.uninstall(e, "/DELETEDATA", log="C:\\dbztest\\logs\\un2.log"), 0, "解除 結束碼")
    c.check(not c.w.reg_key_exists(), "指向讀得到而沒有安裝的地方：項目已刪")
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw", "共用資料夾已刪", want=False)
    # the folder the entry points at is there, its marker exists but cannot be opened: not sure either
    other, f = c.T + "\\other-inst", c.T + "\\inst3"
    c.eq(c.install(other), 0, "被指向的那一份")
    c.eq(c.install(f), 0, "要解除的這一份")
    c.populate_player_data(f)
    c.w.reg_set("InstallLocation", other)
    h = c.hold(other + "\\" + MARKER, 15000, share=0)
    try:
        c.eq(c.uninstall(f, "/DELETEDATA", log="C:\\dbztest\\logs\\un3.log"), 0, "解除 結束碼")
    finally:
        h.wait(timeout=60)
    c.eq(c.w.reg_get("InstallLocation"), other, "標記檔打不開：項目保留")
    c.check(c.w.desktop_lnk.exists(), "捷徑保留")
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw\\game\\data.bin", "共用資料夾保留")
    c.log_has("C:\\dbztest\\logs\\un3.log", "那裡的標記檔打不開")


@scenario("寫 state=removed 失敗（標記檔被開著）：結束碼 20、寫紀錄，不靜默")
def t12_marker_write_fails(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    h = c.hold(d + "\\" + MARKER, 15000, share=1)
    try:
        rc = c.uninstall(d)
    finally:
        h.wait(timeout=60)
    c.eq(rc, 20, "結束碼")
    c.log_has("C:\\dbztest\\logs\\un.log", "無法把標記檔")
    c.eq(marker_state(c, d), "installed", "標記檔仍是 state=installed")
    saves_in_place(c, d)
    c.eq(c.install(d), 0, "重裝同位置 結束碼")
    c.exists(d + "\\" + APP_ID, "原地、沒有建子資料夾", want=False)


@scenario("免安裝版資料夾 → 原地升級 → 解除並勾刪除（清單外的檔留著、標記檔 state=removed）→ 再裝：原地")
def t04_portable_deletedata_reinstall(c: Ctx):
    d = c.T + "\\portable"
    shutil.copyfile(c.build.portable_launcher(), c.write(d + "\\" + EXE_NAME))
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    c.write(d + "\\save\\" + SYNC_MARKER, b"sync")
    others = plant_others(c, d)
    c.write(c.T + "\\src\\a.sfc", b"ROM")
    c.eq(c.install(d, "/ROM=C:\\dbztest\\src\\a.sfc"), 0, "原地升級 結束碼")
    c.eq(c.uninstall(d, "/DELETEDATA"), 0, "解除並勾刪除 結束碼")
    for f in ("save\\saves", "save\\" + SYNC_MARKER, "save\\setup\\rom.sfc", EXE_NAME, UNINST):
        c.exists(d + "\\" + f, f"清單內的 {f} 已刪", want=False)
    others_unchanged(c, d, others)
    c.eq(marker_state(c, d), "removed", "標記檔 state=removed")
    c.check(not c.w.reg_key_exists(), "登錄項目已刪")
    c.eq(c.install(d), 0, "再裝 結束碼")
    c.exists(d + "\\" + APP_ID, "原地、沒有建子資料夾", want=False)
    others_unchanged(c, d, others, "再裝後：")
    entry_ok(c)


@scenario("裝進空資料夾 → 安裝資料夾與 save\\ 根層放別的東西 → 解除並勾刪除：那些都還在")
def t05_others_survive_deletedata(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    c.write(d + "\\hd-pack-v0.3.8.zip", b"MINE")
    others = plant_others(c, d)
    c.eq(c.uninstall(d, "/DELETEDATA"), 0, "解除並勾刪除 結束碼")
    others_unchanged(c, d, others)
    c.exists(d + "\\hd-pack-v0.3.8.zip", "玩家另存的美術包保留")
    for f in ("save\\saves", "save\\cache", "save\\art-override", "save\\" + SYNC_MARKER, "hd-pack.zip",
              "hd-pack.zip.old", "hd-pack.zip.old2", "hd-pack.zip.old99", "hd-pack.zip.part"):
        c.exists(d + "\\" + f, f"清單內的 {f} 已刪", want=False)
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw", "啟動器資料夾已刪", want=False)
    c.exists(c.w.localappdata + "\\dbz-ssd-recompiled\\upstream.cfg", "原版遊戲的資料夾本身保留")
    c.eq(marker_state(c, d), "removed", "還有別的東西：標記檔 state=removed")
    c.eq(c.w.own_get("InstallDir"), d, "上次的位置保留")
    roundtrip(c, d)


@scenario("整個資料夾複製一份 → 在副本上解除：原本那份的登錄項目、捷徑、%LOCALAPPDATA%\\dbz-ssd-zhtw 都還在")
def t06_uninstall_copy(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    copy = c.T + "\\copy"
    shutil.copytree(c.w.u(d), c.w.u(copy))
    orig_before = c.snapshot(d)
    c.eq(c.uninstall(copy, "/DELETEDATA"), 0, "在副本上解除 結束碼")
    c.unchanged(d, orig_before, "原本那份一個位元組都沒變")
    c.eq(c.w.reg_get("InstallLocation"), d, "登錄項目仍指向原本那份")
    c.check((c.w.startmenu_dir / f"{PRODUCT}.lnk").exists() and c.w.desktop_lnk.exists(), "捷徑還在")
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw\\game\\data.bin", "啟動器資料夾還在")
    entry_ok(c)


@scenario("兩個位置各裝一份 → 解除較早那份：較晚那份的登錄項目與捷徑還在")
def t07_two_installs(c: Ctx):
    a, b = c.T + "\\old", c.T + "\\new"
    c.eq(c.install(a), 0, "裝到較早的位置 結束碼")
    c.populate_player_data(a)
    others = plant_others(c, a)
    c.eq(c.install(b), 0, "裝到較晚的位置 結束碼")
    c.eq(c.uninstall(a, "/DELETEDATA"), 0, "解除較早那份 結束碼")
    c.exists(a + "\\save\\saves", "較早那份的存檔已刪（有勾）", want=False)
    c.exists(b + "\\" + EXE_NAME, "較晚那份的啟動器還在")
    c.eq(c.w.reg_get("InstallLocation"), b, "登錄仍指向較晚那份")
    c.check((c.w.startmenu_dir / f"{PRODUCT}.lnk").exists() and c.w.desktop_lnk.exists(), "較晚那份的捷徑還在")
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw\\game\\data.bin", "共用的啟動器資料夾還在")
    c.log_has("C:\\dbztest\\logs\\un.log", "指向另一個安裝")
    others_unchanged(c, a, others, "較早那份：")
    entry_ok(c)


@scenario("裝 → 殘留（20）→ 重裝；裝 → 殘留 → 排除原因 → 再解除（0）")
def t08_leftover_then_reinstall_or_rerun(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    others = plant_others(c, d)
    locked = c.w.u(d + "\\save\\saves")
    os.chmod(locked, 0o555)
    try:
        c.eq(c.uninstall(d, "/DELETEDATA"), 20, "第一次 結束碼")
    finally:
        os.chmod(locked, 0o755)
    c.eq(marker_state(c, d), "installed", "標記檔停在 state=installed")
    c.exists(d + "\\" + UNINST, "解除安裝程式保留")
    c.eq(c.w.reg_get("InstallLocation"), d, "登錄項目寫回、指向這裡")
    entry_ok(c, "殘留後：")
    c.check(not c.w.desktop_lnk.exists(), "啟動器 exe 已刪：不補回指不到東西的捷徑")
    c.log_has("C:\\dbztest\\logs\\un.log", "沒有刪乾淨（結束碼 20）")
    c.eq(c.install(d), 0, "殘留後重裝 結束碼")
    c.exists(d + "\\" + APP_ID, "沒有建子資料夾", want=False)
    entry_ok(c, "重裝後：")
    others_unchanged(c, d, others, "殘留後：")
    c.eq(c.uninstall(d, "/DELETEDATA", log="C:\\dbztest\\logs\\un2.log"), 0, "排除原因後再解除 結束碼")
    others_unchanged(c, d, others, "再解除後：")
    c.exists(d + "\\save\\saves", "清單內的存檔已刪", want=False)
    c.eq(marker_state(c, d), "removed", "還有清單外的檔：標記檔 state=removed")
    c.check(not c.w.reg_key_exists(), "登錄項目已刪")


@scenario("解除安裝中途被砍（刪了登錄項目與程式檔、還沒處理標記檔）→ 重裝：原地、不建子資料夾")
def t09_interrupted(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    others = plant_others(c, d)
    # the state an uninstaller killed between its first sections and -un.Finish leaves behind
    c.w.reg_delete()
    c.w.desktop_lnk.unlink()
    c.w.u(d + "\\" + EXE_NAME).unlink()
    c.eq(marker_state(c, d), "installed", "標記檔仍是 state=installed")
    c.eq(c.install(d), 0, "重裝 結束碼")
    c.exists(d + "\\" + APP_ID, "原地、沒有建子資料夾", want=False)
    saves_in_place(c, d)
    others_unchanged(c, d, others)
    entry_ok(c)


@scenario("解除安裝不帶 /DELETEDATA：程式、捷徑、登錄項目移除；玩家資料全部保留；標記檔 state=removed")
def u01_plain(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    c.eq(c.uninstall(d), 0, "解除安裝 結束碼")
    for name in (EXE_NAME, UNINST, "hd-pack.zip.part"):
        c.exists(d + "\\" + name, want=False)
    c.eq(marker_state(c, d), "removed", "標記檔 state=removed")
    c.check(not c.w.reg_key_exists(), "登錄機碼已刪")
    c.check(not c.w.startmenu_dir.exists() and not c.w.desktop_lnk.exists(), "捷徑與開始功能表資料夾已刪")
    for name in ("save\\" + SYNC_MARKER, "save\\saves\\ssd.sav", "hd-pack.zip", "hd-pack.zip.old"):
        c.exists(d + "\\" + name)
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw\\game\\data.bin")


@scenario("/DELETEDATA 且沒有其他檔：安裝資料夾整個移除、標記檔與上次的位置都清掉")
def u03_deletedata_all(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    c.eq(c.uninstall(d, "/DELETEDATA"), 0, "解除安裝 結束碼")
    c.exists(d, "安裝資料夾已移除（含 save\\.launcher-art）", want=False)
    c.eq(c.w.own_get("InstallDir"), None, "上次的位置已清掉")
    e = c.T + "\\inst2"
    c.eq(c.install(e), 0, "另一份安裝")
    c.populate_player_data(e)
    c.write(e + "\\desktop.ini", b"[.ShellClassInfo]")
    c.eq(c.uninstall(e, "/DELETEDATA", log="C:\\dbztest\\logs\\un2.log"), 0, "只多一個 desktop.ini 時 結束碼")
    c.exists(e + "\\" + MARKER, "只剩 desktop.ini：標記檔刪掉（和判斷空資料夾同一套忽略規則）", want=False)
    c.log_has("C:\\dbztest\\logs\\un2.log", "只剩 desktop.ini／Thumbs.db，資料夾留著")
    c.log_has("C:\\dbztest\\logs\\un2.log", "已完全移除", "資料夾還在時紀錄不寫「已完全移除」", want=False)


@scenario("/DELETEDATA、save\\setup\\ 只刪三個預放檔，玩家放在 setup\\ 的其他檔保留")
def u04_setup_only(c: Ctx):
    c.write(c.T + "\\src\\a.sfc", b"A")
    c.write(c.T + "\\src\\a.ips", b"P")
    d = c.T + "\\inst"
    c.eq(c.install(d, "/ROM=C:\\dbztest\\src\\a.sfc", "/IPS=C:\\dbztest\\src\\a.ips"), 0, "安裝 結束碼")
    c.write(d + "\\save\\setup\\readme-mine.txt", b"MINE")
    c.eq(c.uninstall(d, "/DELETEDATA"), 0, "解除安裝 結束碼")
    c.exists(d + "\\save\\setup\\rom.sfc", want=False)
    c.exists(d + "\\save\\setup\\dbz102.ips", want=False)
    c.exists(d + "\\save\\setup\\readme-mine.txt")


@scenario("/DELETEDATA、save\\ 沒有同步標記：清單內的 saves\\ 照刪（只看名字，不看來歷）")
def u05_list_not_provenance(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.write(d + "\\save\\saves\\ssd.sav", b"BATTERY")
    c.eq(c.uninstall(d, "/DELETEDATA"), 0, "解除安裝 結束碼")
    c.exists(d + "\\save\\saves", "saves\\ 已刪", want=False)
    c.exists(d, "沒有別的東西：安裝資料夾移除", want=False)


@scenario("_?= 指到不是本程式的資料夾：結束碼 21，什麼都沒動")
def u07_not_ours(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    stray = c.T + "\\stray"
    c.write(stray + "\\save\\" + SYNC_MARKER, b"sync")
    c.write(stray + "\\save\\saves\\ssd.sav", b"SAVE")
    c.write(stray + "\\hd-pack.zip", b"zip")
    c.write(stray + "\\" + EXE_NAME, b"SOMEONE ELSES")
    before = c.snapshot(stray)
    inst_before = c.snapshot(d)
    un = c.w.u(d + "\\" + UNINST)
    c.eq(c.uninstall(stray, "/DELETEDATA", exe=un), 21, "解除安裝 結束碼")
    c.unchanged(stray, before, "stray 一個位元組都沒變")
    c.unchanged(d, inst_before, "真正的安裝位置也沒被動")
    c.eq(c.w.reg_get("InstallLocation"), d, "登錄仍指向真正的安裝位置")
    c.check((c.w.startmenu_dir / f"{PRODUCT}.lnk").exists(), "捷徑還在")
    c.log_has("C:\\dbztest\\logs\\un.log", "不是本程式目前的安裝")


@scenario("-un.Finish 屬於解除安裝程式且最後執行：安裝後標記仍在；紀錄的完成行在刪資料之後")
def u09_finish_section(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.exists(d + "\\" + MARKER, "安裝後標記檔仍在（安裝程式沒有執行 -un.Finish）")
    c.populate_player_data(d)
    c.eq(c.uninstall(d, "/DELETEDATA"), 0, "解除安裝 結束碼")
    log = c.read_log("C:\\dbztest\\logs\\un.log")
    i_data, i_done = log.find("刪除玩家資料"), log.find("解除安裝完成")
    c.check(0 <= i_data < i_done, "紀錄順序：刪資料 → 解除安裝完成", log[-400:])


@scenario("解除安裝連跑兩次（第二次用事先複製的解除安裝程式）：第二次標記檔已是 removed，結束碼 21、什麼都沒動")
def u10_twice(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    keep = c.build.work / "uninstall-keep.exe"
    shutil.copyfile(c.w.u(d + "\\" + UNINST), keep)
    c.eq(c.uninstall(d), 0, "第一次 結束碼")
    before = c.snapshot(d)
    c.eq(c.uninstall(d, "/DELETEDATA", exe=keep, log="C:\\dbztest\\logs\\un2.log"), 21, "第二次 結束碼")
    c.unchanged(d, before, "第二次什麼都沒動")
    c.log_has("C:\\dbztest\\logs\\un2.log", "已經解除安裝過")


@scenario("（參考）save\\ 裡有 macOS 符號連結指向外面：外面的檔不被刪。Wine 不等於 NTFS 連接點，實機另驗")
def u11_symlink_reference(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    c.populate_player_data(d)
    c.write(c.T + "\\outside\\keep.txt", b"KEEP")
    os.symlink(str(c.w.u(c.T + "\\outside")), c.w.u(d + "\\save\\link"))
    c.uninstall(d, "/DELETEDATA")
    c.exists(c.T + "\\outside\\keep.txt", "連結指向的外部檔案保留")


@scenario("啟動器 exe 正在使用（刪不掉）：結束碼 20、標記 installed、解除安裝程式與登錄項目保留；關掉後再執行一次刪乾淨")
def u13_exe_in_use(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "安裝 結束碼")
    h = c.hold(d + "\\" + EXE_NAME, 15000)
    try:
        rc = c.uninstall(d)
    finally:
        h.wait(timeout=60)
    c.eq(rc, 20, "結束碼")
    c.exists(d + "\\" + EXE_NAME, "啟動器 exe 還在（被占用）")
    c.exists(d + "\\" + UNINST, "解除安裝程式保留")
    c.eq(marker_state(c, d), "installed", "標記檔停在 state=installed")
    c.eq(c.w.reg_get("InstallLocation"), d, "登錄項目保留")
    entry_ok(c)
    c.log_has("C:\\dbztest\\logs\\un.log", d + "\\" + EXE_NAME)
    c.eq(c.uninstall(d, log="C:\\dbztest\\logs\\un2.log"), 0, "關掉後再執行一次 結束碼")
    c.exists(d, "安裝資料夾整個移除", want=False)
    c.check(not c.w.reg_key_exists(), "登錄機碼已刪")


@scenario("/DELETEDATA 要單獨出現才算：/DELETEDATA=0、/DeleteDataFalse 不刪且寫紀錄；小寫可以；前面參數有撇號也認得")
def u14_deletedata_parsing(c: Ctx):
    for i, (args, want, note) in enumerate((
            (["/DELETEDATA=0"], False, "/DELETEDATA=0"),
            (["/DeleteDataFalse"], False, "/DeleteDataFalse"),
            (["/deletedata"], True, "小寫 /deletedata"))):
        d = c.T + f"\\inst{i}"
        c.eq(c.install(d), 0, f"{note}：安裝 結束碼")
        c.write(d + "\\save\\" + SYNC_MARKER, b"sync")
        c.write(d + "\\hd-pack.zip", b"zip")
        log = f"C:\\dbztest\\logs\\un{i}.log"
        c.eq(c.uninstall(d, *args, log=log), 0, f"{note}：解除安裝 結束碼")
        c.exists(d + "\\save\\" + SYNC_MARKER, f"{note}：save\\ 的同步標記 {'已刪' if want else '保留'}", want=not want)
        c.exists(d + "\\hd-pack.zip", f"{note}：hd-pack.zip {'已刪' if want else '保留'}", want=not want)
        if not want:
            c.log_has(log, "不是單獨的 /DELETEDATA，沒有採用")
    d = c.T + "\\inst9"
    c.eq(c.install(d), 0, "撇號：安裝 結束碼")
    c.write(d + "\\save\\" + SYNC_MARKER, b"sync")
    c.w.u("C:\\dbztest\\logs\\O'Neil").mkdir()
    log = "C:\\dbztest\\logs\\O'Neil\\un.log"
    c.eq(c.uninstall(d, "/DELETEDATA", log=log), 0, "撇號：解除安裝 結束碼")
    c.exists(d + "\\save", "撇號：/LOG 有撇號時 /DELETEDATA 仍採用、save\\ 已刪", want=False)
    c.log_has(log, "刪除玩家資料", "撇號：紀錄寫到 O'Neil 資料夾")


# ----------------------------------------------------------------------------------------------------------------------
# GUI scenarios (--gui): the wizard driven through guidrive.exe, screenshots taken on the Mac side
# ----------------------------------------------------------------------------------------------------------------------

# the release's name as players see it: the installer reads launcher/RELEASE_NAME when -DDISPLAY_VERSION is not given
RELEASE_NAME = (REPO / "launcher" / "RELEASE_NAME").read_text(encoding="utf-8").strip()
SETUP_TITLE = f"{PRODUCT} {RELEASE_NAME} 安裝程式"
UN_TITLE = f"{PRODUCT} 解除安裝"
IDOK, IDCANCEL, IDYES, IDNO = 1, 2, 6, 7


class Gui:
    """One running wizard (installer or uninstaller) and the guidrive / screenshot helpers for it."""

    def __init__(self, c: Ctx, title: str, proc: subprocess.Popen):
        self.c, self.title, self.proc = c, title, proc

    def drive(self, cmd, kind="main", **kv) -> int:
        return self.c.drive(cmd, TITLE=self.title, KIND=kind, **kv)

    def wait(self, kind="main") -> bool:
        return self.drive("wait", kind) == 0

    def controls(self, kind="main"):
        """[(id, class, state, text)] of every control, from guidrive dump."""
        out = self.c.T + "\\gui\\dump.txt"
        if self.drive("dump", kind, OUT=out) != 0:
            return []
        rows = []
        for ln in self.c.read_log(out).split("\r\n"):
            parts = ln.split("\t", 3)
            if len(parts) == 4 and re.fullmatch(r"-?\d+", parts[0]):
                rows.append([int(parts[0]), parts[1], parts[2], parts[3]])
            elif rows:  # a control's own text can span lines
                rows[-1][3] += "\n" + ln
        return [tuple(r) for r in rows]

    def text_on_page(self, kind="main") -> str:
        return "\n".join(r[3] for r in self.controls(kind) if "visible=1" in r[2])

    def fits(self, kind="main"):
        """From guidrive fits: [(id, need, height, bottom, parent_height)] for every shown text (need is the height the
        installer measured for the text in the control's own font, 0 = it never measured that text; Wine cannot read
        another process's font, so it cannot be measured from outside). The shown controls' places go to
        self.layout: [(id, class, style, (left, top, right, bottom) in the parent, (parent width, height),
        (left, top, right, bottom) on the screen)]."""
        out = self.c.T + "\\gui\\fits.txt"
        self.layout = []
        if self.drive("fits", kind, OUT=out) != 0:
            return []
        rows = []
        for ln in self.c.read_log(out).split("\r\n"):
            parts = ln.split("\t")
            if len(parts) == 5 and all(re.fullmatch(r"-?\d+", v) for v in parts):
                rows.append(tuple(int(v) for v in parts))
            elif len(parts) == 14 and parts[0] == "C":
                n = [int(v) for v in parts[3:]]
                self.layout.append((int(parts[1]), parts[2], n[0], tuple(n[1:5]), tuple(n[5:7]), tuple(n[7:11])))
        return rows

    @staticmethod
    def operable(cls, style) -> bool:
        """Controls a player works: buttons, check boxes, radio buttons (not group boxes), fields, the components tree."""
        return (cls == "Button" and style & 0xF != 7) or cls in ("Edit", "SysTreeView32", "ComboBox")

    def texts_fit(self, page: str, written_later=()):
        """Every shown text on the page was measured, is at least as tall as its text needs, and stays inside its page;
        every shown control lies inside its parent, and no two controls a player works overlap.
        written_later: texts NSIS writes after the page is shown (the components page's space required), so the
        installer cannot measure them: they must be at least one line (the shortest measured text) tall."""
        rows = self.fits()
        self.c.check(bool(rows), f"{page}：量得到文字的高度")
        one_line = min((need for _, need, *_ in rows if need > 0), default=0)
        for cid, need, have, bottom, parent_h in rows:
            if cid in written_later and need == 0:
                self.c.check(have >= one_line > 0, f"{page}：文字 {cid}（之後才寫）至少一行高", f"有 {have}、一行 {one_line}")
            else:
                self.c.check(0 < need <= have, f"{page}：文字 {cid} 整段放得下", f"需要 {need}、有 {have}")
            self.c.check(bottom <= parent_h, f"{page}：文字 {cid} 沒有超出頁面", f"底 {bottom}、頁高 {parent_h}")
        self.c.check(bool(self.layout), f"{page}：讀得到控制項的位置")
        for cid, cls, style, (l, t, r, b), (pw, ph), _ in self.layout:
            self.c.check(0 <= l and 0 <= t and r <= pw and b <= ph, f"{page}：控制項 {cid}（{cls}）在所屬視窗裡",
                         f"{l},{t},{r},{b}／{pw}x{ph}")
        work = [(cid, cls, scr) for cid, cls, style, _, _, scr in self.layout if self.operable(cls, style)]
        for i, (ida, ca, (al, at, ar, ab)) in enumerate(work):
            for idb, cb, (bl, bt, br, bb) in work[i + 1:]:
                over = min(ar, br) > max(al, bl) and min(ab, bb) > max(at, bt)
                self.c.check(not over, f"{page}：{ida}（{ca}）與 {idb}（{cb}）沒有重疊",
                             f"{al},{at},{ar},{ab} / {bl},{bt},{br},{bb}")

    def panel_stack(self, page: str):
        """Welcome / finish page (the skin's panel): the text (1202), then the link (an owner-drawn button, any ID),
        then the check boxes, top to bottom in that order, each at most two lines below the one before; the link's left
        edge on the text's, and the link and the boxes inside the text's column (the panel's inside). The IDs of the
        link and boxes are not fixed (nsDialogs numbers them in the order MUI makes them), so they are found by kind."""
        rows = self.fits()
        one_line = min((need for _, need, *_ in rows if need > 0), default=0)
        ctl = [(cid, cls, style & 0xF, rect) for cid, cls, style, rect, _, _ in self.layout]
        text = [rect for cid, cls, _, rect in ctl if cid == 1202]
        self.c.check(len(text) == 1, f"{page}：有說明段落（1202）")
        if len(text) != 1:
            return [], []
        tl, tt, tr, tb = text[0]
        link = [(cid, rect) for cid, cls, kind, rect in ctl if cls == "Button" and kind == 0xB]
        boxes = [(cid, rect) for cid, cls, kind, rect in ctl if cls == "Button" and (2 <= kind <= 6 or kind == 9)]
        above, what = tb, "說明段落"
        for cid, (l, t, r, b) in link + boxes:
            name = "連結" if (cid, (l, t, r, b)) in link else "核取方塊"
            self.c.check(above <= t <= above + 2 * one_line, f"{page}：{name} {cid} 緊接在{what}下面",
                         f"上一個底 {above}、它的頂 {t}、一行 {one_line}")
            self.c.check(tl <= l and r <= tr, f"{page}：{name} {cid} 在段落那一欄裡（面板內）",
                         f"它 {l}..{r}、段落 {tl}..{tr}")
            above, what = b, f"{name} {cid}"
        for cid, (l, t, r, b) in link:
            self.c.check(l == tl, f"{page}：連結 {cid} 的左緣和說明段落對齊", f"連結 {l}、段落 {tl}")
        return link, boxes

    def folder_label(self, page: str):
        """The directory page as in the design: the group box hidden, the "▶ 安裝資料夾" label (static 1997,
        SKIN_LABEL_ID in ui/skin.nsh) shown with the group box's caption."""
        rows = self.controls()
        label = [r for r in rows if r[0] == 1997 and "visible=1" in r[2]]
        self.c.check(len(label) == 1 and label[0][3] == "安裝資料夾", f"{page}：有「安裝資料夾」標籤",
                     str(label[:1]))
        box = [r for r in rows if r[0] == 1020]
        self.c.check(bool(box) and all("visible=0" in r[2] for r in box), f"{page}：群組框看不到", str(box[:1]))

    def skinned(self) -> bool:
        """The skin's composed background (static 1999, SKIN_BG_ID in ui/skin.nsh) is on the window."""
        return any(r[0] == 1999 and "visible=1" in r[2] for r in self.controls())

    def wait_text(self, text, kind="main", timeout=30) -> bool:
        end = time.time() + timeout
        while time.time() < end:
            if text in self.text_on_page(kind):
                return True
            time.sleep(0.5)
        return False

    def box_text(self) -> str:
        out = self.c.T + "\\gui\\box.txt"
        if self.drive("boxtext", "box", OUT=out) != 0:
            return ""
        return self.c.read_log(out)

    def ctl_id(self, cls, nth=0, text=None, kind="main"):
        rows = [r for r in self.controls(kind) if r[1] == cls and "visible=1" in r[2] and (text is None or text in r[3])]
        return rows[nth][0] if len(rows) > nth else None

    def click(self, cid, kind="main"):
        rc = self.drive("click", kind, ID=str(cid))
        time.sleep(0.8)
        return rc

    def settext(self, cid, text):
        return self.drive("settext", ID=str(cid), TEXT=text)

    def shot(self, name, kind="main"):
        """Screenshot of this wizard's window (main: the biggest window with the title, box: the smallest)."""
        if not self.c.shots:
            return
        time.sleep(0.6)
        r = subprocess.run([str(self.c.build.winlist())], capture_output=True, timeout=30)
        wins = []
        for ln in r.stdout.decode("utf-8", "replace").splitlines():
            parts = ln.split("\t", 3)
            if len(parts) == 4 and parts[3].startswith(self.title):
                x, y, w, h = (int(v) for v in parts[2].split(","))
                wins.append((w * h, int(parts[0])))
        if not wins:
            self.c.check(False, f"截圖 {name}：找不到視窗")
            return
        wins.sort()
        wid = wins[-1][1] if kind == "main" else wins[0][1]
        path = self.c.shots / f"{name}.png"
        subprocess.run(["screencapture", "-x", "-o", "-l", str(wid), str(path)], timeout=30)
        self.c.check(path.exists() and path.stat().st_size > 5000, f"截圖 {name}", str(path))

    def finish(self, timeout=60) -> int:
        try:
            return self.proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.c.check(False, "精靈在時限內沒有結束")
            return -1


def gui_install(c: Ctx, exe=None, *args) -> Gui:
    g = Gui(c, SETUP_TITLE, c.w.popen(exe or c.setup_exe, ["/LOG=C:\\dbztest\\logs\\gui.log", *args]))
    c.check(g.wait(), "安裝精靈出現")
    return g


def gui_uninstall(c: Ctx, d, wizard=True) -> Gui:
    c.build.uncopies += 1
    copy = c.build.work / f"un{c.build.uncopies}" / "Au_.exe"
    copy.parent.mkdir()
    shutil.copyfile(c.w.u(d + "\\" + UNINST), copy)
    # before the wizard exists (un.onInit) a message box's title is not the wizard's caption: match on the product
    g = Gui(c, UN_TITLE if wizard else PRODUCT, c.w.popen(copy, ["/LOG=C:\\dbztest\\logs\\gui-un.log", f"_?={d}"]))
    if wizard:
        c.check(g.wait(), "解除安裝精靈出現")
    return g


def dir_attempt(c: Ctx, g: Gui, path, expect, shot=None, answer=IDOK):
    """On the directory page: type `path`, press Next, expect a message box containing `expect`, answer it."""
    g.settext(1019, path)
    g.click(IDOK)
    text = g.box_text()
    c.check(expect in text, f"選位置頁 {path}：提示「{expect}」", text[:200])
    if shot:
        g.shot(shot, "box")
    g.click(answer, "box")
    c.check(g.ctl_id("Edit") == 1019, f"選位置頁 {path}：按完仍在選位置頁")


@scenario("圖形介面安裝：各頁截圖；根目錄的提示；ROM 頁檢查；完成頁（別人的資料夾見 g06）", gui=True)
def g01_install_walkthrough(c: Ctx):
    c.write(c.T + "\\src\\game.bin", b"BIN")
    c.write(c.T + "\\src\\dbz.sfc", b"ROMDATA" * 100)
    g = gui_install(c)
    g.shot("01-welcome")
    c.check(g.skinned(), "歡迎頁：設計 B 的底圖")
    g.texts_fit("歡迎頁")
    g.panel_stack("歡迎頁")
    text = g.text_on_page()
    c.check("之後出新版時，下載新版的安裝程式再裝一次即可" in text, "歡迎頁（無 AUTO_UPDATE）的更新說明")
    c.check("高畫質的畫面要靠 HD 美術包，\n可以選擇要不要下載；\n不下載也能玩，畫面是原版像素。" in text
            and "HD 美術包可以選擇是否從上游下載" not in text, "歡迎頁：高畫質要靠 HD 美術包、不下載也能玩", text[-300:])
    g.click(IDOK)
    c.check(g.ctl_id("Edit") == 1019, "第二頁是選安裝位置")
    g.shot("02-directory")
    c.check(g.skinned(), "選位置頁：設計 B 的底圖")
    g.texts_fit("選位置頁")
    g.folder_label("選位置頁")
    # a drive root: moved to C:\DBZ-SSD-zhTW with a notice, then checked as usual (empty: on to the ROM page)
    g.settext(1019, "C:\\")
    g.click(IDOK)
    text = g.box_text()
    c.check("不能直接裝在磁碟的根目錄" in text and "C:\\" + APP_ID in text, "選位置頁 C:\\：告知改裝到 C:\\DBZ-SSD-zhTW", text[:200])
    g.shot("03-directory-root", "box")
    g.click(IDOK, "box")
    c.check(g.wait_text("選擇遊戲 ROM 與 IPS"), "根目錄改到子資料夾後照常往下")
    g.click(3)
    c.check(g.ctl_id("Edit") == 1019, "按「上一步」回到選位置頁")
    field = [r[3].rstrip("\\") for r in g.controls() if r[0] == 1019]
    c.eq(field[:1], ["C:\\" + APP_ID], "欄位是改過的位置")

    g.settext(1019, c.T + "\\gui-inst")
    g.click(IDOK)
    c.check(g.wait_text("選擇遊戲 ROM 與 IPS"), "第三頁是選 ROM 與 IPS")
    g.shot("06-rom")
    c.check(g.skinned(), "ROM 頁：設計 B 的底圖")
    g.texts_fit("ROM 頁")
    text = g.text_on_page()
    c.check("第一次開啟遊戲時，程式會自己檢查 ROM 是不是可用的版本" in text and "原檔不會被改動" in text
            and "請你重新選擇" in text and "檔案對不對由啟動器" not in text, "ROM 頁：自我檢查的說明（舊的那句已換掉）",
            text[-400:])
    rom_edit = g.ctl_id("Edit", 0)
    g.settext(rom_edit, c.T + "\\src\\game.bin")
    g.click(IDOK)
    text = g.box_text()
    c.check("遊戲 ROM 要是 .sfc 或 .smc 檔" in text, "ROM 頁：副檔名不對的提示", text[:200])
    g.shot("07-rom-bad-extension", "box")
    g.click(IDOK, "box")
    g.settext(rom_edit, c.T + "\\src\\dbz.sfc")
    g.click(IDOK)
    c.check(g.wait_text(HD_PAGE_TITLE), "第四頁是 HD 美術包")
    g.shot("06b-hd-pack")
    c.check(g.skinned(), "HD 美術包頁：設計 B 的底圖")
    g.texts_fit("HD 美術包頁")
    hd_page_shown(c, g, "HD 美術包頁")
    g.click(IDOK)
    c.check(g.wait_text("安裝完成"), "安裝後出現完成頁")
    g.shot("08-finish")
    c.check(g.skinned(), "完成頁：設計 B 的底圖")
    g.texts_fit("完成頁")
    link, boxes = g.panel_stack("完成頁")
    c.check(len(link) == 1 and len(boxes) == 1, "完成頁：一個 Ko-fi 連結、一個核取方塊", f"{link} {boxes}")
    run_box = g.ctl_id("Button", text="立即開啟遊戲")
    c.check(run_box is not None, "完成頁有「立即開啟遊戲」")
    boxes = [r for r in g.controls() if r[1] == "Button" and "visible=1" in r[2] and "HD" in r[3]]
    c.check(not boxes, "完成頁沒有「下載 HD 美術包」核取方塊（移到 HD 美術包頁）", str(boxes))
    if run_box is not None:
        g.drive("uncheck", ID=str(run_box))
    g.click(IDOK)
    c.eq(g.finish(), 0, "安裝精靈結束碼")
    d = c.T + "\\gui-inst"
    c.exists(d + "\\" + MARKER)
    p = c.w.u(d + "\\save\\setup\\rom.sfc")
    c.eq(p.read_bytes() if p.exists() else None, b"ROMDATA" * 100, "選的 ROM 已複製到 save\\setup\\rom.sfc")
    c.eq(c.w.reg_get("InstallLocation"), d, "登錄 InstallLocation")
    # the title font is the installer's own (AddFontResourceEx with FR_PRIVATE): never put in the system's or the user's
    # font folder. This only checks the font folders: that it is loaded privately and released at the end cannot be seen
    # in Wine (Wine deletes a font file even while it is loaded), that is item 25 of the hardware checklist (README)
    for fonts in ("C:\\windows\\Fonts", c.w.localappdata + "\\Microsoft\\Windows\\Fonts"):
        p = c.w.u(fonts)
        found = sorted(f.name for f in p.iterdir() if "lxgw" in f.name.lower()) if p.is_dir() else []
        c.eq(found, [], f"標題字型沒有裝進 {fonts}")


@scenario("歡迎頁：-DAUTO_UPDATE 的更新說明", gui=True)
def g02_welcome_autoupdate(c: Ctx):
    exe = c.build.work / "DBZ-SSD-zh-TW-RC1-autoupdate-test.exe"
    if not exe.exists():
        c.build.compile(c.nsi, {"LAUNCHER_EXE": c.build.standin(), "RELEASE": "45", "OUTFILE": exe,
                                "AUTO_UPDATE": None})
    g = gui_install(c, exe)
    text = g.text_on_page()
    c.check("啟動器開啟時會詢問是否更新" in text, "歡迎頁（AUTO_UPDATE）的更新說明", text[-200:])
    c.check("下載新版的安裝程式再裝一次" not in text, "歡迎頁（AUTO_UPDATE）沒有「再裝一次」的說法")
    g.shot("01-welcome-auto-update")
    g.texts_fit("歡迎頁（AUTO_UPDATE）")
    g.click(IDCANCEL)
    g.click(IDYES, "box")
    g.finish()


@scenario("重裝到新位置、舊位置有 save\\：提醒存檔不會搬過來，按「否」留在選位置頁", gui=True)
def g03_reinstall_elsewhere(c: Ctx):
    old = c.T + "\\old"
    c.eq(c.install(old), 0, "先靜默裝到舊位置")
    c.write(old + "\\save\\saves\\slot1.srm", b"SAVE")
    g = gui_install(c)
    text = g.text_on_page()
    c.check(f"偵測到已安裝的 {RELEASE_NAME}，這會在原位置重新安裝，存檔與 HD 美術包都會保留。" in text,
            "歡迎頁第一段：同一版，在原位置重新安裝", text[:300])
    g.shot("01c-welcome-reinstall")
    g.texts_fit("歡迎頁（重新安裝）")
    g.click(IDOK)
    field = [r[3].rstrip("\\") for r in g.controls() if r[0] == 1019]  # the page shows it with a trailing "\"
    c.eq(field[:1], [old], "選位置頁預設帶出上次的位置（InstallDirRegKey）")
    g.settext(1019, c.T + "\\new")
    g.click(IDOK)
    text = g.box_text()
    c.check("存檔與 ROM 設定（那裡的 save 資料夾）會留在舊位置" in text, "提醒存檔不會搬過來", text[:400])
    c.check("舊位置的程式也不會被移除" in text and "「新增或移除程式」會改成指向新位置" in text
            and "uninstall-dbz-zhtw.exe" in text and "存檔預設保留" in text,
            "提醒也說明舊位置的程式不會被移除、怎麼移除它", text[:400])
    g.shot("09-directory-moved", "box")
    g.click(IDNO, "box")
    c.check(g.ctl_id("Edit") == 1019, "按「否」後仍在選位置頁")
    g.click(IDCANCEL)
    g.click(IDYES, "box")
    g.finish()
    c.exists(c.T + "\\new", "按「否」後新位置沒有被建立", want=False)


@scenario("圖形介面解除安裝：勾「同時刪除」→ 再確認（先按否、再按是）→ 玩家資料刪除", gui=True)
def g04_uninstall_deletedata(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "先靜默安裝")
    c.populate_player_data(d)
    g = gui_uninstall(c, d)
    g.shot("11-uninstall-confirm")
    c.check(g.skinned(), "解除安裝確認頁：設計 B 的底圖")
    g.texts_fit("解除安裝確認頁")
    g.click(IDOK)
    tree = g.ctl_id("SysTreeView32")
    c.check(tree is not None, "第二頁是選擇要移除的項目（元件樹）")
    g.shot("12-uninstall-components")
    c.check(g.skinned(), "元件頁：設計 B 的底圖")
    g.texts_fit("元件頁", written_later=(1023,))
    if tree is not None:
        g.drive("treecheck", ID=str(tree), TEXT="1")
        time.sleep(0.5)
    g.shot("13-uninstall-components-checked")
    g.click(IDOK)
    text = g.box_text()
    c.check("確定要永久刪除" in text and "dbz-ssd-zhtw" in text, "勾了之後再確認一次，列出會刪的路徑", text[:300])
    g.shot("14-uninstall-confirm-delete", "box")
    g.click(IDNO, "box")
    c.check(g.ctl_id("SysTreeView32") is not None, "按「否」留在元件頁")
    c.exists(d + "\\save\\" + SYNC_MARKER, "按「否」之後 save\\ 還在")
    g.click(IDOK)
    g.click(IDYES, "box")
    c.check(g.wait_text("完成") or g.wait_text("關閉"), "解除安裝跑完")
    g.shot("15-uninstall-done")
    c.check(g.skinned(), "解除安裝完成頁：設計 B 的底圖")
    g.texts_fit("解除安裝完成頁")
    g.click(IDOK)
    g.finish()
    c.exists(d + "\\save", "save\\ 已刪", want=False)
    c.exists(d + "\\hd-pack.zip", "hd-pack.zip 已刪", want=False)
    c.exists(c.w.localappdata + "\\dbz-ssd-zhtw", "啟動器資料夾已刪", want=False)
    c.check(not c.w.reg_key_exists(), "登錄機碼已刪")


@scenario("圖形介面解除安裝的位置不是本程式的安裝：開頭就告知並中止，結束碼 21、什麼都沒刪", gui=True)
def g05_uninstall_not_ours(c: Ctx):
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "先靜默安裝")
    c.w.u(d + "\\" + MARKER).unlink()
    c.w.reg_set("InstallLocation", c.T + "\\elsewhere")
    c.write(d + "\\save\\" + SYNC_MARKER, b"sync")
    before = c.snapshot(d)
    g = gui_uninstall(c, d, wizard=False)
    text = g.box_text()
    c.check("不是本程式目前的安裝位置" in text and "沒有刪除任何東西" in text, "開頭的訊息說明中止原因", text[:300])
    g.shot("16-uninstall-not-ours", "box")
    g.click(IDOK, "box")
    c.eq(g.finish(), 21, "結束碼")
    c.unchanged(d, before, "安裝資料夾一個位元組都沒變")


@scenario("選到已有其他檔案的資料夾：改裝到裡面的 DBZ-SSD-zhTW 並告知，原資料夾不變；子資料夾也有別的檔就擋下", gui=True)
def g06_nonempty_goes_to_subfolder(c: Ctx):
    pick = c.T + "\\pick"
    c.write(pick + "\\save\\other\\keep.txt", b"KEEP")
    c.write(pick + "\\save\\" + SYNC_MARKER, b"sync")   # earlier saves in the chosen folder
    c.write(pick + "\\notes.txt", b"mine")
    before = c.snapshot(pick)
    g = gui_install(c)
    g.click(IDOK)
    g.settext(1019, pick)
    g.click(IDOK)
    text = g.box_text()
    sub = pick + "\\" + APP_ID
    c.check("改裝到它裡面的新資料夾" in text and sub in text, "告知改裝到子資料夾", text[:300])
    c.check("這個資料夾裡有先前的存檔" in text and pick + "\\save\\" in text, "告知原資料夾裡有先前的存檔、遊戲不會讀到", text[:400])
    g.shot("05-directory-subfolder", "box")
    g.click(IDOK, "box")
    c.check(g.wait_text("選擇遊戲 ROM 與 IPS"), "繼續到選 ROM 頁")
    g.click(IDOK)
    hd_page(c, g, "子資料夾")
    c.check(g.wait_text("安裝完成"), "安裝完成")
    run_box = g.ctl_id("Button", text="立即開啟遊戲")
    if run_box is not None:
        g.drive("uncheck", ID=str(run_box))
    g.click(IDOK)
    c.eq(g.finish(), 0, "安裝精靈結束碼")
    c.exists(sub + "\\" + MARKER, "裝在子資料夾")
    c.eq(c.w.reg_get("InstallLocation"), sub, "登錄記子資料夾")
    c.unchanged(pick, before, "原資料夾一個位元組都沒變（子資料夾以外）", ignore=(APP_ID,))

    pick2 = c.T + "\\pick2"
    c.write(pick2 + "\\notes.txt", b"mine")
    c.write(pick2 + "\\" + APP_ID + "\\other.txt", b"OTHER")
    before2 = c.snapshot(pick2)
    g = gui_install(c)
    g.click(IDOK)
    g.settext(1019, pick2)
    g.click(IDOK)
    text = g.box_text()
    c.check("也有別的檔案" in text, "子資料夾也有別的檔：擋下並說明", text[:300])
    g.shot("05b-directory-subfolder-taken", "box")
    g.click(IDOK, "box")
    field = [r[3].rstrip("\\") for r in g.controls() if r[0] == 1019]
    c.eq(field[:1], [pick2], "仍在選位置頁、欄位還是原本選的資料夾")
    g.click(IDCANCEL)
    g.click(IDYES, "box")
    g.finish()
    c.unchanged(pick2, before2, "第二個資料夾一個位元組都沒變")


def browse_to(c: Ctx, g: Gui, path) -> str:
    """Pick `path` with the directory page's Browse button: the folder dialog opens on the folder in the field, OK takes
    it (the code path of a folder picked there). Returns what the field holds afterwards."""
    g.settext(1019, path)
    g.click(1001)
    bg = Gui(c, "Browse for Folder", g.proc)
    c.check(bg.wait("any"), f"瀏覽 {path}：資料夾對話框出現")
    bg.click(1, "any")
    time.sleep(0.8)
    field = [r[3] for r in g.controls() if r[0] == 1019]
    return field[0] if field else ""


HD_PAGE_TITLE = "HD 高畫質美術包"
HD_BOX_TEXT = "安裝完成後下載 HD 美術包"


def hd_page(c: Ctx, g: Gui, what, download=False):
    """On the HD art pack page (after the ROM page): tick "download after the install" when `download`, then Next.
    Returns the check box's ID (None when the page has none: a pack is there already)."""
    c.check(g.wait_text(HD_PAGE_TITLE), f"{what}：到了 HD 美術包頁")
    box = g.ctl_id("Button", text=HD_BOX_TEXT)
    if download and box is not None:
        g.drive("check", ID=str(box))
    g.click(IDOK)
    return box


HD_PAGE_WORDS = ("想要高畫質的畫面，需要 HD 美術包", "畫面是原版的像素圖", "上游作者 JackRushante 的作品", "CC BY-NC-SA 4.0",
                 "不是本繁中版的內容", "約 462 MB", "安裝完成後才下載", "可隨時取消", "從開始功能表下載",
                 "核對檔案大小與 SHA-256", "和上游原檔完全相同才使用", "上游抓不到時會改從備援下載")


def hd_page_shown(c: Ctx, g: Gui, what):
    """The HD art pack page without a pack in the folder: why the pack (each sentence as the launcher does it),
    the comparison picture, the check box."""
    text = g.text_on_page()
    for words in HD_PAGE_WORDS:
        c.check(words in text, f"{what}：說明有「{words}」", text[-300:])
    c.check(g.ctl_id("Button", text=HD_BOX_TEXT) is not None, f"{what}：有「{HD_BOX_TEXT}」核取方塊")
    hd_pictures(c, g, what)


def hd_pictures(c: Ctx, g: Gui, what):
    """The two comparison pictures (原版 above, HD below): the same size, at least the 100 % size (180x100), one under
    the other in the left column, inside the page, and everything else (the text, the check box) to their right."""
    g.fits()
    # pictures on the page other than the skin's background (static 1999, SKIN_BG_ID in ui/skin.nsh)
    pics = [((l, t, r, b), parent) for cid, cls, style, (l, t, r, b), parent, _ in g.layout
            if cls == "Static" and style & 0x1F == 0x0E and cid != 1999]
    c.check(len(pics) == 2, f"{what}：有兩張對照圖（原版、HD）", str(pics))
    if len(pics) != 2:
        return
    (a, page), (b, _) = sorted(pics, key=lambda p: p[0][1])
    ph = page[1]
    size = lambda r: (r[2] - r[0], r[3] - r[1])
    c.check(size(a) == size(b) and size(a)[0] >= 180 and size(a)[1] >= 100, f"{what}：兩張一樣大、至少 180x100",
            f"{size(a)} {size(b)}")
    c.check(a[0] == b[0] == 0 and a[3] <= b[1] and b[3] <= ph, f"{what}：兩張上下疊在左欄、在頁面裡",
            f"{a} {b} 頁高 {ph}")
    # the page's other controls (the same parent: the main window's buttons and header have another size)
    others = [(cid, cls, rect) for cid, cls, style, rect, parent, _ in g.layout
              if parent == page and cid != 1999 and not (cls == "Static" and style & 0x1F == 0x0E)]
    c.check(bool(others), f"{what}：圖的右邊有說明", str(others))
    for cid, cls, (l, t, r, bt) in others:
        c.check(l >= a[2], f"{what}：{cid}（{cls}）在圖的右邊、沒有蓋到圖", f"左緣 {l}、圖右緣 {a[2]}")


def launcher_calls(c: Ctx, want: int, timeout=15) -> list:
    """The recorder's calls (`[]` the game, `[--hd-pack]` the pack), once `want` of them came (Exec does not wait),
    plus a moment for one more that should not come."""
    p = c.w.u(LAUNCHER_CALLS)
    read = lambda: p.read_text(encoding="latin-1").split() if p.exists() else []
    end = time.time() + timeout
    while time.time() < end and len(read()) < want:
        time.sleep(0.5)
    time.sleep(3)
    return read()


def finish_from_rom_page(c: Ctx, g: Gui, what):
    c.check(g.wait_text("選擇遊戲 ROM 與 IPS"), f"{what}：到選 ROM 頁")
    g.click(IDOK)
    hd_page(c, g, what)
    c.check(g.wait_text("安裝完成"), f"{what}：安裝完成")
    run_box = g.ctl_id("Button", text="立即開啟遊戲")
    if run_box is not None:
        g.drive("uncheck", ID=str(run_box))
    g.click(IDOK)
    c.eq(g.finish(), 0, f"{what}：精靈結束碼")


@scenario("按「瀏覽」選資料夾：免安裝版（原地）、本程式的（原地）、別人的非空資料夾（子資料夾）、空資料夾（直接裝）", gui=True)
def g07_browse(c: Ctx):
    cases = []
    p = c.T + "\\portable"
    shutil.copyfile(c.build.portable_launcher(), c.write(p + "\\" + EXE_NAME))
    c.write(p + "\\save\\saves\\ssd.sav", b"BATTERY")
    cases.append(("免安裝版資料夾", p, p))
    o = c.T + "\\ours"
    cases.append(("本程式的資料夾", o, o))
    f = c.T + "\\foreign"
    c.write(f + "\\notes.txt", b"mine")
    cases.append(("別人的非空資料夾", f, f + "\\" + APP_ID))
    e = c.T + "\\empty"
    c.w.u(e).mkdir()
    cases.append(("空資料夾", e, e))
    cases.append(("磁碟根目錄", "C:\\", "C:\\" + APP_ID))
    for what, folder, target in cases:
        if what == "本程式的資料夾":
            c.eq(c.install(folder), 0, f"{what}：先靜默裝好")
        c.w.reg_delete()   # no "last time it was elsewhere" prompt between the cases
        g = gui_install(c)
        g.click(IDOK)
        field = browse_to(c, g, folder)
        c.eq(field.rstrip("\\"), folder.rstrip("\\"), f"{what}：瀏覽後欄位就是選的資料夾（沒有自動接上 \\{APP_ID}）")
        g.shot(f"17-browse-{len(c.checks)}")
        g.click(IDOK)
        if target != folder:
            text = g.box_text()
            c.check(("改裝到它裡面的新資料夾" in text or "會改裝到" in text) and target in text,
                    f"{what}：告知改裝到 {target}", text[:200])
            g.click(IDOK, "box")
        finish_from_rom_page(c, g, what)
        c.eq(marker_state(c, target), "installed", f"{what}：裝在 {target}")
        if target == folder:
            c.exists(folder + "\\" + APP_ID, f"{what}：沒有建子資料夾", want=False)
    saves_in_place(c, p, "免安裝版：")


@scenario("選位置頁選到標記檔打不開的安裝：擋下並請玩家關掉遊戲或防毒後再試，留在選位置頁，不改裝到子資料夾", gui=True)
def g08_marker_unreadable(c: Ctx):
    # Limit: the marker is held by guidrive hold for 30 s (started 2 s before the wizard): see s25
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "先靜默安裝")
    before = c.snapshot(d)
    h = c.hold(d + "\\" + MARKER, 30000, share=0)
    try:
        g = gui_install(c)
        g.click(IDOK)
        g.settext(1019, d)
        g.click(IDOK)
        text = g.box_text()
        c.check("安裝標記檔打不開" in text and d + "\\" + MARKER in text, "告知標記檔打不開與它的位置", text[:300])
        c.check("請關掉遊戲與其他正在使用它的程式（以及防毒軟體的掃描）後再試一次" in text, "請玩家關掉遊戲、其他程式或防毒後再試",
                text[:300])
        c.check("改裝到" not in text, "沒有說要改裝到子資料夾", text[:300])
        g.shot("05c-directory-marker-locked", "box")
        g.click(IDOK, "box")
        c.check(g.ctl_id("Edit") == 1019, "按完仍在選位置頁")
        field = [r[3].rstrip("\\") for r in g.controls() if r[0] == 1019]
        c.eq(field[:1], [d], "欄位仍是選的位置（沒有改成子資料夾）")
        g.click(IDCANCEL)
        g.click(IDYES, "box")
        g.finish()
    finally:
        h.wait(timeout=60)
    c.unchanged(d, before, "安裝資料夾沒有任何變動")
    c.exists(d + "\\" + APP_ID, "沒有建子資料夾", want=False)



@scenario("選位置頁通過後標記檔才被鎖住：安裝段開頭的再分類擋下，結束碼 14、用「標記檔打不開」的說明（不是「不是本程式的資料夾」）", gui=True)
def g10_marker_locked_after_directory_page(c: Ctx):
    # Limit: the marker is held by guidrive hold (see s25), started on the ROM page, after the directory page passed
    d = c.T + "\\inst"
    c.eq(c.install(d), 0, "先靜默安裝")
    before = c.snapshot(d)
    g = gui_install(c)
    g.click(IDOK)
    g.settext(1019, d)
    g.click(IDOK)
    c.check(g.wait_text("選擇遊戲 ROM 與 IPS"), "選位置頁通過，到了 ROM 頁")
    h = c.hold(d + "\\" + MARKER, 25000, share=0)
    try:
        g.click(IDOK)
        hd_page(c, g, "標記檔被佔用")
        text = g.box_text()
        c.check("安裝標記檔打不開" in text and d + "\\" + MARKER in text, "安裝時擋下並說明標記檔打不開", text[:300])
        c.check("不是本程式的安裝位置" not in text, "沒有說成「不是本程式的資料夾」", text[:300])
        g.shot("10-install-marker-locked", "box")
        g.click(IDOK, "box")
        time.sleep(1)
        g.click(IDCANCEL)
        rc = g.finish()
    finally:
        h.wait(timeout=60)
    c.eq(rc, 14, "安裝精靈結束碼")
    c.log_has("C:\\dbztest\\logs\\gui.log", "中止（結束碼 14）：這個位置的安裝標記檔打不開")
    c.unchanged(d, before, "安裝資料夾沒有任何變動")


@scenario("解除安裝的位置在 Windows 資料夾裡（不能刪玩家資料）：原因寫在元件頁上方說明裡、整段看得到，樹狀項目只留「（無法使用）」", gui=True)
def g09_uninstall_blocked_reason(c: Ctx):
    src = c.T + "\\src-inst"
    c.eq(c.install(src), 0, "先裝一份，借它的解除安裝程式")
    d = BLOCKED_TEST_DIR   # cleaned by Ctx.reset
    c.write(d + "\\" + MARKER, b"app=DBZ-SSD-zhTW\r\nversion=v44\r\ninstalled=2026-11-15 10:00:00\r\nstate=installed\r\n")
    shutil.copyfile(c.w.u(src + "\\" + UNINST), c.w.u(d + "\\" + UNINST))
    g = gui_uninstall(c, d)
    g.click(IDOK)
    c.check(g.ctl_id("SysTreeView32") is not None, "到了選擇要移除的項目頁")
    top = [r[3] for r in g.controls() if r[0] == 1006 and "visible=1" in r[2]]
    c.check(bool(top) and "第二項無法使用：安裝位置在 Windows 資料夾裡" in top[0], "原因寫在上方說明裡", str(top[:1])[-160:])
    g.shot("16-uninstall-components-blocked")
    g.texts_fit("元件頁（不能刪資料）", written_later=(1023,))
    g.click(IDCANCEL)
    g.click(IDYES, "box")
    g.finish()
    # Limit: the tree's item text cannot be read from another process (TVM_GETITEM needs memory in the wizard), so
    # this compares strings in the source, not what the tree shows
    nsi = c.nsi.read_text(encoding="utf-8-sig")
    c.check("與遊戲設定（無法使用）\"" in nsi and "（無法使用：$U_Block）" not in nsi,
            "樹狀項目只寫「（無法使用）」，原因不接在後面")


# Wine draws the wizard in Heiti TC, narrower than Windows' 新細明體: there the earlier finish page's "下載 HD 美術包（之後也可從開始功能表）" fit
# on one line while on Windows it wrapped (and was cut off). The check boxes are lengthened so they wrap in Wine too;
# ASCII words, because Wine breaks lines only at spaces
FINISH_BOX_PAD = " (test padding, long enough that this check box text has to wrap onto several lines in Wine as well)"


def wraps(c: Ctx, g: Gui, page, cid):
    """The check box `cid` is in the fits check and, in this scenario, does wrap (needs two lines or more)."""
    rows = {i: need for i, need, *_ in g.fits()}
    one_line = min((n for n in rows.values() if n > 0), default=0)
    c.check(cid in rows, f"{page}：核取方塊 {cid} 在檢查範圍內", str(sorted(rows)))
    c.check(rows.get(cid, 0) >= 2 * one_line > 0, f"{page}：核取方塊 {cid} 這個情境確實折成多行",
            f"需要 {rows.get(cid)}、一行 {one_line}")


@scenario("核取方塊的文字折成多行時整段看得到：HD 美術包頁與完成頁（-DTEST_FINISH_BOX_PAD 加長核取方塊）", gui=True)
def g11_finish_boxes_wrap(c: Ctx):
    exe = c.build.work / "DBZ-SSD-zh-TW-RC1-longbox-test.exe"
    if not exe.exists():
        c.build.compile(c.nsi, {"LAUNCHER_EXE": c.build.standin(), "RELEASE": "45", "OUTFILE": exe,
                                "TEST_FINISH_BOX_PAD": FINISH_BOX_PAD})
    c.write(c.T + "\\src\\dbz.sfc", b"ROMDATA" * 100)
    g = gui_install(c, exe)
    g.click(IDOK)
    g.settext(1019, c.T + "\\gui-long")
    g.click(IDOK)
    c.check(g.wait_text("選擇遊戲 ROM 與 IPS"), "到了選 ROM 頁")
    g.settext(g.ctl_id("Edit", 0), c.T + "\\src\\dbz.sfc")
    g.click(IDOK)
    c.check(g.wait_text(HD_PAGE_TITLE), "到了 HD 美術包頁")
    g.shot("17a-hd-pack-long-box")
    g.texts_fit("HD 美術包頁（核取方塊加長）")
    box = g.ctl_id("Button", text=HD_BOX_TEXT)
    c.check(box is not None, "HD 美術包頁：有核取方塊")
    if box is not None:
        wraps(c, g, "HD 美術包頁", box)
    g.click(IDOK)
    c.check(g.wait_text("安裝完成"), "安裝後出現完成頁")
    g.shot("17b-finish-long-box")
    g.texts_fit("完成頁（核取方塊加長）")
    link, boxes = g.panel_stack("完成頁（核取方塊加長）")
    c.check(len(link) == 1 and len(boxes) == 1, "完成頁（核取方塊加長）：一個 Ko-fi 連結、一個核取方塊", f"{link} {boxes}")
    wraps(c, g, "完成頁", 1203)
    g.drive("uncheck", ID="1203")
    g.click(IDOK)
    c.eq(g.finish(), 0, "安裝精靈結束碼")


def recorded_install(c: Ctx, d, download: bool, run: bool, what):
    """GUI install of the recorder build to `d` (empty), "download the HD pack" ticked or not on its page and "run the
    game" left ticked or not on the finish page; returns the launcher's calls."""
    c.write(c.T + "\\src\\dbz.sfc", b"ROMDATA" * 100)
    g = gui_install(c, c.build.recording_setup(c.nsi))
    g.click(IDOK)
    g.settext(1019, d)
    g.click(IDOK)
    c.check(g.wait_text("選擇遊戲 ROM 與 IPS"), f"{what}：到了選 ROM 頁")
    g.settext(g.ctl_id("Edit", 0), c.T + "\\src\\dbz.sfc")
    g.click(IDOK)
    box = hd_page(c, g, what, download=download)
    c.check(box is not None, f"{what}：HD 美術包頁有核取方塊")
    c.check(g.wait_text("安裝完成"), f"{what}：安裝完成")
    if not run:
        g.drive("uncheck", ID="1203")
    g.click(IDOK)
    c.eq(g.finish(), 0, f"{what}：精靈結束碼")
    return launcher_calls(c, int(download) + int(run))


@scenario("HD 美術包頁勾「下載」、完成頁也勾「立即開啟遊戲」：先跑 --hd-pack（等它結束）再開遊戲", gui=True)
def g12_hd_download_then_game(c: Ctx):
    calls = recorded_install(c, c.T + "\\gui-hd1", True, True, "兩個都勾")
    c.eq(calls, ["[--hd-pack]", "[]"], "啟動器被叫的順序：--hd-pack，然後遊戲")


@scenario("HD 美術包頁勾「下載」、完成頁取消「立即開啟遊戲」：只跑 --hd-pack", gui=True)
def g13_hd_download_only(c: Ctx):
    calls = recorded_install(c, c.T + "\\gui-hd2", True, False, "只勾下載")
    c.eq(calls, ["[--hd-pack]"], "啟動器只被叫一次：--hd-pack")


@scenario("HD 美術包頁不動（預設不勾）、完成頁勾「立即開啟遊戲」：只開遊戲，不跑 --hd-pack", gui=True)
def g14_hd_default_not_downloaded(c: Ctx):
    calls = recorded_install(c, c.T + "\\gui-hd3", False, True, "預設")
    c.eq(calls, ["[]"], "啟動器只被叫一次：遊戲（預設不下載美術包）")


@scenario("重裝到已有 hd-pack.zip 的安裝：HD 美術包頁照實說已經有了、不再問；完成時不跑 --hd-pack", gui=True)
def g15_hd_pack_already_there(c: Ctx):
    d = c.T + "\\inst-hd"
    c.eq(c.install(d), 0, "先靜默安裝")
    c.write(d + "\\hd-pack.zip", b"PK-not-really")
    g = gui_install(c, c.build.recording_setup(c.nsi))
    g.click(IDOK)
    g.settext(1019, d)
    g.click(IDOK)
    c.check(g.wait_text("選擇遊戲 ROM 與 IPS"), "重裝：到了選 ROM 頁")
    g.click(IDOK)
    c.check(g.wait_text(HD_PAGE_TITLE), "重裝：到了 HD 美術包頁")
    text = g.text_on_page()
    c.check("這個安裝位置已經有 HD 美術包" in text and "hd-pack.zip" in text and "不用再下載" in text,
            "HD 美術包頁：照實說已經有了", text[-300:])
    c.check(g.ctl_id("Button", text=HD_BOX_TEXT) is None, "HD 美術包頁：不再問（沒有核取方塊）")
    hd_pictures(c, g, "HD 美術包頁（已經有了）")
    g.shot("18-hd-pack-already-there")
    g.texts_fit("HD 美術包頁（已經有了）")
    g.click(IDOK)
    c.check(g.wait_text("安裝完成"), "重裝：安裝完成")
    g.drive("uncheck", ID="1203")
    g.click(IDOK)
    c.eq(g.finish(), 0, "重裝：精靈結束碼")
    c.eq(launcher_calls(c, 0), [], "啟動器沒有被叫（沒有 --hd-pack）")
    c.eq(c.w.u(d + "\\hd-pack.zip").read_bytes(), b"PK-not-really", "原本的 hd-pack.zip 沒有被動到")

@scenario("已裝 RC2（真的 RC2 安裝程式）→ 執行新版安裝程式：歡迎頁寫「偵測到已安裝的 RC2，這會原地升級為 RC3」，選位置頁帶出那個位置", gui=True)
def g16_welcome_upgrade(c: Ctx):
    d = old_install(c, "RC2")
    g = Gui(c, f"{PRODUCT} {NEXT_NAME} 安裝程式",
            c.w.popen(c.build.next_setup(c.nsi), ["/LOG=C:\\dbztest\\logs\\gui.log"]))
    c.check(g.wait(), "安裝精靈出現")
    text = g.text_on_page()
    c.check(f"偵測到已安裝的 RC2，這會原地升級為 {NEXT_NAME}，存檔與 HD 美術包都會保留。" in text,
            "歡迎頁第一段：偵測到 RC2、原地升級", text[:300])
    c.check(f"這會安裝 {NEXT_NAME}" not in text, "歡迎頁沒有全新安裝的說法")
    g.shot("19-welcome-upgrade")
    g.texts_fit("歡迎頁（升級）")
    g.panel_stack("歡迎頁（升級）")
    g.click(IDOK)
    field = [r[3].rstrip("\\") for r in g.controls() if r[0] == 1019]
    c.eq(field[:1], [d], "選位置頁帶出 RC2 的位置")
    g.shot("20-directory-upgrade")
    g.click(IDCANCEL)
    g.click(IDYES, "box")
    g.finish()
    c.eq(c.w.reg_get("DisplayVersion"), "RC2", "取消之後仍是 RC2（沒有裝任何東西）")


# ----------------------------------------------------------------------------------------------------------------------

def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--wine", type=Path, default=Path(os.environ.get(
        "DBZ_WINE", HOME / "Tools/wine/11.0_1/wine/bin/wine")))
    ap.add_argument("--prefix", type=Path, default=Path(os.environ.get(
        "DBZ_WINEPREFIX", HOME / "Tools/wine/prefix-dbz-installer")))
    ap.add_argument("--makensis", type=Path, default=Path(os.environ.get(
        "DBZ_MAKENSIS", HOME / "Tools/nsis/3.13/bin/makensis")))
    ap.add_argument("--old-setups", type=Path, default=Path(os.environ.get(
        "DBZ_OLD_SETUPS", HOME / "Tools/dbz-old-setups")),
                    help="folder of the released DBZ-SSD-zh-TW-RC1-setup.exe, -RC2-setup.exe, -RC3-setup.exe and -v1.0-setup.exe (upgrade scenarios v01-v03, upgrade_from_rc3 and upgrade_from_v1_0)")
    ap.add_argument("--ref", help="build installer/ from this git commit instead of the working tree")
    ap.add_argument("--nsi", type=Path, help="build from this copy of dbz-zhtw.nsi (mutation checks)")
    ap.add_argument("--auto-update", action="store_true", help="compile with -DAUTO_UPDATE")
    ap.add_argument("--only", action="append", default=[], help="run scenarios whose name contains TEXT")
    ap.add_argument("--keep-work", action="store_true")
    ap.add_argument("--gui", action="store_true",
                    help="run the GUI scenarios instead (windows appear on screen; screenshots need Screen Recording)")
    ap.add_argument("--shots", type=Path, help="where --gui saves its screenshots (PNG)")
    ap.add_argument("--dpi", type=int, help="run with the prefix's display scaling at this DPI (144 = 150%%, 192 = "
                                            "200%%); put back as it was afterwards")
    args = ap.parse_args(argv)

    # loaded by this script's own path: `python3 -I` leaves the script's folder off sys.path
    import importlib.util
    spec = importlib.util.spec_from_file_location("check_data_list", Path(__file__).resolve().parent / "check_data_list.py")
    check_data_list = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(check_data_list)
    print("check_data_list.py：")
    if check_data_list.main() != 0:
        print("FAIL check_data_list.py：刪除清單沒有涵蓋啟動器或遊戲寫的名字（見上）")
        return 1
    wine = Wine(args.wine, args.prefix)
    wine.guard_host_isolation()
    work = Path(tempfile.mkdtemp(prefix="dbz-installer-tests-"))
    build = Build(args.makensis, work)
    build.old_setups = args.old_setups
    nsi, notes = (args.nsi.resolve(), []) if args.nsi else build.source_tree(args.ref)
    setup_exe = build.setup(nsi, args.auto_update)
    print(f"安裝檔：{nsi}（{'-DAUTO_UPDATE' if args.auto_update else '無 AUTO_UPDATE'}）")
    for n in notes:
        print(f"注意：{n}")

    desktop_before = {p.name for p in (HOME / "Desktop").iterdir()}
    picked = [s for s in SCENARIOS if s[3] == args.gui and (not args.only or any(o in s[0] for o in args.only))]
    shots = None
    if args.gui:
        shots = args.shots or work / "shots"
        shots.mkdir(parents=True, exist_ok=True)
        # the null graphics driver draws nothing anyone can capture; make sure the Mac driver is in use
        wine.reg("delete", r"HKCU\Software\Wine\Drivers", "/v", "Graphics", "/f")
        wine.ensure_cjk_font()
    passed = failed = checks_ok = checks_bad = 0
    t0 = time.time()
    wine.start_server()
    # only read before the try: everything that changes the prefix is inside it, and the DPI is put back whatever
    # happens there (reset() included)
    dpi_before = wine.dpi_values() if args.dpi else None
    dpi_back = True
    try:
        if args.dpi:
            wine.set_dpi(args.dpi)
        # Wine reports every dot-file as hidden unless ShowDotFiles is on: on, attrib shows only the hidden flag the
        # program itself set (s23)
        wine.reg("add", r"HKCU\Software\Wine", "/v", "ShowDotFiles", "/d", "Y", "/f")
        passed, failed, checks_ok, checks_bad = run_all(picked, wine, build, setup_exe, nsi, shots)
    finally:
        try:
            Ctx(wine, build, setup_exe, nsi).reset()
        finally:
            if dpi_before is not None:
                wine.restore_dpi(dpi_before)
                now = wine.dpi_values()
                dpi_back = now == dpi_before
                print(f"DPI 設定已改回：{now}" if dpi_back else f"FAIL DPI 設定沒有改回：{now}，原本 {dpi_before}",
                      flush=True)
            wine.stop_server()
    failed += 0 if dpi_back else 1
    leaked = {p.name for p in (HOME / "Desktop").iterdir()} - desktop_before
    if leaked:
        failed += 1
        print(f"FAIL Mac 桌面多了檔案：{sorted(leaked)}")
    print(f"\n情境 {passed} 通過、{failed} 失敗；檢查項 {checks_ok} 通過、{checks_bad} 失敗；"
          f"{time.time() - t0:.0f} 秒")
    if shots:
        print(f"截圖：{shots}")
    if args.keep_work:
        print(f"工作資料夾：{work}")
    else:
        shutil.rmtree(work, ignore_errors=True)
    return 0 if failed == 0 else 1


def run_all(picked, wine, build, setup_exe, nsi, shots):
    passed = failed = checks_ok = checks_bad = 0
    for name, title, fn, _ in picked:
        ctx = Ctx(wine, build, setup_exe, nsi, shots)
        ctx.reset()
        try:
            fn(ctx)
        except Exception as e:  # a crash is a failed scenario, not a dead run
            ctx.check(False, "情境執行時發生例外", f"{type(e).__name__}: {e}")
        bad = [ch for ch in ctx.checks if not ch[0]]
        checks_ok += len(ctx.checks) - len(bad)
        checks_bad += len(bad)
        if bad or not ctx.checks:
            failed += 1
            print(f"FAIL {name}：{title}", flush=True)
            for _, what, detail in bad:
                print(f"     - {what}：{detail}".rstrip("："), flush=True)
        else:
            passed += 1
            print(f"PASS {name}：{title}（{len(ctx.checks)} 項）", flush=True)
    return passed, failed, checks_ok, checks_bad


if __name__ == "__main__":
    sys.exit(main())
