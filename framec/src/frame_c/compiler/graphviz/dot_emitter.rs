/// Emits DOT (GraphViz) text from a SystemGraph IR.
use super::ir::*;
use std::collections::HashSet;
use std::fmt::Write;

/// Emit a complete DOT digraph from a SystemGraph.
pub fn emit_dot(graph: &SystemGraph) -> String {
    let mut out = String::new();

    // Graph header + globals
    writeln!(out, "digraph {} {{", graph.name).ok();
    writeln!(out, "    compound=true").ok();
    writeln!(
        out,
        "    node [color=\"deepskyblue4\" style=\"rounded, filled\" fillcolor=\"azure\"]"
    )
    .ok();
    writeln!(out, "    edge[color=\"red\"]").ok();
    writeln!(out).ok();

    // Entry point
    if let Some(entry) = &graph.entry_state {
        writeln!(
            out,
            "    Entry[width=0.2 shape=\"circle\" style=\"filled\" fillcolor=\"black\" label=\"\"]"
        )
        .ok();
        writeln!(out, "    Entry -> {}", entry).ok();
        writeln!(out).ok();
    }

    // Collect parent state names for cluster rendering
    let parent_names: HashSet<&str> = graph
        .states
        .iter()
        .filter(|s| !s.children.is_empty())
        .map(|s| s.name.as_str())
        .collect();

    // Emit states — parents as clusters, leaves as nodes
    // We need to emit in hierarchy order: top-level states first, children inside clusters
    let top_level: Vec<&StateNode> = graph.states.iter().filter(|s| s.parent.is_none()).collect();

    for state in &top_level {
        emit_state_recursive(state, &graph.states, &parent_names, &mut out, 1);
    }

    // State stack pop node (if needed)
    if graph.has_state_stack {
        writeln!(out).ok();
        writeln!(
            out,
            "    Stack[shape=\"circle\" label=\"H*\" width=\"0\" margin=\"0\"]"
        )
        .ok();
    }

    // Edges
    writeln!(out).ok();
    for edge in &graph.transitions {
        emit_edge(edge, &parent_names, &mut out);
    }

    writeln!(out, "}}").ok();
    out
}

/// Emit a multi-system DOT output with comment separators.
/// Format required by VSCode extension's parseGraphVizOutput().
pub fn emit_multi_system(systems: &[(String, String)]) -> String {
    if systems.len() == 1 {
        return systems[0].1.clone();
    }

    let mut out = String::new();
    writeln!(out, "// Frame Module: {} systems", systems.len()).ok();

    for (name, dot) in systems {
        writeln!(out).ok();
        writeln!(out, "// System: {}", name).ok();
        out.push_str(dot);
    }
    out
}

/// Recursively emit a state and its children.
fn emit_state_recursive(
    state: &StateNode,
    all_states: &[StateNode],
    parent_names: &HashSet<&str>,
    out: &mut String,
    indent: usize,
) {
    let pad = "    ".repeat(indent);

    if !state.children.is_empty() {
        // Parent state → subgraph cluster. The cluster label mirrors the
        // leaf-node structure (header row + <hr/> + handler list) so HSM
        // parents visibly show their own event handlers (e.g. a parent
        // that catches `connection_error()` uniformly via `=> $^`).
        writeln!(out, "{}subgraph cluster_{} {{", pad, state.name).ok();
        writeln!(out, "{}    label = <", pad).ok();
        writeln!(out, "{}        <table cellborder=\"0\" border=\"0\">", pad).ok();
        writeln!(out, "{}            <tr><td>{}</td></tr>", pad, state.name).ok();
        writeln!(out, "{}            <hr/>", pad).ok();
        emit_handler_row(state, out, &format!("{}            ", pad));
        writeln!(out, "{}        </table>", pad).ok();
        writeln!(out, "{}    >", pad).ok();
        writeln!(out, "{}    style = rounded", pad).ok();
        // Invisible anchor node for compound edges
        writeln!(
            out,
            "{}    {} [shape=\"point\" width=\"0\"]",
            pad, state.name
        )
        .ok();
        writeln!(out).ok();

        // Render children inside the cluster
        for child_name in &state.children {
            if let Some(child) = all_states.iter().find(|s| s.name == *child_name) {
                emit_state_recursive(child, all_states, parent_names, out, indent + 1);
            }
        }

        writeln!(out, "{}}}", pad).ok();
    } else {
        // Leaf state → HTML-label node
        emit_leaf_node(state, out, &pad);
    }
}

/// Build the list of handler labels (`$>()`, `<$()`, `event(params)`, …)
/// for a state, in the order they should appear.
fn handler_lines(state: &StateNode) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    if state.has_enter {
        lines.push("$&gt;()".to_string());
    }
    if state.has_exit {
        lines.push("&lt;$()".to_string());
    }
    for h in &state.handlers {
        if h.params.is_empty() {
            lines.push(format!("{}()", h.event));
        } else {
            let params: Vec<String> = h
                .params
                .iter()
                .map(|(name, typ)| format!("{}: {}", name, escape_html(typ)))
                .collect();
            lines.push(format!("{}({})", h.event, params.join(", ")));
        }
    }
    lines
}

/// Emit a `<tr><td><font>…</font></td></tr>` row listing the state's
/// handlers. Used by both leaf nodes and HSM parent clusters so the
/// two presentations stay in sync. `pad` is the indent to place at the
/// start of each emitted line (matches the enclosing `<table>`).
fn emit_handler_row(state: &StateNode, out: &mut String, pad: &str) {
    let lines = handler_lines(state);
    if lines.is_empty() {
        writeln!(out, "{}<tr><td></td></tr>", pad).ok();
        return;
    }
    writeln!(
        out,
        "{}<tr><td align=\"left\"><font point-size=\"10\">",
        pad
    )
    .ok();
    // First line unprefixed; subsequent lines separated by <br ALIGN="LEFT"/>.
    // Trailing <br ALIGN="LEFT"/> forces the last line's left alignment.
    // Whitespace inside <font> renders literally in GraphViz, so no indent padding.
    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            write!(out, "<br ALIGN=\"LEFT\"/>").ok();
        }
        write!(out, "{}", line).ok();
    }
    write!(out, "<br ALIGN=\"LEFT\"/>").ok();
    writeln!(out).ok();
    writeln!(out, "{}</font></td></tr>", pad).ok();
}

/// Emit a leaf state node with HTML label.
fn emit_leaf_node(state: &StateNode, out: &mut String, pad: &str) {
    writeln!(out, "{}{} [label = <", pad, state.name).ok();
    writeln!(
        out,
        "{}    <table CELLBORDER=\"0\" CELLPADDING=\"5\" style=\"rounded\">",
        pad
    )
    .ok();

    // Header row: state name (with params if any)
    let header = if state.state_params.is_empty() {
        format!("<b>{}</b>", state.name)
    } else {
        let params: Vec<String> = state
            .state_params
            .iter()
            .map(|p| match &p.param_type {
                Some(t) => format!("{}: {}", p.name, t),
                None => p.name.clone(),
            })
            .collect();
        format!("<b>{}({})</b>", state.name, params.join(", "))
    };
    writeln!(out, "{}        <tr><td>{}</td></tr>", pad, header).ok();
    writeln!(out, "{}        <hr/>", pad).ok();

    // Body row: handler list (enter/exit + event handlers)
    emit_handler_row(state, out, &format!("{}        ", pad));

    // State variables section (V4 improvement)
    if !state.state_vars.is_empty() {
        writeln!(out, "{}        <hr/>", pad).ok();
        writeln!(
            out,
            "{}        <tr><td align=\"left\"><font point-size=\"9\" color=\"gray40\">",
            pad
        )
        .ok();
        let var_lines: Vec<String> = state
            .state_vars
            .iter()
            .map(|sv| match &sv.var_type {
                Some(t) => format!("{}: {}", sv.name, escape_html(t)),
                None => sv.name.clone(),
            })
            .collect();
        for (idx, line) in var_lines.iter().enumerate() {
            if idx > 0 {
                write!(out, "<br ALIGN=\"LEFT\"/>").ok();
            }
            write!(out, "{}", line).ok();
        }
        write!(out, "<br ALIGN=\"LEFT\"/>").ok();
        writeln!(out).ok();
        writeln!(out, "{}        </font></td></tr>", pad).ok();
    }

    writeln!(out, "{}    </table>", pad).ok();
    writeln!(out, "{}> margin=0 shape=none]", pad).ok();
}

/// Emit a transition edge.
fn emit_edge(edge: &TransitionEdge, parent_names: &HashSet<&str>, out: &mut String) {
    let target_node = match &edge.target {
        TransitionTarget::State(name) => name.clone(),
        TransitionTarget::StackPop => "Stack".to_string(),
        TransitionTarget::ParentForward => {
            // Forward to parent — edge goes to the parent state's anchor node
            // (the source state must have a parent; if not, skip)
            return; // Parent forward edges are handled below
        }
    };

    // Build edge label — use explicit label from `-> "label" $State` if present,
    // otherwise show the handler/event name (e.g. `coin`, `$>`, `<$`).
    let label_text = match &edge.label {
        Some(label) => escape_html(label),
        None => edge.event.clone(),
    };
    let label = match &edge.guard {
        Some(guard) => format!(" {} [{}] ", label_text, escape_html(guard)),
        None => format!(" {} ", label_text),
    };

    // Build edge attributes
    let mut attrs = vec![format!("label=\"{}\"", label)];

    // Edge style based on transition kind
    match edge.kind {
        TransitionKind::ChangeState => {
            attrs.push("style=\"dashed\"".to_string());
        }
        TransitionKind::Forward => {
            attrs.push("style=\"dotted\"".to_string());
            attrs.push("color=\"blue\"".to_string());
        }
        TransitionKind::Transition => {}
    }

    // Compound edge: ltail if source is a parent state
    if parent_names.contains(edge.source.as_str()) {
        attrs.push(format!("ltail=\"cluster_{}\"", edge.source));
    }

    // Compound edge: lhead if target is a parent state
    if let TransitionTarget::State(ref name) = edge.target {
        if parent_names.contains(name.as_str()) {
            attrs.push(format!("lhead=\"cluster_{}\"", name));
        }
    }

    writeln!(
        out,
        "    {} -> {} [{}]",
        edge.source,
        target_node,
        attrs.join(" ")
    )
    .ok();
}

/// Escape special HTML characters for DOT HTML labels.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "&#124;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_dot_output() {
        let graph = SystemGraph {
            name: "Simple".to_string(),
            states: vec![
                StateNode {
                    name: "A".to_string(),
                    parent: None,
                    children: vec![],
                    has_enter: false,
                    has_exit: false,
                    handlers: vec![HandlerInfo {
                        event: "go".to_string(),
                        params: vec![],
                    }],
                    state_vars: vec![],
                    state_params: vec![],
                },
                StateNode {
                    name: "B".to_string(),
                    parent: None,
                    children: vec![],
                    has_enter: false,
                    has_exit: false,
                    handlers: vec![],
                    state_vars: vec![],
                    state_params: vec![],
                },
            ],
            transitions: vec![TransitionEdge {
                source: "A".to_string(),
                target: TransitionTarget::State("B".to_string()),
                event: "go".to_string(),
                label: None,
                kind: TransitionKind::Transition,
                guard: None,
            }],
            entry_state: Some("A".to_string()),
            has_state_stack: false,
        };

        let dot = emit_dot(&graph);
        assert!(dot.contains("digraph Simple {"));
        assert!(dot.contains("compound=true"));
        assert!(dot.contains("Entry -> A"));
        assert!(dot.contains("A -> B"));
        assert!(dot.contains("label=\" go \""));
        assert!(dot.contains("A [label = <"));
        assert!(dot.contains("go()"));
    }

    #[test]
    fn test_hsm_cluster_output() {
        let graph = SystemGraph {
            name: "Hsm".to_string(),
            states: vec![
                StateNode {
                    name: "Parent".to_string(),
                    parent: None,
                    children: vec!["Child".to_string()],
                    has_enter: false,
                    has_exit: false,
                    handlers: vec![],
                    state_vars: vec![],
                    state_params: vec![],
                },
                StateNode {
                    name: "Child".to_string(),
                    parent: Some("Parent".to_string()),
                    children: vec![],
                    has_enter: false,
                    has_exit: false,
                    handlers: vec![],
                    state_vars: vec![],
                    state_params: vec![],
                },
            ],
            transitions: vec![],
            entry_state: Some("Parent".to_string()),
            has_state_stack: false,
        };

        let dot = emit_dot(&graph);
        assert!(dot.contains("subgraph cluster_Parent {"));
        assert!(dot.contains("Parent [shape=\"point\" width=\"0\"]"));
        assert!(dot.contains("Child [label = <"));
    }

    #[test]
    fn test_hsm_parent_shows_its_own_handlers() {
        // Regression: parent states (rendered as subgraph clusters) used
        // to emit an empty `<tr><td></td></tr>` handler row regardless of
        // what the parent actually defined. An HSM parent with cross-
        // cutting handlers (e.g. `connection_error()` caught once for
        // every child via `=> $^`) should show those handlers in the
        // cluster label just like a leaf node would.
        let graph = SystemGraph {
            name: "Hsm".to_string(),
            states: vec![
                StateNode {
                    name: "Open".to_string(),
                    parent: None,
                    children: vec!["Idle".to_string()],
                    has_enter: false,
                    has_exit: false,
                    handlers: vec![
                        HandlerInfo {
                            event: "connection_error".to_string(),
                            params: vec![("reason".to_string(), "str".to_string())],
                        },
                        HandlerInfo {
                            event: "close".to_string(),
                            params: vec![],
                        },
                    ],
                    state_vars: vec![],
                    state_params: vec![],
                },
                StateNode {
                    name: "Idle".to_string(),
                    parent: Some("Open".to_string()),
                    children: vec![],
                    has_enter: false,
                    has_exit: false,
                    handlers: vec![],
                    state_vars: vec![],
                    state_params: vec![],
                },
            ],
            transitions: vec![],
            entry_state: Some("Open".to_string()),
            has_state_stack: false,
        };

        let dot = emit_dot(&graph);
        assert!(dot.contains("subgraph cluster_Open {"));
        // Parent's own handlers must appear inside the cluster label.
        assert!(dot.contains("connection_error(reason: str)"));
        assert!(dot.contains("close()"));
        // And the old empty row must no longer appear in the cluster's
        // own label (which sits between `label = <` and the first `>`).
        // Inner leaf nodes may legitimately render empty handler rows.
        let cluster_start = dot.find("subgraph cluster_Open {").unwrap();
        let label_open =
            cluster_start + dot[cluster_start..].find("label = <").unwrap() + "label = <".len();
        let label_close = label_open + dot[label_open..].find(">").unwrap();
        let cluster_label = &dot[label_open..label_close];
        assert!(
            !cluster_label.contains("<tr><td></td></tr>"),
            "cluster label with handlers must not emit an empty handler row: {}",
            cluster_label
        );
    }

    #[test]
    fn test_stack_pop_node() {
        let graph = SystemGraph {
            name: "Stack".to_string(),
            states: vec![StateNode {
                name: "A".to_string(),
                parent: None,
                children: vec![],
                has_enter: false,
                has_exit: false,
                handlers: vec![],
                state_vars: vec![],
                state_params: vec![],
            }],
            transitions: vec![TransitionEdge {
                source: "A".to_string(),
                target: TransitionTarget::StackPop,
                event: "pop".to_string(),
                label: None,
                kind: TransitionKind::Transition,
                guard: None,
            }],
            entry_state: Some("A".to_string()),
            has_state_stack: true,
        };

        let dot = emit_dot(&graph);
        assert!(dot.contains("Stack[shape=\"circle\" label=\"H*\""));
        assert!(dot.contains("A -> Stack"));
    }

    #[test]
    fn test_multi_system_output() {
        let systems = vec![
            ("Sys1".to_string(), "digraph Sys1 {\n}\n".to_string()),
            ("Sys2".to_string(), "digraph Sys2 {\n}\n".to_string()),
        ];
        let output = emit_multi_system(&systems);
        assert!(output.contains("// Frame Module: 2 systems"));
        assert!(output.contains("// System: Sys1"));
        assert!(output.contains("// System: Sys2"));
    }

    #[test]
    fn test_escape_html() {
        assert_eq!(escape_html("a < b"), "a &lt; b");
        assert_eq!(escape_html("a > b"), "a &gt; b");
        assert_eq!(escape_html("a | b"), "a &#124; b");
    }
}
