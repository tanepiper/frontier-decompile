"""The skills in .claude/skills/re-*/SKILL.md, their sections, and the method's steps.

    from skills import skills, find, section, steps
    for s in skills(): print(s.name, s.description)
    text = section(find('re-trace').body, 'pitfalls')     # one ## or ### section, or None
    diagram, rows = steps()        # the loop and its table, from re-start's "The loop"

The steps come from the re-start skill, so they change when that skill changes.
"""
import difflib
import re
from dataclasses import dataclass
from pathlib import Path

from paths import SKILLS, rel

HEADING = re.compile(r'^(#{1,6})\s+(.*)$')


@dataclass
class Skill:
    name: str
    description: str
    path: Path
    body: str         # the Markdown after the front matter

    def as_dict(self):
        return {'name': self.name, 'description': self.description, 'path': rel(self.path),
                'sections': [title for _, title, _ in headings(self.body)]}


def _parse(path):
    text = path.read_text(encoding='utf-8')
    meta, body = {}, text
    if text.startswith('---\n'):
        end = text.find('\n---', 4)
        for line in text[4:end].splitlines():
            key, _, value = line.partition(':')
            meta[key.strip()] = value.strip()
        body = text[end + 4:].lstrip('\n')
    return Skill(meta.get('name', path.parent.name), meta.get('description', ''), path, body)


def skills():
    """Every skill, by name."""
    return [_parse(p) for p in sorted(SKILLS.glob('*/SKILL.md'))]


def find(name):
    """(skill, []) for `re-trace` or `trace`; (None, close names) when nothing matches."""
    every = skills()
    for s in every:
        if name in (s.name, s.name.removeprefix('re-')):
            return s, []
    return None, difflib.get_close_matches(name, [s.name for s in every], n=3, cutoff=0.5)


def headings(body):
    """(level, title, line number) for every heading outside code blocks."""
    out, fenced = [], False
    for i, line in enumerate(body.splitlines()):
        if line.startswith('```'):
            fenced = not fenced
        m = None if fenced else HEADING.match(line)
        if m:
            out.append((len(m.group(1)), m.group(2).strip(), i))
    return out


def section(body, title):
    """The section whose heading contains `title` (any case), up to the next heading as high."""
    lines, marks = body.splitlines(), headings(body)
    for n, (level, name, start) in enumerate(marks):
        if title.lower() in name.lower():
            end = next((s for lv, _, s in marks[n + 1:] if lv <= level), len(lines))
            return '\n'.join(lines[start:end]).rstrip()
    return None


def steps():
    """The loop diagram and the step table of re-start: (diagram, [(step, question, skill)])."""
    start, _ = find('re-start')
    text = (section(start.body, 'The loop') if start else None) or ''
    diagram = re.search(r'```\n(.*?)```', text, re.S)
    rows = []
    for line in text.splitlines():
        cells = [c.strip() for c in line.strip().strip('|').split('|')]
        if len(cells) == 3 and cells[2].startswith('`'):
            rows.append((cells[0], cells[1], cells[2].strip('`')))
    return (diagram.group(1).rstrip() if diagram else ''), rows
