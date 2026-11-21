# Domain Docs

How the engineering skills find this repo's domain knowledge before exploring the codebase.

## Before exploring, ask the archguard MCP server

This repo keeps no glossary file. Domain terms, structure and rules come from the archguard MCP
server, which reads them from the code and `archguard.toml` instead of a hand-maintained list:

- **`domain_model`**: entities, aggregates, relations, state machines, use cases and events, with
  the source file of each. Use its names when you name a domain concept.
- **`architecture_context`**: declared layers, rules, quality goals and trade-offs.
- **`architecture_atlas`** and **`semantic_model`**: packages, classes, methods and calls.
- **`ui_flow`**: screens, endpoints and who may reach them.
- **`development_guide`**: every route through the tools.

Then read the ADRs in **`docs/adr/`** that touch the area you are about to work in.

## Use the model's vocabulary

When your output names a domain concept (an issue title, a refactor proposal, a hypothesis, a
test name), use the name `domain_model` or the ADRs use. Don't drift to synonyms.

If a concept you need is in neither, that is a signal: either you are inventing language the
project doesn't use (reconsider), or the model has a gap (note it, with the file that defines the
concept).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_
