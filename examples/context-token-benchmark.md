# Context tokens with and without MCP

[English](context-token-benchmark.md) | [Deutsch](context-token-benchmark_DE.md)

Measurements on 2026-10-28 show **no general token saving** over targeted `rg`
searches and file reads for these three examples. MCP responses include additional
structured evidence and metadata; the two methods do not provide equal coverage.

## Results

`tiktoken 0.14.0`, `o200k_base` encoding. Counts include actual retrieval
arguments/commands and response texts across all follow-up pages. Both methods
include the same complete target-file read with line numbers. These figures do
not yet include tool schemas.

| Target file / project | Targeted search | MCP, 4 KiB | MCP, 16 KiB |
| --- | ---: | ---: | ---: |
| `src/a/alpha.ts` / TS cycle | 613 | 909 | 948 |
| `AbstractBagDecorator.java` / Commons Collections | 2,721 | 4,191 | 3,321 |
| `ExtraRuleResolver.cs` / Jellyfin | 2,554 | 2,595 | 2,339 |
| **Total** | **5,888** | **7,695** | **6,608** |
| Increase over search | — | **30.7%** | **12.2%** |

All MCP pages are complete, omit nothing and are revision-bound within each
variant. Both variants contain the same items. The 4-KiB variant needs 1/4/2
calls; the 16-KiB variant needs one per example. The larger variant passes the
standard run's revision on its first call to verify the same data state. Those
extra argument tokens are counted, making it slightly larger for the small TS
example.

Providing compact tool descriptions once adds **549 text tokens** for
`change_context` or **4,847** for the actual 26-tool catalog. Across the three
16-KiB retrievals that totals 7,157 or 11,455 tokens, versus 5,888 for search.
This separately accounts for schema context supplied once; it makes no
assumption about a particular client.

With `cl100k_base`, the conclusion remains: search 5,758; 4-KiB MCP 7,505 (30.3%
more); 16-KiB MCP 6,428 (11.6% more).

## Different evidence

Each task examined direct callers and dependencies of a specified file,
evidenced cycles and configuration. Search plans were written independently of
MCP results and do not read generated graphs. The comparison did not include
making a code change and taking it through successful tests.

- TS: both methods establish two incoming dependencies, one outgoing dependency and `alpha ↔ beta`. MCP additionally reports effective default gates.
- Commons: search finds three subclasses and three imports. MCP additionally identifies `CollectionSortedBag.java` as a caller through an inherited method call. Search does not prove the absence of cycles.
- Jellyfin: both establish two callers and the `ExtraRuleResolver ↔ VideoResolver` file cycle. Search locates seven explicit local type dependencies; MCP returns only three outgoing files. An additional actual `rg` retrieval for the four missing types (`NamingOptions`, `ExtraRule`, `MediaType`, `ExtraRuleType`) measured **718 tokens**. This raises the 16-KiB variant to **3,057 tokens**, **19.7% more** than search. The table's raw 8.4% saving therefore does not establish equivalent dependency discovery.

Semantic status remains `unavailable` for TS and `partial` for Java/C#. Complete
MCP pages still do not constitute a complete semantic impact report: edge
evidence and cycle samples are bounded, and pure type references and dynamic
calls are not fully captured. Supplementary search does not establish complete
semantic equivalence either.

## What was counted

Search: the sum of individually tokenized command strings, `stdout` and `stderr`.
MCP: original compact `arguments_text` and `response_text` for every call, plus
the identical target-file read. Response JSON was not reformatted for counting.
Larger budgets are byte budgets, not token budgets. Percentages use total counts.

Excluded: shared task and shell-tool descriptions, model responses, reasoning,
RPC envelopes, client formatting and retransmitted conversation history. Schema
overhead counts JSON text, not an exact model protocol. No API call with usage
billing was made and no runtime saving was measured. Neither encoding is claimed
to be the exact tokenizer of the Codex model used here. OpenAI distinguishes local
text counting from complete API input, which includes roles and tool formatting:
[Token counting](https://developers.openai.com/api/docs/guides/token-counting).

## Reproduce

Prerequisites: the three examples in `scripts/benchmarks/context_benchmark_cases.json` are
available locally and the CLI is built with MCP support. The fixed baseline
replays recorded search commands without a shell; it is not a general search
agent. Sources, compiler state and configuration affect results.

```sh
python3 scripts/benchmarks/capture_context_baseline.py
python3 scripts/benchmarks/capture_context_benchmark.py
python3 -m venv /tmp/archguard-token-benchmark-venv
/tmp/archguard-token-benchmark-venv/bin/python -m pip install tiktoken==0.14.0
TIKTOKEN_CACHE_DIR=/tmp/archguard-token-benchmark-cache /tmp/archguard-token-benchmark-venv/bin/python scripts/benchmarks/benchmark_context_tokens.py
python3 -m unittest discover -s scripts -p 'test_*context*.py'
```

Tokenizer vocabulary downloads on first use. Source code is not sent to an LLM
API. MCP may update its normal local caches. The 13 regression tests require
neither network access nor a tokenizer.

Local measurement artifacts live under
`.archguard/reports/context-token-benchmark/`: `tokens.json` with both encodings,
original baseline/MCP transcripts, source/artifact hashes and `provenance.json`
with binary hash and Git revisions. These generated files are ignored by Git.
The separate Jellyfin follow-up is saved as `baseline/jellyfin-mcp-followup.json`
and counted when present; it is not part of the fixed baseline replay. Command:

```sh
rg -n -B 6 -g '*.cs' -g '!tests/**' -g '!fuzz/**' -g '!**/obj/**' -g '!**/bin/**' -g '!**/AssemblyInfo.cs' -g '!SharedVersion.cs' '(class|enum) (NamingOptions|ExtraRule|MediaType|ExtraRuleType)\b' .
```

Run that command in the Jellyfin checkout. For these examples, the larger budget
reduces MCP text including target-file reads by 14.1%. Further optimizations must
be checked against this targeted-search baseline and evidence coverage; a small
first page alone does not demonstrate savings.

## Targeted graph database query

A follow-up experiment used the same Commons Collections example: create a
separate mirror with `db sync` (390 nodes, 1,121 edges), then query only direct
neighbors through the existing MCP `graph_query` tool. The projection returns
`direction`, `file` and `refs`. All seven relationships and reference counts match
the direct neighbors in the saved `change_context`; the response is not truncated.

| Retrieval, excluding target-file read and schema | Text tokens, o200k_base |
| --- | ---: |
| Targeted DB query: arguments | 156 |
| Targeted DB query: response | 156 |
| **Targeted DB query total** | **312** |
| Complete `change_context`, 16 KiB | 2,324 |

The `graph_query` schema would add 336 text tokens separately. This shows a
smaller answer to a narrower question, **not an equivalent replacement for the
complete context package**: the DB response lacks source-line evidence, rules,
revision and semantic coverage. Here `file` is a logical graph name, not
necessarily a physical path. Sync and schema knowledge are preparation, excluded
from query-token counts. The database was synchronized immediately before this
experiment.

The current DB is a separate mirror; `change_context` does not use it. It stores
nodes, dependencies, findings and snapshot totals, but no methods or source
evidence. `graph_query` does not check freshness. Sync also combines a full graph
with a module report: metrics and cycle flags stored for file nodes must therefore
not be treated as reliable file-level analysis. This experiment uses only
`depends_on` and reference counts.

A useful extension would be a small revision-bound overview with targeted
retrieval of selected edge evidence. The DB mirror currently lacks source
mapping, evidence, coverage status and an atomically published data revision.
The existing `edge_evidence` operates on the module view and cannot directly add
this file evidence. Changing storage does not produce pure type dependencies.
Token reductions come from selection and response projection; the same mechanism
is possible on the in-memory graph.

The original query, response, counter and DB hash are saved locally at
`.archguard/reports/context-token-benchmark/graph-db/commons-neighbors.json`;
the sync run is in `commons-sync.json`. The isolated database lives in the Commons
checkout at `.archguard/reports/context-token-benchmark/graph.db`.
