# ADR 0014: GraphQL schemas as contracts and service-connection evidence

- **Status:** accepted
- **Date:** 2026-11-18

## Context

ArchGuard reads OpenAPI contracts through an explicit import
(`archguard contracts --report … --service …`, `.archguard/contracts.json`)
and AsyncAPI documents for pattern evidence. gRPC appears only as client
calls in the service topology. GraphQL was not covered at all, although
GraphQL services usually keep their schema in the repository
(`schema.graphql`, Apollo `typeDefs = gql\`…\``) and their clients keep
operation documents next to the code. Unlike OpenAPI, the schema and its
consumers can therefore be found in the sources without an import.

No GraphQL parser is in `Cargo.lock`. The existing crates
(`graphql-parser`, `apollo-parser`, `async-graphql-parser`) would pull in a
full parser and its dependencies for what ArchGuard needs: type names,
fields, arguments with nullability and defaults, and the root fields of an
operation.

## Decision

1. **A small own reader** (`archguard-engine/src/graphql.rs`) tokenizes
   GraphQL and reads type system definitions (`type`, `interface`,
   `input`, `enum`, `union`, `scalar`, `schema`, `directive`, `extend …`)
   and executable definitions (operations, fragments). It does not
   validate. Directive uses, including federation `@key`, `@external`,
   `@link`, are recorded as written text with their schema coordinate and
   never interpreted. Subgraphs are not composed into a supergraph.
2. **Found, not imported.** Schema files (`.graphql`, `.graphqls`, `.gql`)
   and `gql`/`graphql` tagged templates in TS/JS are read from the
   analysed tree (discovery exclusions apply; test and mock paths are
   skipped). A schema belongs to the service whose build context or
   project directory is the deepest one containing the file; if none or
   several services share that scope, the provider stays unknown. All
   schema documents of one service form one contract.
3. **Links by root fields, exactly one match.** An operation links its
   service to a provider only when exactly one schema declares all its
   root fields on the root type for its kind (from `schema { … }` or the
   default `Query`/`Mutation`/`Subscription`). Root-level fragment spreads
   are followed inside the same document. Zero matches, several matches,
   unknown fragments or a document outside every service scope leave the
   operation unresolved with that reason. A service using its own schema
   is no link. Nested selections are not checked.
4. **Topology edges.** Each link also becomes `HttpCall` evidence
   (`pointer: graphql:<operation>`) in the service topology, so diagrams,
   cycles and arc42 see GraphQL connections like other source-derived
   calls. Configured HTTP endpoints whose variable name or value contains
   `graphql` are reported as `configured_endpoint` links when the target
   owns a schema; the value is searched, never output.
5. **Breaking changes against an explicit baseline.**
   `archguard contracts --graphql-baseline` writes
   `.archguard/graphql-contracts.json`. Reading compares the current
   schemas with it in the OpenAPI style (`breaking`, `review`, `info`):
   removed type, field or enum value, removed argument, new required
   argument and an argument that became non-null are breaking; changed
   field or argument types need review; a nullable argument or added
   field is info. Consumers of a change are the linked services whose
   operations select the changed root field.
6. **One report.** The result is the `graphql` part of the API contract
   report, so `archguard contracts`, MCP `api_contracts`, review and arc42
   inputs carry it. It is computed on read and never stored in
   `contracts.json`.

## Alternatives

- **An external parser crate.** Rejected for now: full spec coverage is
  not needed, and every crate adds a dependency to audit. The reader is
  replaceable behind `graphql::parse` if validation becomes necessary.
- **Explicit import like OpenAPI.** Rejected: GraphQL schemas and operation
  documents live in the repository; an import would add a step without
  adding evidence. The baseline stays explicit so that a comparison never
  starts silently.
- **Matching by field name across all types or by endpoint URL alone.**
  Rejected: root fields such as `node` are common to many subgraphs, and a
  URL names a host, not a schema. Only a unique root-field match links.
- **Comparing with a Git merge base.** Not done, as for OpenAPI; the
  saved baseline is the comparison base.

## Consequences

- GraphQL schemas, links and unresolved operations appear without
  configuration. Repositories without GraphQL documents show no `graphql`
  section.
- A link is static evidence; it does not prove a request ran. Generated
  clients, persisted queries and operations built at runtime are not seen.
- Federation directives are visible but have no effect on ownership or
  matching; an entity extended in several subgraphs is a type in each
  subgraph's contract.
- The live server fingerprint (`api_contracts_fingerprint`) covers the
  OpenAPI snapshot only; edits to GraphQL files are picked up on the next
  read, not by that fingerprint.
