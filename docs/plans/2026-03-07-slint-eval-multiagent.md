# Slint GUI Evaluation — Multi-Agent Coordination Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Evaluate whether Slint can replace egui as the rust-daq GUI by running three parallel kill-tests, coordinating results via BeadHub mail/chat, and reaching a go/no-go decision.

**Architecture:** Four agents work concurrently — one coordinator (alice) scaffolds the Slint skeleton and synthesizes results; three specialists (bob, charlie, dave) each own one kill-test. All work in isolated git worktrees on separate branches. No agent merges to main — output is findings, not shipped code.

**Tech Stack:** Slint 1.x, Rust async/Tokio, trunk (WASM), tonic-web-wasm-client, BeadHub mail + chat for coordination.

---

## Beads Tasks

| ID | Agent | Title |
|----|-------|-------|
| `rust-daq-001` | alice | [coordinator] Scaffold skeleton + coordinate workstreams |
| `rust-daq-002` | bob   | [bob] WASM + gRPC-web browser test |
| `rust-daq-003` | charlie | [charlie] Docking + floating-window prototype |
| `rust-daq-004` | dave  | [dave] Plotting widget + rendering benchmark |

Dependencies: 002, 003, 004 each depend on 001. `bd-ejx9` (epic) closes after all four are done.

---

## Coordination Flow

```
alice: claim rust-daq-001 → scaffold → mail bob/charlie/dave "skeleton ready, claim your tasks"
         ↓
bob ────────────────────────────────────────────────────────┐
charlie ─ work independently, no shared files               ├─ mail alice results
dave ───────────────────────────────────────────────────────┘
         ↓
alice: reads 3 mail reports → opens 3-way chat → makes go/no-go → closes bd-ejx9
```

---

## Task 1 (alice): Scaffold Skeleton

**Worktree:** `/Users/briansquires/code/rust-daq` (main, already open)
**Branch:** create `eval/slint-multiagent` from main

**Step 1: Create branch**
```bash
git checkout -b eval/slint-multiagent
```

**Step 2: Create minimal Slint crate**
```bash
mkdir -p crates/ui-slint/src
```

`crates/ui-slint/Cargo.toml`:
```toml
[package]
name = "ui-slint"
version = "0.1.0"
edition = "2021"

[dependencies]
slint = "1"

[[bin]]
name = "ui-slint"
path = "src/main.rs"
```

`crates/ui-slint/src/main.rs`:
```rust
slint::slint! {
    export component MainWindow inherits Window {
        title: "rust-daq (Slint eval)";
        width: 1280px;
        height: 800px;
        Text { text: "Slint skeleton — evaluation in progress"; }
    }
}

fn main() {
    MainWindow::new().unwrap().run().unwrap();
}
```

**Step 3: Verify it compiles**
```bash
cargo build -p ui-slint
```
Expected: compiles without errors.

**Step 4: Commit**
```bash
git add crates/ui-slint/
git commit -m "feat(eval): add Slint skeleton crate for multi-agent evaluation"
```

**Step 5: Notify specialists**
```bash
bdh update rust-daq-001 --status=done
bdh :aweb mail send bob "Skeleton ready at eval/slint-multiagent. Claim rust-daq-002 and start WASM test."
bdh :aweb mail send charlie "Skeleton ready at eval/slint-multiagent. Claim rust-daq-003 and start docking test."
bdh :aweb mail send dave "Skeleton ready at eval/slint-multiagent. Claim rust-daq-004 and start plot/perf test."
```

---

## Task 2 (bob): WASM + gRPC-web

**Worktree:** `/Users/briansquires/code/rust-daq-bob`
**Branch:** `bob` (already created)

**Step 1: Claim task**
```bash
bdh update rust-daq-002 --status=in_progress
```

**Step 2: Add WASM target**
```bash
rustup target add wasm32-unknown-unknown
cargo install trunk
```

**Step 3: Add web feature to Cargo.toml**
Add to `crates/ui-slint/Cargo.toml`:
```toml
[features]
web = ["slint/wasm"]

[target.'cfg(target_arch = "wasm32")'.dependencies]
tonic-web-wasm-client = "0.6"
wasm-bindgen-futures = "0.4"
```

**Step 4: Add index.html for trunk**
`crates/ui-slint/index.html`:
```html
<!DOCTYPE html>
<html><head><meta charset="utf-8"/></head>
<body><script type="module"></script></body></html>
```

**Step 5: Build for WASM**
```bash
cd crates/ui-slint && trunk build
```
Note: PASS if it compiles. FAIL if Slint's WASM support is incomplete.

**Step 6: Test gRPC-web call**
Add to `src/main.rs` behind `#[cfg(target_arch = "wasm32")]`:
```rust
// call daemon health endpoint via gRPC-web
// document whether it works
```

**Step 7: Report results**
```bash
bdh close rust-daq-002
bdh :aweb mail send alice "[bob] WASM result: <PASS|FAIL> — bundle size: <X>MB — gRPC-web: <PASS|FAIL> — <blockers if any>"
```

---

## Task 3 (charlie): Docking + Floating Windows

**Worktree:** `/Users/briansquires/code/rust-daq-charlie`
**Branch:** `charlie` (already created)

**Step 1: Claim task**
```bash
bdh update rust-daq-003 --status=in_progress
```

**Step 2: Research docking**
Check https://crates.io for Slint docking crates. Check Slint GitHub issues/discussions for floating window support. Document findings.

**Step 3: Build 3-panel prototype**
Create `crates/ui-slint/src/docking_proto.rs` with:
- Camera panel (placeholder 640×480 rect)
- Parameter panel (3 sliders: exposure, gain, threshold)
- Log panel (scrolling text list)

**Step 4: Attempt floating windows**
Try `slint::Window` or platform window APIs to detach a panel into its own OS window.

**Step 5: Document findings**
Answer these specific questions:
1. Is there a `slint-dock` or equivalent crate? (y/n)
2. Can panels become independent OS windows? (y/n)
3. Layout DSL: how many lines to replicate one egui_dock tab? (count)
4. Showstopper for 18-panel rust-daq UI? (y/n + reason)

**Step 6: Report results**
```bash
bdh close rust-daq-003
bdh :aweb mail send alice "[charlie] Docking result: <PASS|FAIL> — floating windows: <y/n> — dock crate: <name|none> — showstopper: <y/n>"
```

---

## Task 4 (dave): Plotting + Benchmark

**Worktree:** `/Users/briansquires/code/rust-daq-dave`
**Branch:** `dave` (already created)

**Step 1: Claim task**
```bash
bdh update rust-daq-004 --status=in_progress
```

**Step 2: Research plotting**
Check `slint-charts`, `plotters` with Slint backend, or custom canvas. Document which approach is viable.

**Step 3: Build 1D trace prototype**
Simulate 1024-point detector trace updating at 30 Hz. Render in a Slint component.

**Step 4: Benchmark render loop**
```rust
let mut frames = 0u32;
let start = std::time::Instant::now();
while start.elapsed().as_secs() < 5 {
    // trigger repaint
    frames += 1;
}
let fps = frames as f64 / 5.0;
println!("FPS: {fps:.1}");
```
Target: ≥ 60 fps (egui baseline: 163 fps ceiling from evaluation notes).

**Step 5: Report results**
```bash
bdh close rust-daq-004
bdh :aweb mail send alice "[dave] Plot/perf result: <PASS|FAIL> — <FPS> fps — plot solution: <crate|none> — <one line>"
```

---

## Task 5 (alice): Synthesize + Decide

**Step 1: Read all three mail reports**
```bash
bdh :aweb mail list
bdh :aweb mail open bob
bdh :aweb mail open charlie
bdh :aweb mail open dave
```

**Step 2: Open 3-way chat for consensus**
```bash
bdh :aweb chat send-and-wait bob "All results in. WASM: <result>, Docking: <result>, Perf: <result>. Recommend GO/NO-GO?" --start-conversation
bdh :aweb chat send-and-wait charlie "Same question — GO or NO-GO on Slint migration?" --start-conversation
bdh :aweb chat send-and-wait dave "Your perf numbers + GO/NO-GO recommendation?" --start-conversation
```

**Step 3: Close the epic with decision**
```bash
bdh close bd-ejx9 --reason="Decision: <GO|NO-GO> — WASM <pass|fail>, Docking <pass|fail>, Perf <N fps>. <1-2 sentence rationale>"
```

**Step 4: Record in bd memories**
```bash
bdh remember "slint-eval-result: <GO|NO-GO> — <date> — WASM <result>, docking <result>, perf <N fps>"
```

---

## Starting the Demo

Open 4 terminal sessions, one per agent:

```bash
# Terminal 1 — alice (coordinator)
cd ~/code/rust-daq
bdh update rust-daq-001 --status=in_progress
# ... follow Task 1 steps

# Terminal 2 — bob
cd ~/code/rust-daq-bob
bdh :aweb mail list   # wait for alice's "skeleton ready" mail
bdh update rust-daq-002 --status=in_progress

# Terminal 3 — charlie
cd ~/code/rust-daq-charlie
bdh :aweb mail list
bdh update rust-daq-003 --status=in_progress

# Terminal 4 — dave
cd ~/code/rust-daq-dave
bdh :aweb mail list
bdh update rust-daq-004 --status=in_progress
```

The BeadHub dashboard at http://localhost:5173 will show all 4 workspaces active with live claim status.
