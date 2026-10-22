"""gameframe: help and discovery for GameFrame, for people and agents.

Usage: gameframe COMMAND [ARGS]       (or: python3 tools/gameframe COMMAND [ARGS])

Commands:
  help [COMMAND]        this text, or one command's options
  steps                 the method's loop, each step with its question and its skill
  skills                every skill with its one-line description
  skill NAME            print a skill (re-trace, or just trace)
      --section TITLE   only the section whose heading contains TITLE
      --sections        only the headings
  tools                 every tool and script, grouped by folder, with its one-line summary
  tool NAME             the tool's own help (its docstring or header comment), unchanged
  run NAME [ARGS]       run a tool or script: Python tools with GameFrame's .venv
  find WORD             which skills and tools mention WORD, line by line
      --max N           lines shown per skill or tool (default 3)
  doctor [ARGS]         check the toolchain (scripts/setup.sh; ARGS such as --install pass on)

steps, skills, tools, tool and find take --json, for agents and scripts.

Examples:
  gameframe steps
  gameframe skill re-emulate --section pitfalls
  gameframe tool peinfo
  gameframe run peinfo target/game.exe --imports
  gameframe find vtable

Inputs: the skills (.claude/skills/re-*/SKILL.md), the tools (tools/, scripts/) and their own
help, read from the files. The same command works in GameFrame and in a project made by
scripts/new-project.sh. Standard library only; it needs no .venv itself.

It does not change any file, install anything, or keep its own copy of the skills or the help:
`run` and `doctor` start the tool or script, which does its own work.
"""
import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import commands  # noqa: E402
from paths import NAME  # noqa: E402

PASS_ON = {'run', 'doctor'}      # commands whose arguments belong to the tool they start


def parser():
    p = argparse.ArgumentParser(prog=NAME, add_help=False)
    sub = p.add_subparsers(dest='command')
    sub.add_parser('help', help='this text, or one command\'s options').add_argument('topic', nargs='?')
    s = sub.add_parser('steps', help='the method\'s loop')
    s.add_argument('--json', action='store_true')
    s = sub.add_parser('skills', help='every skill')
    s.add_argument('--json', action='store_true')
    s = sub.add_parser('skill', help='print a skill')
    s.add_argument('name', help='re-trace, or just trace')
    s.add_argument('--section', metavar='TITLE', help='only the section whose heading contains TITLE')
    s.add_argument('--sections', action='store_true', help='only the headings')
    s = sub.add_parser('tools', help='every tool and script')
    s.add_argument('--json', action='store_true')
    s = sub.add_parser('tool', help='one tool\'s own help')
    s.add_argument('name', help='peinfo, decomp_query, ghidra-export or a path')
    s.add_argument('--json', action='store_true')
    s = sub.add_parser('find', help='which skills and tools mention a word')
    s.add_argument('word', help='plain text, any case')
    s.add_argument('--max', type=int, default=3, metavar='N', help='lines shown per skill or tool (default 3)')
    s.add_argument('--json', action='store_true')
    return p, sub


def main(argv):
    if not argv or argv[0] in ('-h', '--help'):
        print(__doc__.rstrip())
        return 0
    if argv[0] == 'run':
        if len(argv) < 2:
            print(f'usage: {NAME} run NAME [ARGS]', file=sys.stderr)
            return 2
        return commands.run(argv[1], argv[2:])
    if argv[0] == 'doctor':
        return commands.doctor(argv[1:])
    p, sub = parser()
    args = p.parse_args(argv)
    if args.command == 'help':
        if args.topic in sub.choices:
            sub.choices[args.topic].print_help()
        elif args.topic in PASS_ON:
            print(f'{NAME} {args.topic}: see `{NAME} help`; its arguments go to the tool.')
        else:
            print(__doc__.rstrip())
        return 0
    return {'steps': commands.steps, 'skills': commands.list_skills, 'skill': commands.skill,
            'tools': commands.list_tools, 'tool': commands.tool, 'find': commands.find,
            }[args.command](args)


if __name__ == '__main__':
    try:
        sys.exit(main(sys.argv[1:]))
    except BrokenPipeError:            # `gameframe tools | head`
        sys.exit(0)
