---
name: re-rea
description: When and how to use REA (Reverse Engineer Anything, the rea-agents MCP server) alongside GameFrame - quick evidence-backed inspection of binaries, archives, Android/iOS/.NET/Electron/browser targets, focused decompilation and call graphs through Ghidra, and runtime observation. Use for targets the text export does not cover, for a fast first answer about one function, or when a claim needs a citable Evidence record.
---

# Investigate with REA

REA (https://github.com/morluto/rea) is an MCP server that gives the agent reverse-engineering
tools and records every result as Evidence with an ID, coverage and limits. Its own skill,
`reverse-engineer-anything`, documents each tool in detail: install it with REA's setup
(`re-setup`) and read it before a large REA investigation. This skill says how REA fits into
the playbook's loop.

## REA or the text export?

| Use REA (`mcp__rea__*`) when | Use GameFrame's Ghidra export (`re-ghidra`) when |
|---|---|
| The target is not a plain native binary: an APK/IPA, a .NET assembly, an Electron app or ASAR, a browser page, firmware, an archive | You need to search the whole program as text, many times (grep is instant) |
| You want a quick, cited answer about one function: `open_binary` then `analyze_function`, `procedure_callers`, `xrefs` | You trace a system across hundreds of functions over several sessions |
| You need a call path between two functions (`build_call_path`, `trace_call_path`) | You need `decomp_query.py` with your labels applied |
| You need runtime evidence from a running browser or Electron app (`observe_web_session`, `capture_browser_scenario`) | You emulate functions (`re-emulate`): REA does not replace the Unicorn harness |
| You compare two versions of an application (`compare_application_versions`, `find_changed_behavior`) | |
| A finding must carry a citable Evidence ID in a report | |

They agree because both read the same binary through Ghidra; addresses match. Use both: REA for
targeted questions and evidence, the export for broad search.

## The workflow

1. **Route by target** (from REA's skill): a native EXE/DLL → `open_binary` with the path;
   an archive or package → `open_binary` then `inspect_artifact`; .NET → `inspect_managed_artifact`;
   Android → `inspect_android_package`; extracted JavaScript or ASAR →
   `analyze_javascript_application`; Windows resources and icons → `inspect_pe_resources`.
2. **Start with a summary** (`binary_overview` when available), then ask focused questions:
   `search_strings`, `search_procedures`, `analyze_function`, `procedure_callers`,
   `procedure_callees`, `xrefs`, `procedure_pseudo_code`.
3. **Name what you learn in the session**: `set_address_name` and `set_comment` keep labels in
   REA's database; also copy each label into `labels/names.tsv` so GameFrame's tools see it.
4. **Keep the Evidence IDs** in your notes; cite them in the finding's document next to the
   address ("observed: FUN_1400be770 writes +0x44 (REA evidence ev_...)").
5. **Close** native sessions with `close_binary` when done.

## Rules that carry over

- REA's evidence labels match ours: observations, inferences and unknowns stay separate; static
  analysis never proves run-time behaviour.
- Do not repeat an identical REA call; page large results with `inspect_analysis_view` instead.
- REA's runtime tools run the target: run them on the target the user chose, with their
  agreement. A game's online service belongs to someone else; if the user points REA at one,
  mention its terms once, then follow the user's decision.
- Decompiled output from REA is still decompiled code: it stays out of tracked files.

## When REA is not connected

No `mcp__rea__*` tools in the session means the MCP server is not registered or did not start.
Follow `re-setup` section 3. Meanwhile, continue with GameFrame's own tools: Ghidra export,
`decomp_query.py`, `ghidra-run.sh` and the emulator cover native binaries completely.
