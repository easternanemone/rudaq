# V2/V4 Coexistence Architecture Documentation

## Quick Navigation

Welcome to the V2/V4 coexistence architecture documentation. This folder contains all design documents, implementation roadmaps, and risk analysis for the gradual migration from V2 to V4 during Phases 1E-3.

### Document Index

#### 1. START HERE for Phase 1E Implementation

**For Developers Starting Phase 1E Work:**

**File:** `PHASE_1E_USAGE_SUMMARY.md` (8 pages, 382 lines)
**Purpose:** Quick reference guide for Phase 1E component usage
**Time to Read:** 10-15 minutes
**Start if:** You need to implement Phase 1E components NOW

**Contains:**
- One-minute overview
- Quick start guides for each component
- Code examples by use case
- Error handling patterns
- Best practices (critical)
- Troubleshooting quick reference
- Performance expectations

**Then Read:** `PHASE_1E_IMPLEMENTATION_GUIDE.md` (39 pages, 1371 lines)
**Purpose:** Complete implementation guide with detailed examples
**Time to Read:** 45-60 minutes (detailed reference)
**Read if:** You need detailed usage patterns, migration guide, or troubleshooting

**Contains:**
- Complete component architecture
- Detailed DualRuntimeManager usage
- Detailed SharedSerialPort usage
- Detailed VisaSessionManager usage
- Integration patterns
- Full V2-to-coexistence migration guide
- Extended troubleshooting
- Performance analysis

---

#### 2. Executive Summary
**File:** `COEXISTENCE_SUMMARY.md` (4 pages)
**Purpose:** High-level overview of the entire architecture
**Audience:** Everyone (stakeholders, team leads, developers)
**Time to Read:** 15 minutes

**Contains:**
- Problem statement
- Architecture solution overview
- Critical design decisions
- Implementation phases
- Key risks and mitigations
- Next steps

---

#### 2. Complete Architecture Design
**File:** `V2_V4_COEXISTENCE_DESIGN.md` (12 pages)
**Purpose:** Comprehensive technical architecture
**Audience:** Architects, senior developers, tech leads
**Time to Read:** 45 minutes

**Sections:**
- Executive summary
- Architecture overview with diagrams
- Core design principles
- Detailed coexistence strategy
- Actor spawning & supervision
- Message passing & data flow
- Resource sharing strategy
- Shutdown & lifecycle
- Data flow & integration points
- Migration path with per-instrument checklist
- Configuration management
- Risks & mitigations
- Testing strategy
- Code structure and examples

**When to Read:**
- Before starting Phase 1E implementation
- When making architectural decisions
- For understanding resource sharing approach
- For migration planning

---

#### 3. Implementation Roadmap
**File:** `IMPLEMENTATION_ROADMAP.md` (10 pages)
**Purpose:** Concrete, actionable implementation tasks
**Audience:** Implementation teams, project managers
**Time to Read:** 30 minutes

**Sections:**
- Phase 1E: Core infrastructure (7 tasks, 2-3 weeks)
  - DualRuntimeManager
  - SharedSerialPort
  - VisaSessionManager
  - Shared resources
  - Configuration system
  - Measurement bridge
  - Integration tests
- Phase 1F: Instrument migration (5 tasks, 3-4 weeks)
  - Newport1830C review & testing
  - Elliptec V4 implementation
  - Configuration tools
  - Shadow mode testing
- Phase 2: Production stability (3 tasks, 2-3 weeks)
  - Stress testing
  - Documentation
  - User communication
- Phase 3: V2 cleanup (2 tasks, spread over 4-6 months)

**For Each Task:**
- Effort estimate
- Dependencies
- Deliverables
- Acceptance criteria
- Code structure examples

**Dependency Graph** shows how tasks relate

**When to Read:**
- For sprint planning
- For task assignment
- To understand timeline
- For progress tracking

---

#### 4. Risks & Mitigations
**File:** `RISKS_AND_BLOCKERS.md` (11 pages)
**Purpose:** Identify and mitigate risks
**Audience:** Risk managers, architects, leads
**Time to Read:** 30 minutes

**Risk Categories:**
- **CRITICAL Risks (3)**
  - Hardware resource deadlock
  - VISA single-session limitation
  - Data corruption from concurrent storage
- **HIGH Risks (3)**
  - Complex shutdown sequence
  - Serial port driver instability
  - Memory/CPU contention
- **MEDIUM Risks (3)**
  - Configuration complexity
  - Measurement format incompatibility
  - Version compatibility issues

**For Each Risk:**
- Detailed description
- Probability & impact
- Mitigation strategy with code examples
- Verification checklist
- Testing approach

**Additional Content:**
- Risk monitoring procedures
- Escalation paths
- Contingency plans
- Risk scoring methodology

**When to Read:**
- During Phase 1E planning
- Before critical decisions
- When designing shared resources
- For risk gate reviews

---

#### 5. Immediate Blockers & Decisions
**File:** `IMMEDIATE_BLOCKERS.md` (7 pages)
**Purpose:** Identify and resolve blockers before Phase 1E
**Audience:** Architecture review, team leads
**Time to Read:** 20 minutes

**Content:**
- **Critical Blockers (4)** with action items
  - Kameo actor lifecycle integration
  - Tokio + Kameo runtime coexistence
  - Shared resource contention testing
  - VISA SDK licensing & installation
- **Major Open Questions (4)** requiring decisions
  - HDF5 file sharing approach
  - GUI architecture (merged vs separate)
  - Instrument ID conflict handling
  - Backward compatibility requirements
- **Medium-Priority Unknowns (3)**
- **Phase 1E Pre-Requisites** checklist
- **Decision timeline** and authority
- **Risk from unknowns** and mitigations
- **Early testing spike** recommendation

**When to Read:**
- Before Phase 1E starts
- For architecture review meeting
- To understand what needs resolution

---

#### 6. This File: Navigation Guide
**File:** `README.md` (you are here)

---

## Recommended Reading Order

### For Project Managers
1. COEXISTENCE_SUMMARY.md (20 min)
2. IMPLEMENTATION_ROADMAP.md (30 min)
3. RISKS_AND_BLOCKERS.md - Risk summary section (10 min)

**Total: ~1 hour**

### For Architects & Tech Leads
1. COEXISTENCE_SUMMARY.md (15 min)
2. V2_V4_COEXISTENCE_DESIGN.md (45 min)
3. RISKS_AND_BLOCKERS.md (30 min)
4. IMMEDIATE_BLOCKERS.md (20 min)

**Total: ~2 hours** (this is the architecture review)

### For Developers (Implementation Team)
1. COEXISTENCE_SUMMARY.md (15 min)
2. V2_V4_COEXISTENCE_DESIGN.md - Relevant sections (20 min)
3. IMPLEMENTATION_ROADMAP.md - Your phase (20 min)
4. RISKS_AND_BLOCKERS.md - Relevant risks (15 min)

**Total: ~1 hour** (per phase)

### For New Team Members
1. COEXISTENCE_SUMMARY.md (15 min)
2. V2_V4_COEXISTENCE_DESIGN.md - Architecture overview (20 min)
3. IMPLEMENTATION_ROADMAP.md - Current phase (20 min)

**Total: ~1 hour**

---

## Key Documents Referenced

### V4 Architecture
- `ARCHITECTURE.md` - V4 overview and core technologies
- `src/lib.rs` - V4 module structure

### V2 Architecture
- `src/app_actor.rs` - DaqManagerActor (V2 supervisor)
- `src/actors/mod.rs` - V4 actor structure
- `src/instrument/registry_v2.rs` - V2 instrument registration

### Configuration
- `src/config_v4.rs` - V4 configuration system (Figment-based)
- Configuration files: `config.v2.toml`, `config.v4.toml` (example)

---

## Quick Facts

| Aspect | Detail |
|--------|--------|
| **Total Design Time** | ~2226 lines across 4 documents |
| **Phases Covered** | 1E (2-3 weeks), 1F (3-4 weeks), 2 (2-3 weeks), 3 (4-6 months) |
| **Instruments Designed** | SCPI (Phase 1D), ESP300 (Phase 1D), PVCAM (Phase 1D), Newport (Phase 1F), Elliptec (Phase 1F) |
| **Critical Risks** | 3 (deadlock, VISA limitation, data corruption) |
| **Phase 1E Tasks** | 7 core infrastructure tasks |
| **Success Metric** | V4 production-ready by end of Phase 2 |

---

## Architecture At A Glance

```
Dual Independent Subsystems
│
├── V2 (tokio)          V4 (Kameo)
│   ├── DaqManager       ├── InstrMgr
│   ├── V2 Instr         ├── V4 Instr
│   └── DataDist         └── DataPub
│
Shared Resources (Arc<Mutex<>>)
├── SerialPortPool
├── VisaSessionManager
└── DeviceCache
```

---

## Design Principles

1. **Independence**: Separate actor systems, no direct coupling
2. **Shared Resources**: Type-safe exclusive access with Arc<Mutex<>>
3. **Message Passing**: Data flows through channels
4. **Graceful Shutdown**: Ordered with timeouts
5. **Conservative Initially**: Optimize after Phase 2 validation

---

## Timeline Summary

- **Phase 1E** (2-3 weeks): Core infrastructure
- **Phase 1F** (3-4 weeks): Per-instrument migration
- **Phase 2** (2-3 weeks): Production validation
- **Phase 3** (4-6 months): V2 deprecation & cleanup

**Total: 8-12 weeks to Phase 2 readiness**

---

## Getting Help

### For Questions About...

**Architecture & Design:**
- See: V2_V4_COEXISTENCE_DESIGN.md
- Contact: Architecture review team

**Implementation Tasks:**
- See: IMPLEMENTATION_ROADMAP.md
- Contact: Phase 1E task owner

**Risks & Concerns:**
- See: RISKS_AND_BLOCKERS.md
- Contact: Risk manager

**Blockers & Decisions:**
- See: IMMEDIATE_BLOCKERS.md
- Contact: Architecture review

**Specific Code Examples:**
- See: V2_V4_COEXISTENCE_DESIGN.md sections 9 & 10
- Location: `/src/dual_runtime/` (to be implemented)

---

## Document Status

| Document | Status | Audience | Updated | Lines |
|----------|--------|----------|---------|-------|
| PHASE_1E_USAGE_SUMMARY.md | NEW | Developers | 2025-11-17 | 382 |
| PHASE_1E_IMPLEMENTATION_GUIDE.md | NEW | Developers | 2025-11-17 | 1,371 |
| COEXISTENCE_SUMMARY.md | Complete | Everyone | 2025-11-17 | 503 |
| V2_V4_COEXISTENCE_DESIGN.md | Complete | Architects | 2025-11-17 | 847 |
| IMPLEMENTATION_ROADMAP.md | Complete | Developers | 2025-11-17 | 709 |
| RISKS_AND_BLOCKERS.md | Complete | Risk mgmt | 2025-11-17 | 670 |
| IMMEDIATE_BLOCKERS.md | Complete | Arch review | 2025-11-17 | ? |
| README.md | Updated | Everyone | 2025-11-17 | 378 |
| **TOTAL** | **8 docs** | **All** | **2025-11-17** | **4,860+** |

---

## Next Steps After Reading

### Immediately (This Week)
1. [ ] Architecture review team reads all documents
2. [ ] Identify any design issues or questions
3. [ ] Schedule 1-hour review meeting
4. [ ] Resolve blockers identified in IMMEDIATE_BLOCKERS.md

### For Phase 1E Planning (Next Week)
1. [ ] Create Beads issues for Phase 1E tasks
2. [ ] Assign task ownership
3. [ ] Set sprint goal: Infrastructure complete
4. [ ] Create test harness for contention testing

### Parallel Work
1. [ ] Research Kameo lifecycle/shutdown semantics
2. [ ] Audit dependency compatibility
3. [ ] Prepare team training materials
4. [ ] Plan user communication timeline

---

## Appendix: File Locations

All documents in: `/Users/briansquires/code/rust-daq/v4-daq/docs/architecture/`

```
docs/architecture/
├── README.md (this file)
├── COEXISTENCE_SUMMARY.md
├── V2_V4_COEXISTENCE_DESIGN.md
├── IMPLEMENTATION_ROADMAP.md
├── RISKS_AND_BLOCKERS.md
└── IMMEDIATE_BLOCKERS.md
```

Related project files:
- V4 actors: `src/actors/`
- V4 config: `src/config_v4.rs`
- V2 core: `src/app_actor.rs`
- To be implemented: `src/dual_runtime/`

---

**Architecture Design Status:** COMPLETE - Ready for Implementation
**Approval Status:** AWAITING ARCHITECTURE REVIEW
**Last Updated:** 2025-11-17
**Owner:** System Architecture Designer

---

*For questions or clarifications, refer to the specific document sections or contact the architecture review team.*
