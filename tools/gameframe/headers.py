"""Read the help a tool already carries: a Python docstring, a shell or Java header comment.

Every tool documents itself (AGENTS.md rule 3). These functions read that text from the file
without running or importing it, so they work before scripts/setup.sh has made the .venv.
"""
import ast


def python_doc(path):
    """The module docstring of a Python file, or ''."""
    try:
        return ast.get_docstring(ast.parse(path.read_text(encoding='utf-8'))) or ''
    except (SyntaxError, UnicodeDecodeError):
        return ''


def comment_doc(path, mark):
    """The comment block at the top of a file whose comment lines start with `mark` (# or //).

    A shebang line is skipped. The block ends at the first line that is not a comment.
    """
    out = []
    for i, line in enumerate(path.read_text(encoding='utf-8').splitlines()):
        if i == 0 and line.startswith('#!'):
            continue
        if not line.startswith(mark):
            break
        text = line[len(mark):]
        out.append(text[1:] if text.startswith(' ') else text)
    return '\n'.join(out).strip('\n')


def has_main(path):
    """True when a Python file runs as a program (it has an `if __name__ == '__main__'` block)."""
    text = path.read_text(encoding='utf-8')
    return "__name__ == '__main__'" in text or '__name__ == "__main__"' in text


def summary(doc):
    """The first sentence of the first paragraph, on one line."""
    para = []
    for line in doc.strip().splitlines():
        if not line.strip():
            break
        if line.lstrip().startswith(('Usage:', 'usage:')) or line.startswith('  '):
            if para:
                break
            continue
        para.append(line.strip())
    text = ' '.join(para)
    cut = text.find('. ')
    text = text[:cut + 1] if cut >= 0 else text
    return text[:-1] + '.' if text.endswith(':') else text
