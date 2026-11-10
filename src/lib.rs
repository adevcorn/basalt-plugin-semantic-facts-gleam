//! Semantic-facts-gleam — parser-derived semantic facts for Gleam.
//!
//! Provides `semantic-facts@gleam/v1` by consuming:
//! - `parse.call-sites@gleam/v1`
//! - `parse.retrieval@gleam/v1`
//!
//! Request format from core (capability handle):
//! `[src_len u32][src][path_len u32][path][root_len u32][root]`

#![no_std]
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
use basalt_plugin_sdk::facts::serialize_facts;
use basalt_plugin_sdk::facts::{IOKind, SemanticFact, SymbolKind};
use basalt_plugin_sdk::prelude::*;

basalt_plugin_meta! {
    name:              "semantic-facts-gleam",
    version:           env!("CARGO_PKG_VERSION"),
    hook_flags:        CAP_CAPABILITY_HANDLE | CAP_SEMANTIC_FACTS,
    provides:          "semantic-facts@gleam/v1",
    requires:          "parse.call-sites@gleam/v1\nparse.retrieval@gleam/v1",
    optional_requires: "",
    file_globs:        "**/*.gleam",
    activates_on:      "",
    activation_events: "",
}

// ── Capability handle ───────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn basalt_capability_handle(
    cap_ptr: *const u8,
    cap_len: usize,
    req_ptr: *const u8,
    req_len: usize,
) -> i64 {
    let _ = (cap_ptr, cap_len);
    if req_ptr.is_null() || req_len < 4 {
        return pack_empty();
    }
    let request = unsafe { core::slice::from_raw_parts(req_ptr, req_len) };
    let Some((src, path)) = parse_request(request) else {
        return pack_error(-1002);
    };
    if src.is_empty() {
        return pack_empty();
    }

    match derive_semantic_facts(src, path) {
        Ok(facts) => pack_success(facts),
        Err(code) => pack_error(code),
    }
}

/// Decode `[src_len][src][path_len][path][root_len][root]`.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn parse_request(request: &[u8]) -> Option<(&[u8], &str)> {
    if request.len() < 4 {
        return None;
    }
    let src_len = u32::from_le_bytes([request[0], request[1], request[2], request[3]]) as usize;
    if request.len() < 4 + src_len {
        return None;
    }
    let src = &request[4..4 + src_len];

    let mut pos = 4 + src_len;
    let mut path = "";
    if request.len() >= pos + 4 {
        let len = u32::from_le_bytes([
            request[pos],
            request[pos + 1],
            request[pos + 2],
            request[pos + 3],
        ]) as usize;
        pos += 4;
        if request.len() >= pos + len {
            path = core::str::from_utf8(&request[pos..pos + len]).unwrap_or("");
        }
    }
    Some((src, path))
}

// ── Fact derivation ─────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn derive_semantic_facts(src: &[u8], path: &str) -> Result<Vec<u8>, i64> {
    let call_sites_raw = invoke_parse_call_sites(src).map_err(|_| -1003)?;
    let retrieval_raw = invoke_parse_retrieval(src).map_err(|_| -1003)?;

    let call_sites = decode_call_sites(&call_sites_raw);
    let retrieval = decode_retrieval(&retrieval_raw);
    let module = module_name(path);

    let mut facts: Vec<SemanticFact> = Vec::new();
    derive_declarations(&retrieval, &module, &mut facts);
    derive_call_edges(&call_sites, &mut facts);
    derive_imports(src, &mut facts);

    Ok(serialize_facts(&facts))
}

#[cfg(target_arch = "wasm32")]
fn invoke_parse_call_sites(src: &[u8]) -> Result<Vec<u8>, i64> {
    let mut request = Vec::with_capacity(4 + src.len() + 4);
    request.extend_from_slice(&(src.len() as u32).to_le_bytes());
    request.extend_from_slice(src);
    request.extend_from_slice(&16384u32.to_le_bytes());
    invoke_capability("parse.call-sites@gleam/v1", &request).map_err(|_| -1003)
}

#[cfg(target_arch = "wasm32")]
fn invoke_parse_retrieval(src: &[u8]) -> Result<Vec<u8>, i64> {
    let mut request = Vec::with_capacity(4 + src.len() + 4);
    request.extend_from_slice(&(src.len() as u32).to_le_bytes());
    request.extend_from_slice(src);
    request.extend_from_slice(&8192u32.to_le_bytes());
    invoke_capability("parse.retrieval@gleam/v1", &request).map_err(|_| -1003)
}

// ── Parser response decoding ────────────────────────────────────────────────

fn decode_call_sites(data: &[u8]) -> Vec<(u32, &str)> {
    if data.len() < 4 {
        return Vec::new();
    }
    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let mut sites = Vec::with_capacity(count);
    let mut pos = 4;
    for _ in 0..count {
        if pos + 68 > data.len() {
            break;
        }
        let offset = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        let name_bytes = &data[pos + 4..pos + 68];
        let nul = name_bytes.iter().position(|&b| b == 0).unwrap_or(64);
        if let Ok(name) = core::str::from_utf8(&name_bytes[..nul]) {
            if !name.is_empty() {
                sites.push((offset, name));
            }
        }
        pos += 68;
    }
    sites
}

/// `(offset, length, label, kind)` from a 104-byte v1 retrieval record.
fn decode_retrieval(data: &[u8]) -> Vec<(u32, u32, &str, u8)> {
    if data.len() < 4 {
        return Vec::new();
    }
    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let mut chunks = Vec::with_capacity(count);
    let mut pos = 4;
    for _ in 0..count {
        if pos + 104 > data.len() {
            break;
        }
        let offset = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        let length =
            u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);
        let label_bytes = &data[pos + 8..pos + 103];
        let kind = data[pos + 103];
        let nul = label_bytes.iter().position(|&b| b == 0).unwrap_or(95);
        if let Ok(label) = core::str::from_utf8(&label_bytes[..nul]) {
            let label = label.trim();
            if !label.is_empty() && length > 0 {
                chunks.push((offset, length, label, kind));
            }
        }
        pos += 104;
    }
    chunks
}

/// Derive the Gleam module path from the workspace-relative file path.
fn module_name(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let stripped = ["src/", "test/", "./"]
        .iter()
        .find_map(|prefix| normalized.strip_prefix(prefix))
        .unwrap_or(&normalized);
    let module = stripped.strip_suffix(".gleam").unwrap_or(stripped);
    module.to_string()
}

fn label_name<'a>(label: &'a str, kind: &str) -> &'a str {
    label
        .strip_prefix(kind)
        .map(str::trim)
        .unwrap_or(label)
        .trim()
}

fn derive_declarations(
    retrieval: &[(u32, u32, &str, u8)],
    module: &str,
    facts: &mut Vec<SemanticFact>,
) {
    let type_scopes: Vec<(u32, u32, &str)> = retrieval
        .iter()
        .filter(|(_, _, _, kind)| *kind == 2)
        .map(|(offset, length, label, _)| (*offset, *length, label_name(label, "type")))
        .collect();

    for &(offset, length, label, kind) in retrieval {
        match kind {
            2 => {
                let name = label_name(label, "type");
                if name.is_empty() {
                    continue;
                }
                facts.push(SemanticFact::DeclareSymbol {
                    kind: SymbolKind::Type,
                    offset,
                    length,
                    name: name.to_string(),
                    qualified_name: qualified(module, name),
                });
            }
            3 => {
                let name = label_name(label, "function");
                if name.is_empty() {
                    continue;
                }
                facts.push(SemanticFact::DeclareSymbol {
                    kind: SymbolKind::Function,
                    offset,
                    length,
                    name: name.to_string(),
                    qualified_name: qualified(module, name),
                });
            }
            5 => {
                let name = label_name(label, "property");
                if name.is_empty() {
                    continue;
                }
                facts.push(SemanticFact::DeclareSymbol {
                    kind: SymbolKind::Constant,
                    offset,
                    length,
                    name: name.to_string(),
                    qualified_name: qualified(module, name),
                });
            }
            6 => {
                let name = label_name(label, "enum_case");
                if name.is_empty() {
                    continue;
                }
                // Data constructors live inside a type definition; find the
                // innermost enclosing type to build `module::Type::Case`.
                let parent = type_scopes
                    .iter()
                    .filter(|(start, len, _)| *start <= offset && offset < start + len)
                    .min_by_key(|(_, len, _)| *len)
                    .map(|(_, _, name)| *name);
                let qualified_name = match parent {
                    Some(parent) => Some(format!("{module}::{parent}::{name}")),
                    None => qualified(module, name),
                };
                facts.push(SemanticFact::DeclareSymbol {
                    kind: SymbolKind::EnumMember,
                    offset,
                    length,
                    name: name.to_string(),
                    qualified_name,
                });
            }
            _ => {}
        }
    }
}

fn qualified(module: &str, name: &str) -> Option<String> {
    if module.is_empty() {
        None
    } else {
        Some(format!("{module}::{name}"))
    }
}

fn derive_call_edges(call_sites: &[(u32, &str)], facts: &mut Vec<SemanticFact>) {
    for &(offset, callee) in call_sites {
        let length = callee.len() as u32;
        facts.push(SemanticFact::Calls {
            caller_offset: offset,
            caller_length: length,
            callee: callee.to_string(),
        });
        if let Some(io_kind) = classify_io(callee) {
            facts.push(SemanticFact::IOOperation {
                io_kind,
                offset,
                length,
                descriptor: Some(callee.to_string()),
            });
        }
    }
}

/// Gleam-specific I/O classification over callee names.
fn classify_io(callee: &str) -> Option<IOKind> {
    let lower = callee.to_ascii_lowercase();
    let tail = lower.rsplit('.').next().unwrap_or(&lower);

    if lower.contains("httpc") || lower.contains("http.") || lower.contains("http_client") {
        Some(IOKind::Network)
    } else if lower.contains("simplifile") || lower.starts_with("file.") {
        if tail.contains("read") || tail.contains("open") {
            Some(IOKind::FileRead)
        } else if tail.contains("write") || tail.contains("append") || tail.contains("delete") {
            Some(IOKind::FileWrite)
        } else {
            None
        }
    } else if lower.starts_with("io.") || matches!(tail, "println" | "print" | "debug") {
        Some(IOKind::StdIO)
    } else {
        None
    }
}

/// Extract `import module[/path] [as alias]` and `import module.{items}`.
fn derive_imports(src: &[u8], facts: &mut Vec<SemanticFact>) {
    let text = String::from_utf8_lossy(src);
    let base_ptr = text.as_ptr() as usize;
    for line in text.lines() {
        let line_offset = (line.as_ptr() as usize).saturating_sub(base_ptr) as u32;
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("import ") {
            let rest = rest.trim();
            let module = rest
                .split(|c: char| c.is_whitespace() || c == '{')
                .next()
                .unwrap_or("")
                .trim_end_matches('.')
                .trim();
            if !module.is_empty() {
                let alias = rest
                    .split_once(" as ")
                    .map(|(_, alias)| alias.trim().to_string())
                    .filter(|alias| !alias.is_empty());
                let offset = line_offset
                    + (line.len() - line.trim_start().len()) as u32
                    + "import ".len() as u32;
                facts.push(SemanticFact::ImportModule {
                    offset,
                    length: module.len() as u32,
                    module_path: module.to_string(),
                    alias,
                });
                facts.push(SemanticFact::DependsOn {
                    offset,
                    length: module.len() as u32,
                    dependency: module.to_string(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn fixture() -> Vec<(u32, u32, &'static str, u8)> {
        vec![
            (0, 300, "type CalendarQuery", 2),
            (40, 60, "enum_case Today", 6),
            (120, 150, "function parse_query", 3),
            (280, 20, "property default_query", 5),
            (310, 40, "module gleam/list", 1),
        ]
    }

    #[test]
    fn declarations_carry_module_qualified_names() {
        let mut facts = Vec::new();
        derive_declarations(&fixture(), "add_helper/calendar", &mut facts);

        let qualified_name = |name: &str| {
            facts.iter().find_map(|fact| match fact {
                SemanticFact::DeclareSymbol {
                    name: n,
                    qualified_name,
                    ..
                } if n == name => qualified_name.clone(),
                _ => None,
            })
        };

        assert_eq!(
            qualified_name("parse_query"),
            Some("add_helper/calendar::parse_query".to_string())
        );
        assert_eq!(
            qualified_name("CalendarQuery"),
            Some("add_helper/calendar::CalendarQuery".to_string())
        );
        assert_eq!(
            qualified_name("Today"),
            Some("add_helper/calendar::CalendarQuery::Today".to_string())
        );
        assert_eq!(
            qualified_name("default_query"),
            Some("add_helper/calendar::default_query".to_string())
        );
    }

    #[test]
    fn module_chunks_do_not_become_declarations() {
        let mut facts = Vec::new();
        derive_declarations(&fixture(), "app", &mut facts);
        assert!(!facts.iter().any(|fact| matches!(
            fact,
            SemanticFact::DeclareSymbol { name, .. } if name == "gleam/list"
        )));
    }

    #[test]
    fn imports_emit_module_and_dependency_facts() {
        let src = b"import gleam/list\nimport add_helper/models.{type CalendarEvent}\nimport gleam/io as console\n";
        let mut facts = Vec::new();
        derive_imports(src, &mut facts);

        let modules: Vec<&str> = facts
            .iter()
            .filter_map(|fact| match fact {
                SemanticFact::ImportModule { module_path, .. } => Some(module_path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(modules, vec!["gleam/list", "add_helper/models", "gleam/io"]);

        assert!(facts.iter().any(|fact| matches!(
            fact,
            SemanticFact::ImportModule { module_path, alias: Some(alias), .. }
                if module_path == "gleam/io" && alias == "console"
        )));
        assert!(facts.iter().any(|fact| matches!(
            fact,
            SemanticFact::DependsOn { dependency, .. } if dependency == "add_helper/models"
        )));
    }

    #[test]
    fn call_edges_and_io_are_classified() {
        let sites = vec![
            (0u32, "list.map"),
            (10u32, "io.println"),
            (20u32, "simplifile.read"),
            (30u32, "simplifile.write"),
            (40u32, "httpc.send"),
        ];
        let mut facts = Vec::new();
        derive_call_edges(&sites, &mut facts);
        let kinds: Vec<IOKind> = facts
            .iter()
            .filter_map(|fact| match fact {
                SemanticFact::IOOperation { io_kind, .. } => Some(*io_kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                IOKind::StdIO,
                IOKind::FileRead,
                IOKind::FileWrite,
                IOKind::Network
            ]
        );
        assert_eq!(
            facts
                .iter()
                .filter(|f| matches!(f, SemanticFact::Calls { .. }))
                .count(),
            5
        );
    }

    #[test]
    fn module_name_strips_source_prefixes() {
        assert_eq!(
            module_name("src/add_helper/calendar.gleam"),
            "add_helper/calendar"
        );
        assert_eq!(module_name("test/add_helper_test.gleam"), "add_helper_test");
        assert_eq!(
            module_name("src\\add_helper\\models.gleam"),
            "add_helper/models"
        );
    }

    #[test]
    fn request_decoder_reads_source_and_path() {
        let src = b"import gleam/io";
        let path = b"src/app.gleam";
        let mut request = Vec::new();
        request.extend_from_slice(&(src.len() as u32).to_le_bytes());
        request.extend_from_slice(src);
        request.extend_from_slice(&(path.len() as u32).to_le_bytes());
        request.extend_from_slice(path);
        let (decoded_src, decoded_path) = parse_request(&request).unwrap();
        assert_eq!(decoded_src, src);
        assert_eq!(decoded_path, "src/app.gleam");
    }
}
