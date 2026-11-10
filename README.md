# semantic-facts-gleam

Parser-derived semantic facts for Gleam in Basalt.

## Capabilities

- `SEMANTIC_FACTS` / `semantic-facts@gleam/v1`.
- Requires `parse.call-sites@gleam/v1` and `parse.retrieval@gleam/v1`
  (provided by `parser-gleam`).

## Facts emitted

- `DeclareSymbol` for types, functions, constants, and data constructors.
  Qualified names use the module path derived from the file path
  (`add_helper/calendar::parse_query`, `add_helper/calendar::CalendarQuery::Today`).
- `Calls` for every parser call site, plus `IOOperation` classification for
  `io.*` (StdIO), `simplifile.*` (file read/write), and `httpc.*` (network).
- `ImportModule` and `DependsOn` for `import` statements, including aliases.

## Build

```bash
cargo build --release --target wasm32-unknown-unknown
```

Artifact:
`target/wasm32-unknown-unknown/release/basalt_plugin_semantic_facts_gleam.wasm`.
