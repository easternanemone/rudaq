# Architectural Flaw Analysis: The Three Competing Cores

> **⚠️ HISTORICAL REFERENCE - RESOLVED (2025-12-07)**
>
> This document describes architectural issues that existed in rust-daq prior to the V5 unification.
> The V5 "headless-first" architecture has been completed and all V1/V2/V3/V4 code has been removed.
> This document is retained for historical reference only.

## Summary of Findings

The rust-daq project suffers from a mission-critical architectural flaw: it contains three separate, competing, and incompatible core architectures (V1, V2, and V3) that all coexist within the same codebase. This 'architectural schism' is the root cause of numerous other problems.

**The Three Architectures:**
1.  **V1 (`src/core.rs`):** The original system, which is still partially in use.
2.  **V2 (`crates/daq-core`):** The first attempted rewrite. It introduced a separation between `HardwareAdapter` and `Instrument`, but the migration was never completed.
3.  **V3 (`src/core_v3.rs`):** A second attempted rewrite, aiming to unify the previous two. It introduces better patterns like capability traits (`Camera`, `Stage`), but is also incomplete.

**Consequences:**
*   **Extreme Complexity:** Developers must understand three different systems, leading to high cognitive overhead and a chaotic development environment.
*   **Incomplete Migrations:** The project is littered with glue code (e.g., `From` trait implementations) to convert data types between the different architectural versions. This adds performance overhead and is a major source of bugs.
*   **Broken Abstractions:** To bridge the gaps between architectures, shortcuts were taken. The most egregious is the `as_any()` method on the V2 `HardwareAdapter` trait, which is used by instruments to downcast the adapter to a concrete type (e.g., `SerialAdapter`). This completely breaks the abstraction and creates tight coupling, defeating the purpose of the trait-based design.
*   **Inconsistent Design:** Different versions have different approaches to fundamental problems like configuration and command handling. For example, the V1 architecture has better type safety for instrument parameters than the V2 architecture, making the value of the refactoring questionable.
*   **Untenable Maintenance:** The codebase is a maze of versioned files (`core_v3.rs`), versioned directories (`instruments_v2`), and duplicated instrument implementations. This makes bug fixing and feature development incredibly difficult and error-prone.

## Recommendation

The project must halt all feature development and address this architectural debt as a top priority. A single, unified architecture (likely based on the best ideas from V3, like capability traits) must be chosen. A clear migration plan needs to be created and executed to move all instruments to the chosen architecture, after which the V1 and V2 core files and old instrument implementations must be deleted. Without this radical cleanup, the project is at high risk of collapsing under its own complexity.
