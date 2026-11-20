# Launch copy — English

Status: editorial draft. Publish after the revised site and the release it describes are available. Use the public repository URL below; add the deployed website URL when known. Do not describe a feature as newly released until the release notes confirm it.

## Blog opening

### Architecture for people and coding agents

“Why does this package depend on that one?” is a useful question for a developer and for a coding agent. Both need the same things: the relationship, the source behind it, and the limits of the analysis.

ArchGuard starts with source code and project configuration. Its native desktop app lets you explore a World View, follow cycles and inspect dependency evidence. The CLI makes architecture rules part of a build. The MCP server makes the analysis available to a coding agent.

A configuration edge and a recorded runtime call are different claims. An unresolved target is not a zero. Keeping those distinctions visible is the principle behind the project: missing evidence is not a zero.

Start with the included Service Shop example, which can be analysed without starting containers:

```sh
archguard-desktop examples/service-shop
archguard services examples/service-shop --format json
```

For a real polyglot project, follow the pinned OpenTelemetry Demo walkthrough on the website. Its evidence boundaries are part of the example, not hidden qualifications.

ArchGuard is a non-commercial open-source project under MIT OR Apache-2.0. Feedback is most useful as a reproducible repository, a source revision and a concrete architecture question.

Source and installation: https://github.com/svenhartmann/ArchGuard

## Show HN draft

Title: **Show HN: ArchGuard — explore architecture locally and query it through MCP**

I work on ArchGuard, a non-commercial open-source architecture analysis tool written in Rust. It has a native desktop app, a CLI and a local MCP server. You can follow package dependencies and cycles, inspect supported service-configuration evidence, and ask a coding agent about the same analysis.

The distinction I care about is between “measured zero” and “not measured”. Compiler coverage, unresolved calls and imported runtime data remain explicit. Simulations preview graph changes; they do not edit your source.

The repository includes reproducible examples and the website uses real desktop captures. I would welcome a concrete example where the dependency evidence is wrong or where the scope is unclear.

https://github.com/svenhartmann/ArchGuard

Editorial note: attach the public site link and a short real clip. Check Show HN submission rules on publication day; do not imply endorsements or user counts.

## r/rust adaptation

Title: **A Rust architecture analyser with a native Slint UI, CLI and MCP server**

ArchGuard is a non-commercial project for exploring code dependencies and checking architecture boundaries. The native UI and the agent interface expose the same analysis, including gaps when references or language tooling are missing. The implementation and reproducible examples are public. I am interested in feedback on explainable analysis boundaries and the CLI/MCP workflow.

https://github.com/svenhartmann/ArchGuard

Editorial note: read the community's current self-promotion rules and disclose authorship. For r/softwarearchitecture, lead with a concrete cycle and its evidence rather than implementation language. For Lobsters, use the same technical question and the site's applicable submission rules.

## Mastodon / LinkedIn short post

A dependency diagram becomes useful when you can explain an edge. ArchGuard connects a native architecture view, CLI checks and a local MCP server. Real source evidence; unknown measurements stay unknown. Non-commercial and open source.

https://github.com/svenhartmann/ArchGuard

Clip caption: **From the package landscape to the source behind a cycle. Actual ArchGuard desktop captures of Apache Commons Collections.**

Alt text: **ArchGuard World View followed by dependency graph and source evidence views for Apache Commons Collections; the clip has no audio.**
