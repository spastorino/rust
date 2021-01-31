//! This lint is used to collect metadata about clippy lints. This metadata is exported as a json
//! file and then used to generate the [clippy lint list](https://rust-lang.github.io/rust-clippy/master/index.html)
//!
//! This module and therefor the entire lint is guarded by a feature flag called
//! `internal_metadata_lint`
//!
//! The metadata currently contains:
//! - [ ] TODO The lint declaration line for [#1303](https://github.com/rust-lang/rust-clippy/issues/1303)
//!   and [#6492](https://github.com/rust-lang/rust-clippy/issues/6492)
//! - [ ] TODO The Applicability for each lint for [#4310](https://github.com/rust-lang/rust-clippy/issues/4310)

// # Applicability
// - TODO xFrednet 2021-01-17: Find all methods that take and modify applicability or predefine
//   them?
// - TODO xFrednet 2021-01-17: Find lint emit and collect applicability
// # NITs
// - TODO xFrednet 2021-02-13: Collect depreciations and maybe renames

use if_chain::if_chain;
use rustc_hir::{ExprKind, Item, ItemKind, Mutability};
use rustc_lint::{CheckLintNameResult, LateContext, LateLintPass, LintContext, LintId};
use rustc_session::{declare_tool_lint, impl_lint_pass};
use rustc_span::{sym, Loc, Span};
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::prelude::*;

use crate::utils::internal_lints::is_lint_ref_type;
use crate::utils::span_lint;

const OUTPUT_FILE: &str = "metadata_collection.json";
const BLACK_LISTED_LINTS: [&str; 2] = ["lint_author", "deep_code_inspection"];

declare_clippy_lint! {
    /// **What it does:** Collects metadata about clippy lints for the website.
    ///
    /// This lint will be used to report problems of syntax parsing. You should hopefully never
    /// see this but never say never I guess ^^
    ///
    /// **Why is this bad?** This is not a bad thing but definitely a hacky way to do it. See
    /// issue [#4310](https://github.com/rust-lang/rust-clippy/issues/4310) for a discussion
    /// about the implementation.
    ///
    /// **Known problems:** Hopefully none. It would be pretty uncool to have a problem here :)
    ///
    /// **Example output:**
    /// ```json,ignore
    /// {
    ///     "id": "internal_metadata_collector",
    ///     "id_span": {
    ///         "path": "clippy_lints/src/utils/internal_lints/metadata_collector.rs",
    ///         "line": 1
    ///     },
    ///     "group": "clippy::internal",
    ///     "docs": " **What it does:** Collects metadata about clippy lints for the website. [...] "
    /// }
    /// ```
    pub INTERNAL_METADATA_COLLECTOR,
    internal,
    "A busy bee collection metadata about lints"
}

impl_lint_pass!(MetadataCollector => [INTERNAL_METADATA_COLLECTOR]);

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, Default)]
pub struct MetadataCollector {
    lints: Vec<LintMetadata>,
}

impl Drop for MetadataCollector {
    fn drop(&mut self) {
        // You might ask: How hacky is this?
        // My answer:     YES
        let mut file = OpenOptions::new().write(true).create(true).open(OUTPUT_FILE).unwrap();
        writeln!(file, "{}", serde_json::to_string_pretty(&self.lints).unwrap()).unwrap();
    }
}

#[derive(Debug, Clone, Serialize)]
struct LintMetadata {
    id: String,
    id_span: SerializableSpan,
    group: String,
    docs: String,
}

#[derive(Debug, Clone, Serialize)]
struct SerializableSpan {
    path: String,
    line: usize,
}

impl SerializableSpan {
    fn from_item(cx: &LateContext<'_>, item: &Item<'_>) -> Self {
        Self::from_span(cx, item.ident.span)
    }

    fn from_span(cx: &LateContext<'_>, span: Span) -> Self {
        let loc: Loc = cx.sess().source_map().lookup_char_pos(span.lo());

        Self {
            path: format!("{}", loc.file.name),
            line: 1,
        }
    }
}

impl<'tcx> LateLintPass<'tcx> for MetadataCollector {
    fn check_item(&mut self, cx: &LateContext<'tcx>, item: &'tcx Item<'_>) {
        if_chain! {
            if let ItemKind::Static(ref ty, Mutability::Not, body_id) = item.kind;
            if is_lint_ref_type(cx, ty);
            let expr = &cx.tcx.hir().body(body_id).value;
            if let ExprKind::AddrOf(_, _, ref inner_exp) = expr.kind;
            if let ExprKind::Struct(_, _, _) = inner_exp.kind;
            then {
                let lint_name = item.ident.name.as_str().to_string().to_ascii_lowercase();
                if BLACK_LISTED_LINTS.contains(&lint_name.as_str()) {
                    return;
                }

                let group: String;
                let result = cx.lint_store.check_lint_name(lint_name.as_str(), Some(sym::clippy));
                if let CheckLintNameResult::Tool(Ok(lint_lst)) = result {
                    if let Some(group_some) = get_lint_group(cx, lint_lst[0]) {
                        group = group_some;
                    } else {
                        lint_collection_error(cx, item, "Unable to determine lint group");
                        return;
                    }
                } else {
                    lint_collection_error(cx, item, "Unable to find lint in lint_store");
                    return;
                }

                let docs: String;
                if let Some(docs_some) = extract_attr_docs(item) {
                    docs = docs_some;
                } else {
                    lint_collection_error(cx, item, "could not collect the lint documentation");
                    return;
                };

                self.lints.push(LintMetadata {
                    id: lint_name,
                    id_span: SerializableSpan::from_item(cx, item),
                    group,
                    docs,
                });
            }
        }
    }
}

/// This function collects all documentation that has been added to an item using
/// `#[doc = r""]` attributes. Several attributes are aggravated using line breaks
///
/// ```ignore
/// #[doc = r"Hello world!"]
/// #[doc = r"=^.^="]
/// struct SomeItem {}
/// ```
///
/// Would result in `Hello world!\n=^.^=\n`
fn extract_attr_docs(item: &Item<'_>) -> Option<String> {
    item.attrs
        .iter()
        .filter_map(|ref x| x.doc_str())
        .fold(None, |acc, sym| {
            let mut doc_str = sym.as_str().to_string();
            doc_str.push('\n');

            #[allow(clippy::option_if_let_else)] // See clippy#6737
            if let Some(mut x) = acc {
                x.push_str(&doc_str);
                Some(x)
            } else {
                Some(doc_str)
            }

            // acc.map_or(Some(doc_str), |mut x| {
            //     x.push_str(&doc_str);
            //     Some(x)
            // })
        })
}

fn get_lint_group(cx: &LateContext<'_>, lint_id: LintId) -> Option<String> {
    for (group_name, lints, _) in &cx.lint_store.get_lint_groups() {
        if lints.iter().any(|x| *x == lint_id) {
            return Some((*group_name).to_string());
        }
    }

    None
}

fn lint_collection_error(cx: &LateContext<'_>, item: &Item<'_>, message: &str) {
    span_lint(
        cx,
        INTERNAL_METADATA_COLLECTOR,
        item.ident.span,
        &format!("Metadata collection error for `{}`: {}", item.ident.name, message),
    );
}
