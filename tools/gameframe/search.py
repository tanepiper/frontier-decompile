"""Find which skills and tools mention a word: the lines that match, with the section they are in.

    from search import search
    for kind, name, where, line in search('vtable'): ...

Searches the skills' text and the tools' own help, case-insensitively, as plain text.
"""
import catalog
import skills


def search(word):
    """(kind, name, where, line) for every matching line; kind is 'skill' or 'tool'."""
    word = word.lower()
    hits = []
    for s in skills.skills():
        heading, fenced = '', False
        for line in s.body.splitlines():
            if line.startswith('```'):
                fenced = not fenced
            if not fenced and line.startswith('#'):
                heading = line.lstrip('#').strip()
            if word in line.lower():
                hits.append(('skill', s.name, heading, line.strip()))
        if word in s.description.lower() and not any(h[1] == s.name for h in hits):
            hits.append(('skill', s.name, 'description', s.description))
    for t in catalog.tools():
        for line in t.doc.splitlines():
            if word in line.lower():
                hits.append(('tool', t.name, '', line.strip()))
    return hits
