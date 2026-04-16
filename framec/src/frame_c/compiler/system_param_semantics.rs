use std::collections::HashMap;

use crate::frame_c::compiler::arcanum::Arcanum;
use crate::frame_c::compiler::ast::StateDecl;
use crate::frame_c::visitors::TargetLanguage;
// Choose the first declared state for a system via Arcanum spans.
pub(crate) fn first_state_for_system<'a>(
    arc: &'a Arcanum,
    sys_name: &str,
) -> Option<&'a StateDecl> {
    let sys = arc.systems.get(sys_name)?;
    let mut best: Option<&StateDecl> = None;
    let mut best_start: Option<usize> = None;
    for mach in sys.machines.values() {
        for st in mach.states.values() {
            match best_start {
                None => {
                    best_start = Some(st.span.start);
                    best = Some(st);
                }
                Some(cur) => {
                    if st.span.start < cur {
                        best_start = Some(st.span.start);
                        best = Some(st);
                    }
                }
            }
        }
    }
    best
}

// Extract bare parameter names from a header string.
pub(crate) fn header_param_names(hdr: &str) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(lp) = hdr.find('(') {
        if let Some(rp_rel) = hdr[lp + 1..].find(')') {
            let rp = lp + 1 + rp_rel;
            let inside = &hdr[lp + 1..rp];
            for raw in inside.split(',') {
                let t = raw.trim();
                if t.is_empty() {
                    continue;
                }
                let base = t
                    .split(|c| c == '=' || c == ':')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !base.is_empty() {
                    names.push(base.to_string());
                }
            }
        }
    }
    names
}

// Collect domain variable names per system via the V4 segmenter + parser.
// No byte-level scanning here: the segmenter locates `@@system` blocks and
// the V4 parser produces structured `DomainVar`s — we just project the names.
pub(crate) fn collect_domain_vars_per_system(
    bytes: &[u8],
    lang: TargetLanguage,
) -> HashMap<String, Vec<String>> {
    use crate::frame_c::compiler::frame_ast::Span as AstSpan;
    use crate::frame_c::compiler::pipeline_parser;
    use crate::frame_c::compiler::segmenter;

    let mut result: HashMap<String, Vec<String>> = HashMap::new();

    // Segmentation/parse failures here are silently absorbed: the main
    // compile path surfaces them with proper diagnostics, and re-reporting
    // through E418 would just be noise.
    let Ok(source_map) = segmenter::segment_source(bytes, lang) else {
        return result;
    };

    for segment in &source_map.segments {
        let segmenter::Segment::System {
            name, body_span, ..
        } = segment
        else {
            continue;
        };
        let ast_span = AstSpan::new(body_span.start, body_span.end);
        let Ok(sys_ast) = pipeline_parser::parse_system(bytes, name.clone(), ast_span, lang) else {
            continue;
        };
        result.insert(
            name.clone(),
            sys_ast.domain.iter().map(|dv| dv.name.clone()).collect(),
        );
    }

    result
}
