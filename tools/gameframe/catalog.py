"""GameFrame's tools and scripts, found on disk, each with the help it carries.

    from catalog import tools, find
    for t in tools(): print(t.name, t.kind, t.summary)
    t, close = find('peinfo')      # by name, file name or path; close names when not found

Kinds: `program` (a Python file with a main block), `library` (a Python module or package to
import), `script` (a shell script in scripts/), `ghidra` (a Ghidra script that
scripts/ghidra-run.sh or ghidra-export.sh runs). Only programs and scripts run directly.
"""
import difflib
from dataclasses import dataclass
from pathlib import Path

from headers import comment_doc, has_main, python_doc, summary
from paths import SCRIPTS, TOOLS, rel

SKIP = {'gameframe', 'templates', '__pycache__'}
ORDER = ['disc', 'pe', 'dos', 'triage', 'emu', 'formats', 'verify', 'ghidra']     # the method's order


@dataclass
class Tool:
    name: str        # what to type: peinfo, decomp_query, ghidra-export, dos_emu
    path: Path       # the file, or the package folder
    kind: str        # program, library, script or ghidra
    group: str       # the folder: pe, dos, emu, triage, formats, verify, ghidra, scripts
    doc: str

    @property
    def summary(self):
        return summary(self.doc)

    @property
    def runs(self):
        return self.kind in ('program', 'script')

    def as_dict(self):
        return {'name': self.name, 'path': rel(self.path), 'kind': self.kind,
                'group': self.group, 'summary': self.summary, 'runs': self.runs}


def _python(path, group):
    return Tool(path.stem, path, 'program' if has_main(path) else 'library', group, python_doc(path))


def tools():
    """Every tool under tools/ and every script under scripts/, by group then name."""
    found = []
    folders = [p for p in TOOLS.iterdir() if p.is_dir() and p.name not in SKIP]
    rank = {name: i for i, name in enumerate(ORDER)}
    for folder in sorted(folders, key=lambda p: (rank.get(p.name, len(ORDER)), p.name)):
        for path in sorted(folder.iterdir()):
            if path.name in SKIP or path.name.startswith(('.', '_')):
                continue
            if path.suffix == '.py':
                found.append(_python(path, folder.name))
            elif path.suffix == '.java':
                found.append(Tool(path.stem, path, 'ghidra', folder.name, comment_doc(path, '//')))
            elif path.is_dir() and (path / '__init__.py').exists():
                main = path / '__main__.py'
                kind = 'program' if main.exists() else 'library'
                found.append(Tool(path.name, path, kind, folder.name, python_doc(path / '__init__.py')))
    for path in sorted(SCRIPTS.glob('*.sh')):
        kind = 'library' if path.stem == 'lib' else 'script'
        found.append(Tool(path.stem, path, kind, 'scripts', comment_doc(path, '#')))
    return found


def _keys(t):
    r = rel(t.path)
    return {t.name, t.name.replace('_', '-'), t.name.replace('-', '_'), t.path.name, r,
            str(Path(r).with_suffix(''))}


def find(name):
    """(tool, []) for a name, file name or path; (None, close names) when nothing matches."""
    every = tools()
    for t in every:
        if name in _keys(t) or name.rstrip('/') in _keys(t):
            return t, []
    return None, difflib.get_close_matches(name, [t.name for t in every], n=3, cutoff=0.5)


def command(t, python, args):
    """The command line that runs a program or a script with its arguments."""
    if t.kind == 'script':
        return ['bash', str(t.path), *args]
    return [python, str(t.path), *args]
