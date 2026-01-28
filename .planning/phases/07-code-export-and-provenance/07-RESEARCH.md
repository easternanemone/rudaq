# Phase 7: Code Export and Provenance - Research

**Researched:** 2026-01-22
**Domain:** Code generation, provenance tracking, scientific reproducibility
**Confidence:** MEDIUM

## Summary

Phase 7 requires bidirectional translation between visual experiment graphs and executable Rhai scripts, plus comprehensive provenance tracking for reproducibility. The project already has strong foundations: `daq-scripting` provides Rhai integration, `daq-egui/src/graph` contains the visual graph system, and `common::experiment::document` implements Bluesky-style document streaming with `ExperimentManifest` for hardware parameter snapshots.

**Key findings:**
- Rhai code generation is straightforward text templating from ExperimentNode AST (no complex parser needed)
- egui ecosystem has mature code editor widgets with syntax highlighting (`egui_code_editor`)
- Provenance tracking infrastructure already exists (ExperimentManifest captures device states)
- Git commit hash embedding uses standard build.rs patterns (vergen crate or manual `cargo:rustc-env`)
- One-way code generation (visual→code export only) is architecturally sound and matches scientific workflow tools

**Primary recommendation:** Use template-based code generation from ExperimentNode → Rhai script, integrate `egui_code_editor` for live preview pane, extend ExperimentManifest to capture graph version and git commit hash.

## Standard Stack

The established libraries/tools for code generation and provenance in Rust:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `rhai` | 1.x (in use) | Embedded scripting engine | Already integrated in daq-scripting, Rust-native, sandboxed |
| `egui_code_editor` | 0.2.4+ | Syntax highlighting code editor widget for egui | Mature, integrates with egui, supports syntax highlighting |
| `vergen` | 8.x+ | Build-time git metadata embedding | Industry standard for capturing version info in Rust binaries |
| `serde_json` | 1.x (in use) | JSON serialization | Already used for graph persistence |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `prettyplease` | 0.2+ | AST pretty-printing | If generating Rust code (not needed for Rhai) |
| `syntect` | 5.x+ | Syntax highlighting engine | Advanced highlighting (egui_code_editor uses it) |
| `chrono` | 0.4+ | Timestamp formatting | ISO 8601 dates for provenance metadata |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `egui_code_editor` | `egui::TextEdit` with manual highlighting | Manual highlighting is complex, reinventing the wheel |
| `vergen` | Manual build.rs with `git rev-parse` | vergen is more robust, handles edge cases |
| Template strings | AST visitor pattern | Templates are simpler for Rhai (not generating complex Rust) |

**Installation:**
```bash
# Add to daq-egui/Cargo.toml
egui_code_editor = "0.2"

# Add to workspace root build dependencies (for git hash capture)
[build-dependencies]
vergen = { version = "8", features = ["git", "gitcl"] }
```

## Architecture Patterns

### Recommended Project Structure
```
crates/daq-egui/src/
├── graph/
│   ├── codegen.rs          # ExperimentNode → Rhai code translation
│   ├── nodes.rs            # Existing node definitions
│   └── translation.rs      # Existing ExperimentNode → GraphPlan
├── panels/
│   └── code_preview.rs     # Code preview pane with egui_code_editor
└── widgets/
    └── script_editor.rs    # Full script editing mode (optional for Phase 7)

crates/common/src/experiment/
├── document.rs             # Existing (add git hash to ExperimentManifest)
└── provenance.rs           # New: Provenance helpers (git hash, graph version)
```

### Pattern 1: Template-Based Code Generation
**What:** Convert ExperimentNode AST to Rhai script via string templates
**When to use:** Generating simple scripting languages from structured data
**Example:**
```rust
// Source: Inferred from existing translation.rs pattern
impl ExperimentNode {
    fn to_rhai(&self) -> String {
        match self {
            ExperimentNode::Scan { actuator, start, stop, points } => {
                format!(
                    "// Scan {actuator} from {start} to {stop} in {points} steps\n\
                     for i in 0..{points} {{\n\
                     \tlet pos = {start} + ({stop} - {start}) * i / ({points} - 1);\n\
                     \t{actuator}.move_abs(pos);\n\
                     \t{actuator}.wait_settled();\n\
                     \tyield_event(#{{\n\
                     \t\t\"{actuator}\": pos\n\
                     \t}});\n\
                     }}"
                )
            }
            ExperimentNode::Move(config) => {
                format!(
                    "// Move {device} to {position}\n\
                     {device}.move_abs({position});\n\
                     {wait_line}",
                    device = config.device,
                    position = config.position,
                    wait_line = if config.wait_settled {
                        format!("{}.wait_settled();", config.device)
                    } else {
                        String::new()
                    }
                )
            }
            ExperimentNode::Acquire(config) => {
                let exposure_line = config.exposure_ms.map(|exp| {
                    format!("{}.set_exposure({});\n", config.detector, exp)
                }).unwrap_or_default();

                format!(
                    "// Acquire {count} frame(s) from {detector}\n\
                     {exposure_line}\
                     for _ in 0..{count} {{\n\
                     \t{detector}.trigger();\n\
                     \tlet frame = {detector}.read();\n\
                     \tyield_event(#{{\"frame\": frame}});\n\
                     }}",
                    detector = config.detector,
                    count = config.frame_count,
                )
            }
            ExperimentNode::Wait { condition } => {
                match condition {
                    WaitCondition::Duration { milliseconds } => {
                        format!("sleep({});", milliseconds / 1000.0)
                    }
                    // ... other conditions
                }
            }
            ExperimentNode::Loop(config) => {
                // Loop body nodes must be traversed recursively
                // (see translation.rs::find_loop_body_nodes pattern)
                format!("// Loop: {} iterations\nfor i in 0..{} {{\n\t// [body goes here]\n}}",
                    iterations, iterations)
            }
        }
    }
}
```

### Pattern 2: Live Code Preview Panel
**What:** Side-by-side view of node graph and generated Rhai script
**When to use:** Real-time feedback as user edits graph
**Example:**
```rust
// Using egui_code_editor for syntax highlighting
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};

pub struct CodePreviewPanel {
    generated_code: String,
    theme: ColorTheme,
}

impl CodePreviewPanel {
    fn update(&mut self, graph: &Snarl<ExperimentNode>) {
        // Regenerate code from graph
        self.generated_code = GraphPlan::to_rhai_script(graph);
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Generated Rhai Script");

        // Read-only code view
        CodeEditor::default()
            .id_source("code_preview")
            .with_rows(20)
            .with_fontsize(14.0)
            .with_theme(self.theme)
            .with_syntax(Syntax::rust()) // Rhai syntax similar to Rust
            .with_numlines(true)
            .show(ui, &mut self.generated_code);
    }
}
```

### Pattern 3: Provenance Metadata Capture
**What:** Extend ExperimentManifest to capture graph version + git commit
**When to use:** Every experiment run (already happens in RunEngine)
**Example:**
```rust
// In build.rs (using vergen)
use vergen::{vergen, Config};
fn main() {
    vergen(Config::default()).unwrap();
}

// In common/src/experiment/provenance.rs
impl ExperimentManifest {
    pub fn with_provenance(mut self, graph_file: Option<&Path>) -> Self {
        // Add git commit hash (set at build time)
        self.system_info.insert(
            "git_commit".to_string(),
            env!("VERGEN_GIT_SHA").to_string(),
        );

        // Add graph version if from file
        if let Some(path) = graph_file {
            self.system_info.insert(
                "graph_file".to_string(),
                path.display().to_string(),
            );

            // Compute graph hash (SHA256 of .expgraph file)
            if let Ok(hash) = compute_file_hash(path) {
                self.system_info.insert("graph_hash".to_string(), hash);
            }
        }

        self
    }
}
```

### Anti-Patterns to Avoid
- **Parsing Rhai back to graph:** Extremely fragile, breaks on comments/formatting. Stick to one-way export.
- **Real-time code generation on every node edit:** Generate on-demand (when preview pane is open) to avoid lag.
- **Embedding full .expgraph JSON in ExperimentManifest:** Too large. Use file hash instead.

## Don't Hand-Roll

Problems that look simple but have existing solutions:

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Syntax highlighting in egui | Custom tokenizer + theme engine | `egui_code_editor` crate | Handles line numbers, themes, cursor, selection, 20+ languages |
| Git commit hash at build time | Shell script in build.rs | `vergen` crate | Handles shallow clones, detached HEAD, dirty trees |
| AST pretty-printing | Manual string formatting | `prettyplease` (for Rust) or templates (for Rhai) | Handles indentation, line wrapping, operator precedence |
| ISO 8601 timestamps | Manual formatting | `chrono` crate | Timezone-aware, handles leap seconds, DST |
| File hashing (SHA256) | Custom crypto | `sha2` crate | Constant-time, audited, FIPS-compliant |

**Key insight:** Code generation for simple scripting languages (Rhai) is much simpler than full language parsers. Template-based generation is sufficient and maintainable.

## Common Pitfalls

### Pitfall 1: Graph → Code → Graph Round-Trip Temptation
**What goes wrong:** User edits generated code, expects graph to update. Parser breaks on comments, whitespace, custom functions.
**Why it happens:** Seems like natural UX, but parsing is NP-hard for arbitrary code.
**How to avoid:** Document clearly that code export is one-way. Provide "eject" button that switches to pure script mode (no graph sync).
**Warning signs:** User feature requests for "import script" or "sync code changes back to graph."

### Pitfall 2: Real-Time Code Generation Performance
**What goes wrong:** Generating code on every node drag/edit causes UI lag on large graphs.
**Why it happens:** Graph → Rhai translation is O(N) nodes, topological sort is O(N+E).
**How to avoid:**
- Generate code only when preview pane is open
- Debounce code generation (250ms delay after last edit)
- Cache generated code, invalidate on graph edit
**Warning signs:** Frame drops in egui profiler when editing graph.

### Pitfall 3: Incomplete Provenance (Missing Git Hash)
**What goes wrong:** ExperimentManifest saved without git commit hash, experiments can't be reproduced.
**Why it happens:** Forgot to add vergen to build.rs, or build in non-git directory.
**How to avoid:**
- Make `env!("VERGEN_GIT_SHA")` required at compile time (fails if missing)
- Add CI check that verifies git metadata is embedded
- Fallback to "unknown" + warning log if git unavailable
**Warning signs:** Empty/missing `git_commit` field in saved manifests.

### Pitfall 4: Rhai Syntax Divergence from Rust
**What goes wrong:** Generated Rhai code uses Rust syntax that Rhai doesn't support (e.g., `let mut`, `&`, closures).
**Why it happens:** Rhai is Rust-like but simpler (no lifetimes, no references, limited closures).
**How to avoid:**
- Test generated code execution in `daq-scripting` tests
- Use simple Rhai constructs: loops, functions, basic types
- Avoid Rust-specific features (mut, &, Box, Arc)
**Warning signs:** Generated code fails to parse in RhaiEngine::validate_script().

### Pitfall 5: Graph Serialization Version Skew
**What goes wrong:** Old .expgraph files fail to load after ExperimentNode enum changes.
**Why it happens:** Added new node type or field without migration logic.
**How to avoid:**
- Use serde(default) for new fields
- Increment GraphFile::version when adding breaking changes
- Add migration logic in load_graph() for old versions
**Warning signs:** User reports "Failed to parse graph file" after upgrade.

## Code Examples

Verified patterns from official sources:

### Complete Graph → Rhai Translation
```rust
// Source: Inferred from existing translation.rs + Rhai docs
use egui_snarl::Snarl;
use super::nodes::ExperimentNode;

pub fn graph_to_rhai_script(graph: &Snarl<ExperimentNode>) -> String {
    let mut script = String::new();

    // Header comment
    script.push_str("// Generated Rhai script from visual graph\n");
    script.push_str("// DO NOT EDIT - changes will be lost on next export\n\n");

    // Topological sort (reuse existing logic from translation.rs)
    let sorted_nodes = match topological_sort_nodes(graph) {
        Ok(nodes) => nodes,
        Err(e) => return format!("// ERROR: {}\n", e),
    };

    // Generate code for each node
    for node_id in sorted_nodes {
        if let Some(node) = graph.get_node(node_id) {
            script.push_str(&format!("\n// Node {:?}\n", node_id));
            script.push_str(&node.to_rhai());
            script.push_str("\n");
        }
    }

    script
}
```

### Code Preview Panel Integration
```rust
// Source: egui_code_editor docs + existing panel patterns
use egui_code_editor::{CodeEditor, ColorTheme};

pub struct CodePreviewPanel {
    code: String,
    visible: bool,
    last_graph_hash: u64, // For change detection
}

impl CodePreviewPanel {
    pub fn update(&mut self, ctx: &egui::Context, graph: &Snarl<ExperimentNode>) {
        // Only regenerate if graph changed
        let graph_hash = compute_graph_hash(graph);
        if graph_hash != self.last_graph_hash {
            self.code = graph_to_rhai_script(graph);
            self.last_graph_hash = graph_hash;
        }

        egui::SidePanel::right("code_preview")
            .resizable(true)
            .show_animated(ctx, self.visible, |ui| {
                ui.heading("Generated Code");

                ui.horizontal(|ui| {
                    if ui.button("Copy").clicked() {
                        ui.output_mut(|o| o.copied_text = self.code.clone());
                    }
                    if ui.button("Export...").clicked() {
                        // Open file dialog
                    }
                });

                ui.separator();

                // Read-only code editor
                let mut code = self.code.clone();
                CodeEditor::default()
                    .id_source("code_preview")
                    .with_rows(30)
                    .with_fontsize(12.0)
                    .with_theme(ColorTheme::GRUVBOX)
                    .with_numlines(true)
                    .show(ui, &mut code);
                // Discard changes (read-only)
            });
    }
}
```

### Git Commit Hash Capture
```rust
// Source: vergen crate documentation + community patterns
// In build.rs:
use vergen::EmitBuilder;

fn main() {
    EmitBuilder::builder()
        .all_git()
        .emit()
        .expect("Failed to extract git metadata");
}

// In common/src/experiment/provenance.rs:
impl ExperimentManifest {
    pub fn capture_build_info(&mut self) {
        self.system_info.insert(
            "git_commit".to_string(),
            env!("VERGEN_GIT_SHA").to_string(),
        );
        self.system_info.insert(
            "git_commit_date".to_string(),
            env!("VERGEN_GIT_COMMIT_DATE").to_string(),
        );
        self.system_info.insert(
            "git_dirty".to_string(),
            env!("VERGEN_GIT_DIRTY").to_string(),
        );
        self.system_info.insert(
            "build_date".to_string(),
            env!("VERGEN_BUILD_DATE").to_string(),
        );
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual script writing | Visual graph editor | 2024-2025 | Scientists design without code |
| Rustfmt for code formatting | prettyplease for AST → code | 2021+ | Generated code is always valid |
| Manual git hash in code | vergen build-time capture | 2019+ | Automatic provenance tracking |
| Custom text editors | egui_code_editor | 2023+ | Syntax highlighting out-of-box |

**Deprecated/outdated:**
- **syn + quote for Rhai generation:** Overkill for simple scripting language. Use string templates instead.
- **egui::TextEdit for code:** Missing syntax highlighting, line numbers. Use `egui_code_editor`.
- **Manual timestamp formatting:** Use `chrono` for ISO 8601 compliance.

## Open Questions

1. **Graph Hash Algorithm**
   - What we know: SHA256 is standard, but full .expgraph file may be large
   - What's unclear: Should we hash the entire file or just the graph structure (exclude metadata)?
   - Recommendation: Hash entire file for simplicity, files are typically <100KB

2. **Loop Body Code Generation**
   - What we know: translation.rs has find_loop_body_nodes() for GraphPlan translation
   - What's unclear: How to represent nested loop bodies in linear Rhai script (indentation, comments?)
   - Recommendation: Follow translation.rs pattern, indent body with tabs, add iteration markers as comments

3. **Script Editor Mode (CODE-03 "eject")**
   - What we know: One-way export means code edits can't sync back to graph
   - What's unclear: Should "eject" be destructive (lose graph) or create parallel code-only mode?
   - Recommendation: Parallel mode - keep .expgraph file, switch UI to code editor, disable graph panel

4. **Rhai Syntax Highlighting in egui_code_editor**
   - What we know: egui_code_editor supports Rust syntax (similar to Rhai)
   - What's unclear: Does Rust highlighting work well enough for Rhai, or need custom syntax?
   - Recommendation: Start with Rust syntax, add custom Rhai if users report confusion

## Sources

### Primary (HIGH confidence)
- [Rhai Documentation](https://rhai.rs/) - Syntax and examples
- [egui_code_editor crate](https://crates.io/crates/egui_code_editor) - Code preview widget
- [vergen crate](https://crates.io/crates/vergen) - Build-time git metadata
- Existing codebase:
  - `crates/daq-scripting/src/rhai_engine.rs` - Rhai integration
  - `crates/daq-egui/src/graph/translation.rs` - Graph → Plan translation pattern
  - `crates/common/src/experiment/document.rs` - ExperimentManifest structure

### Secondary (MEDIUM confidence)
- [W3C PROV Standard](https://www.w3.org/TR/prov-overview/) (via WebSearch) - Provenance metadata standards
- [Nature Scientific Data - Metadata Practices](https://www.nature.com/articles/s41597-025-05126-1) - Scientific reproducibility best practices
- [prettyplease crate](https://docs.rs/prettyplease/latest/prettyplease/) - AST pretty-printing reference
- [Provenance Tracking Best Practices](https://rrcns.readthedocs.io/en/latest/provenance_tracking.html) - Neurophysiology data management guide

### Tertiary (LOW confidence)
- [The Art of Formatting Code](https://mcyoung.xyz/2025/03/11/formatters/) - Pretty-printer theory (general background)
- Stack Overflow patterns for build.rs git hash capture - Community knowledge

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - All libraries verified in crates.io, Rhai already integrated
- Architecture: MEDIUM - Template pattern is straightforward, but loop nesting needs validation
- Pitfalls: MEDIUM - Based on common code generation patterns, not Rhai-specific experience
- Provenance: HIGH - ExperimentManifest already exists, vergen is standard practice

**Research date:** 2026-01-22
**Valid until:** 30 days (Stable domain - code generation patterns change slowly)
