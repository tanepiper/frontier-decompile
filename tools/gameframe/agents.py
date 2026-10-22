"""The agents, read from their own files: what each one loads, cannot use, and hands off to.

The table in docs/working/agents.md is this module's output (`gameframe agents --table`), so
the guide always describes the agents as they are configured.
"""
import re

from paths import ROOT

AGENTS = ROOT / '.claude' / 'agents'


def _list(front, key):
    m = re.search(rf'^{key}:\n((?:  - .+\n)+)', front + '\n', re.M)
    return [line[4:].strip() for line in m.group(1).splitlines()] if m else []


def agents():
    out = []
    for p in sorted(AGENTS.glob('*.md')):
        text = p.read_text(encoding='utf-8')
        front = re.match(r'---\n(.*?)\n---\n', text, re.S).group(1)
        desc = re.search(r'^description: "?(.*?)"?$', front, re.M).group(1)
        ends = text.split('## Where it ends', 1)[1].split('\n## ', 1)[0] if '## Where it ends' in text else ''
        out.append({
            'name': p.stem,
            'summary': desc.split('. ')[0].rstrip('.') + '.',
            'skills': _list(front, 'skills'),
            'cannot_use': _list(front, 'disallowedTools'),
            'hands_off_to': sorted(set(re.findall(r'`(re-[a-z]+er)`', ends)) - {p.stem}),
        })
    return out


def table():
    rows = ['| Agent | Loads | Cannot use | Hands off to |', '|---|---|---|---|']
    for a in agents():
        rows.append(f"| `{a['name']}` | {', '.join(a['skills'])} | {', '.join(a['cannot_use']) or 'nothing'} | "
                    f"{', '.join(a['hands_off_to']) or 'the session that sent it'} |")
    return '\n'.join(rows)
