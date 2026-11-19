# ADR 0012: Datastore tables from the services' own source

- **Status:** accepted
- **Date:** 2026-11-18

## Context

The service topology finds datastores from static configuration and reports
two services on the same datastore and database name as a *shared database*,
on the same server only as a *shared server*. Both say nothing about whether
the services touch the same data. Two services with their own tables in one
database are a deployment choice; two services writing the same table are a
coupling an architect wants to see. The README said "Tables are not
analyzed".

## Decision

1. `service_tables` (engine) reads the source below each service's build
   context, or its project directory when there is none. Files below another
   service nested in that directory, test paths and excluded directories
   (`target`, `node_modules`, `build`, …) do not count. It recognizes:
   - C# EF Core: `DbSet<T> Name` (table `Name` by convention), `[Table("x")]`,
     `.ToTable("x")` (both override the `DbSet` name of their entity) and
     migration `CreateTable(name: "x")`.
   - Java/Kotlin JPA and TypeScript TypeORM: `@Entity` classes, named by
     `@Table(name = "x")`, by the entity's own name argument, or by the class
     name in snake_case (Spring Boot's default naming). Spring Data
     repositories add nothing beyond their entities.
   - Python SQLAlchemy `__tablename__ = "x"`.
   - `CREATE TABLE` and `ALTER TABLE` in `.sql` files, which covers Flyway and
     Liquibase SQL migrations.
2. Names are lowercased without schema, quotes or brackets. A name that is not
   a literal (constant, variable, `${placeholder}`) is never guessed: it adds
   a diagnostic "Table name is not a literal" and marks the report incomplete.
3. `datastores.tables` lists each service's tables with file and line.
   A usage of a datastore with tables (not MongoDB, Redis, Cassandra,
   Elasticsearch, Neo4j, CouchDB) gets the service's tables in
   `usage.tables` only when it is the service's only such usage; with several,
   a diagnostic says the tables are not attributed.
4. A shared database whose services name a common table becomes scope `table`
   with `tables` listing the common ones. A shared database where every
   service has known tables, no gap, and no table repeats gets
   `disjoint_tables: true`. Otherwise the finding stays as before.
5. `archguard services` (text and JSON), the MCP `service_topology` tool
   (`shared_datastore_counts.table`, `totals.datastore_tables`) and the C4
   deployment and communication diagrams (datastore details, shown in the
   viewer and the desktop app) expose the result.

## Alternatives

- **Parse SQL queries and ORM calls** to find tables a service reads or
  writes. More precise about use, but query strings are often built at run
  time; the cost is a SQL parser per dialect. Rejected for now.
- **Use full language parsers** (Roslyn, javaparser) for entity mappings.
  More exact for unusual layouts, but the patterns above are literal and a
  text scan finds them without new dependencies.
- **Guess names from configuration** (`spring.jpa.properties`, environment
  variables). Rejected: a guessed table could create a false shared-table
  finding.

## Consequences

- A shared table is evidence from mappings and migrations, not proof of
  runtime access; a mapped table may be unused.
- Schemas are dropped, so `a.orders` and `b.orders` count as one table.
- Convention-based names (DbSet property, entity class) follow the default
  naming strategy; a custom naming strategy can produce different names.
- Liquibase XML/YAML changelogs, Django models and Go are not read; their
  services show no tables, and the finding then makes no table claim.

## History

- **2026-11-19:** Web viewer removed (user decision). The tables are
  shown by the desktop app and returned by `archguard services --format
  json` and MCP `service_topology`.
