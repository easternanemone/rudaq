# Phase 4: Sequences and Control Flow - Context

**Gathered:** 2026-01-22
**Status:** Ready for planning

<domain>
## Phase Boundary

Scientists can compose multi-step sequences with moves, waits, acquire, and loops. This phase adds four new node types to the visual experiment designer: Move, Wait, Acquire, and Loop. These nodes enable sequential experiment control beyond the existing Scan node.

</domain>

<decisions>
## Implementation Decisions

### Move Node Behavior
- **Position mode:** Both absolute and relative modes supported — node has toggle to choose
- **Blocking behavior:** Configurable per node — can wait for motion completion or fire-and-forget
- **Device selection:** Claude's discretion (dropdown or autocomplete from registry)
- **Units:** Claude's discretion (likely device native units with metadata display)

### Wait/Settle Semantics
- **Wait types:** Duration AND condition-based waits supported
- **Condition modes:** Both threshold (value < X) and stability (value stable ± tolerance for N seconds)
- **Timeout:** Claude's discretion on whether required or optional (recommend required for safety)
- **Presets:** Claude's discretion on whether to include settling time presets

### Acquire Node Scope
- **Detector count:** Claude's discretion (single vs multi-detector per node)
- **Exposure setting:** Optional override per node — blank means use device's current setting
- **Frame count:** Configurable burst mode — acquire N frames in rapid succession
- **Data handling:** Both stream to live view AND save to file (save controlled by experiment settings)

### Loop Termination
- **Termination modes:** Count-based, condition-based, AND infinite (all three supported)
- **Condition expression:** Claude's discretion (same as Wait conditions is fine)
- **Iteration variable:** Claude's discretion on exposing loop counter to child nodes
- **Nesting:** Loops CAN be nested (loop inside loop for 2D patterns)

### Claude's Discretion
- Device selection UX pattern (dropdown vs autocomplete)
- Unit handling (native vs configurable)
- Timeout behavior for conditional waits (required vs optional)
- Settling time presets (include or omit)
- Single vs multi-detector Acquire node design
- Loop condition complexity level
- Whether to expose iteration variable

</decisions>

<specifics>
## Specific Ideas

No specific references or examples provided — open to standard patterns that fit the existing node graph editor UX.

</specifics>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope.

</deferred>

---

*Phase: 04-sequences-and-control-flow*
*Context gathered: 2026-01-22*
