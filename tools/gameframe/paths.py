"""Where things are: the command's name, the repository root, the skills, the Python to run tools.

The same layout holds in GameFrame and in a project made by scripts/new-project.sh, so the
root is found from this file's place (tools/gameframe/paths.py) and nothing else.
"""
import os
import sys
from pathlib import Path

NAME = 'gameframe'          # the command's name: change it here and in the launcher's file name
ROOT = Path(__file__).resolve().parents[2]
SKILLS = ROOT / '.claude' / 'skills'
TOOLS = ROOT / 'tools'
SCRIPTS = ROOT / 'scripts'


def python():
    """The Python that runs the tools: $PYTHON, else GameFrame's .venv, else this one.

    Returns (path, note). The note says why when the .venv is missing, or is ''.
    """
    if os.environ.get('PYTHON'):
        return os.environ['PYTHON'], ''
    venv = ROOT / '.venv' / 'bin' / 'python'
    if venv.exists():
        return str(venv), ''
    return sys.executable, (f'{NAME}: no .venv here; using {sys.executable}. '
                            'Tools that need unicorn, pefile or capstone will fail: '
                            'run scripts/setup.sh --install.')


def rel(path):
    """A path relative to the root, for printing."""
    try:
        return str(Path(path).resolve().relative_to(ROOT))
    except ValueError:
        return str(path)
