# Search Architecture Decision: Morph API Integration

**Date:** 2025-12-06
**Status:** DEFERRED - Keep current architecture
**Decision Type:** Architectural (Search Infrastructure)

---

## Executive Summary

**Decision:** Maintain current dual-search architecture with CocoIndex (local) + Morph MCP (Claude Code only). Defer direct Morph API integration until concrete CLI/CI/CD use cases emerge.

**Current Architecture (Verified Working):**
- ✅ **CocoIndex**: Semantic documentation search via `search_hybrid.py` (standalone, free, instant)
- ✅ **Morph Warp Grep**: Fast code search via MCP tools (Claude Code sessions only, zero-setup)

**Rationale:** Primary workflow (AI-assisted development in Claude Code) already has full access to both search engines. Direct API integration adds complexity without addressing current needs.

---

## Context

### Question Evaluated
Should we integrate the Morph API directly into `search_hybrid.py` to create a unified standalone tool, or keep the current architecture where Morph is only available via MCP tools?

### Use Cases Considered
1. **AI assistants in Claude Code sessions** (can use MCP tools directly) - ✅ Already satisfied
2. **Developers running manual CLI searches** - ⚠️ Gap exists but low priority
3. **CI/CD automation scripts** - ⚠️ Gap exists but no current need
4. **Integration into other tooling** - ⚠️ Future consideration

### Constraints
- Morph API requires authentication/credentials
- Local CocoIndex is free and instant (no auth needed)
- MCP tools within Claude Code are fast and credential-free
- rust-daq is primarily developed with AI assistance

---

## Multi-Model Consensus Analysis

Three AI models (gpt-5.1, gpt-5-pro, gpt-5-codex) evaluated the decision with different perspectives:

### 🎯 Points of Agreement (Unanimous)

1. **Pluggable architecture is essential** - Use provider interface pattern, not hardcoded logic
2. **CocoIndex must remain default** - Free, local, instant, zero-setup for all users
3. **Current MCP approach works well** - Claude Code sessions have perfect Morph access
4. **Optional configuration pattern** - Use env vars (`MORPH_API_KEY`), graceful fallbacks
5. **Avoid forcing credentials** - Many users only need local search

### ⚖️ Points of Disagreement

| Aspect | FOR (gpt-5.1) | AGAINST (gpt-5-pro) | NEUTRAL (gpt-5-codex) |
|--------|---------------|---------------------|----------------------|
| **Timing** | Implement now (0.5-1 day MVP) | Wait for demand (2-3 weeks production) | Lightweight module now |
| **Value** | CLI/CI gaps are critical | Current use case satisfied | Optional for automation |
| **Risk** | Manageable maintenance | Cost/complexity outweighs benefit | Modular isolation reduces risk |
| **Confidence** | 9/10 | 8/10 | 7/10 |

---

## Recommended Approach: Phased Implementation

### Phase 1: Foundation (Do Now - ~1 day)

**Goal:** Add provider interface without changing functionality

```python
# In search_hybrid.py

from abc import ABC, abstractmethod

class SearchProvider(ABC):
    """Abstract base class for search backends."""

    @abstractmethod
    def search(self, query: str, limit: int) -> list[SearchResult]:
        """Execute search and return results."""
        pass

class CocoIndexProvider(SearchProvider):
    """Local semantic search via CocoIndex."""

    def search(self, query: str, limit: int) -> list[SearchResult]:
        # Existing CocoIndex logic
        pass

class MorphProvider(SearchProvider):
    """Remote code search via Morph API (optional)."""

    def __init__(self, api_key: str):
        if not api_key:
            raise ValueError("MORPH_API_KEY required")
        self.api_key = api_key

    def search(self, query: str, limit: int) -> list[SearchResult]:
        raise NotImplementedError("Morph API integration not yet implemented")

def get_search_provider() -> SearchProvider:
    """Select search provider based on configuration."""
    morph_key = os.getenv('MORPH_API_KEY')

    if morph_key:
        try:
            return MorphProvider(morph_key)
        except NotImplementedError:
            print("⚠️  Morph API not yet implemented, falling back to CocoIndex")
            return CocoIndexProvider()

    return CocoIndexProvider()  # Default
```

**Benefits:**
- Zero risk (no behavior change)
- Improves architecture
- Enables future extensibility
- Clear migration path

### Phase 2: Optional Morph Module (Implement When Needed - ~2-3 days)

**Trigger Conditions (any of):**
- You personally need Morph in CLI/CI workflows
- External contributor requests feature
- CI/CD automation requires code search

**Implementation:**

Create separate module: `scripts/search_providers/morph_client.py`

```python
import os
import requests
from typing import List, Optional
from .base import SearchProvider, SearchResult

class MorphAPIClient:
    """Thin wrapper for Morph API with retries and fallbacks."""

    def __init__(self, api_key: str, endpoint: str = "https://api.morph.so"):
        self.api_key = api_key
        self.endpoint = endpoint
        self.session = requests.Session()
        self.session.headers.update({
            'Authorization': f'Bearer {api_key}',
            'Content-Type': 'application/json'
        })

    def search(self, query: str, repo_path: str, limit: int = 10) -> dict:
        """Execute Morph warp grep search."""
        # TODO: Add retries, timeout, rate limiting
        # TODO: Add cost tracking/budgeting
        # TODO: Add result caching
        pass

class MorphProvider(SearchProvider):
    """Morph API provider with CocoIndex fallback."""

    def __init__(self, api_key: Optional[str] = None):
        self.api_key = api_key or os.getenv('MORPH_API_KEY')
        if not self.api_key:
            raise ValueError("MORPH_API_KEY environment variable required")

        self.client = MorphAPIClient(self.api_key)
        self.fallback = CocoIndexProvider()

    def search(self, query: str, limit: int) -> List[SearchResult]:
        try:
            results = self.client.search(query, "/path/to/repo", limit)
            return self._parse_results(results)
        except Exception as e:
            print(f"⚠️  Morph API error: {e}, falling back to CocoIndex")
            return self.fallback.search(query, limit)
```

**Key Features:**
- Graceful degradation to CocoIndex on errors
- Cost controls (`--max-morph-queries` flag)
- Caching to reduce API calls
- Clear error messages
- Unit tests with mocked API responses

### Phase 3: Documentation & Hardening (If Phase 2 Implemented)

1. **README documentation:**
   - How to enable Morph (env vars, credentials)
   - Cost estimation guide
   - Fallback behavior explanation

2. **CI/CD integration guide:**
   - Secret management best practices
   - Cost budgeting in pipelines
   - Example GitHub Actions workflow

3. **Testing:**
   - Unit tests with mocked Morph API
   - Integration tests (gated behind `MORPH_INTEGRATION_TEST=1`)
   - Response fixtures for deterministic testing

4. **Monitoring:**
   - API call tracking
   - Cost reporting
   - Error rate monitoring

---

## Decision Matrix

| Criterion | Status Quo<br/>(MCP-only) | Optional Module<br/>(Recommended) | Full Integration<br/>(Not recommended) |
|-----------|---------------------------|-----------------------------------|----------------------------------------|
| **Claude Code** | ✅ Perfect | ✅ Same | ✅ Same |
| **CLI Usage** | ❌ Missing | ✅ Optional | ✅ Always available |
| **CI/CD** | ❌ Missing | ✅ Opt-in | ✅ Always available |
| **Setup Burden** | ✅ Zero | ⚠️ Optional (env var) | ❌ Required for all |
| **Maintenance** | ✅ Minimal | ⚠️ Moderate | ❌ High |
| **Cost Risk** | ✅ None | ⚠️ Controlled (opt-in) | ❌ Uncontrolled |
| **Complexity** | ✅ Simple | ⚠️ Isolated module | ❌ Core script bloat |
| **Primary Use Case** | ✅ Fully satisfied | ✅ Same | ✅ Same |

**Score:** Optional Module = 6/8 ✅ | Status Quo = 5/8 | Full Integration = 3/8

---

## Critical Risks & Mitigations

### If Morph API Integration Proceeds (Phase 2):

1. **Risk: Cost Unpredictability in CI/CD**
   - **Impact:** Unexpected API bills from automated searches
   - **Mitigation:**
     - Require explicit `--enable-morph` flag in CI
     - Add `--max-morph-queries=N` budget control
     - Log all API calls with cost estimates
     - Document cost per query

2. **Risk: Credential Management Complexity**
   - **Impact:** Security issues, difficult setup, support burden
   - **Mitigation:**
     - Support multiple auth methods (env var, file, secret store)
     - Clear documentation with examples
     - Validate credentials on startup, fail fast
     - Never log credentials

3. **Risk: Maintenance Burden from API Changes**
   - **Impact:** Breaking changes require urgent fixes
   - **Mitigation:**
     - Keep client thin and isolated
     - Pin API versions
     - Comprehensive error handling
     - Always fall back to CocoIndex

4. **Risk: Testing Complexity**
   - **Impact:** Flaky tests, hard to reproduce failures
   - **Mitigation:**
     - Mock all API calls in unit tests
     - Use VCR.py for recording real responses
     - Separate integration tests (opt-in)
     - Test offline mode thoroughly

5. **Risk: User Confusion**
   - **Impact:** "Why doesn't Morph work?" support requests
   - **Mitigation:**
     - Crystal-clear error messages
     - Automatic fallback with warning
     - FAQ in documentation
     - Troubleshooting guide

---

## Verification Results (2025-12-06)

Both search systems verified working:

### ✅ CocoIndex (Comprehensive Mode)
- **Query:** "PVCAM unsafe blocks and safety patterns"
- **Results:** 10 relevant documents in 1.2 seconds
- **Top matches:**
  - PVCAM Driver Validation Checklist (49.3% similarity)
  - PVCAM Hardware Validation Report (45.3%)
  - PVCAM Camera Operator Guide (44.9%)
- **Status:** Standalone Python script works perfectly

### ✅ Morph Warp Grep (Quick Mode via MCP)
- **Query:** "PVCAM unsafe blocks"
- **Results:** Found all unsafe blocks in `src/hardware/pvcam.rs`
  - Lines 259-271: FFI error retrieval
  - Lines 346-453: Camera initialization
  - Lines 610-715: Frame acquisition
  - Lines 1020-1100: Parameter queries
- **Status:** MCP integration in Claude Code works perfectly

### ⚠️ Morph via Python Script
- **Status:** Correctly shows "MCP integration required" message
- **Expected:** Python script cannot access MCP servers (by design)

---

## Industry Perspective

Common pattern in developer tools (Sourcegraph, Terraform, kubectl):
- **Core CLI:** Offline, deterministic, free, works everywhere
- **Cloud/Premium Features:** Optional plugins/adapters with explicit opt-in
- **Examples:**
  - Terraform: Local state (default) vs. Terraform Cloud (optional)
  - kubectl: Local cluster (default) vs. Cloud provider auth (optional)
  - OpenAI SDK: Works without API key (errors clearly), enhanced with key

**Best Practice:** Separation of concerns keeps free tier simple while allowing paid features via configuration.

---

## Final Recommendation

### ✅ DO NOW
1. **Keep current architecture** - Both search systems verified working
2. **Document in README** - Explain CocoIndex (standalone) vs. Morph (Claude Code MCP only)
3. **Consider Phase 1** - Add SearchProvider interface if time permits (~1 hour)

### ⏸️ DEFER
4. **Phase 2 implementation** - Only if concrete CLI/CI/CD needs emerge
5. **Morph API client** - No current use case justifies complexity
6. **Cost controls & monitoring** - Not needed without API integration

### ❌ DO NOT
7. **Force Morph on all users** - Preserves zero-setup local search
8. **Embed API logic in core script** - Keep it modular if implemented
9. **Implement without use case** - YAGNI principle applies

---

## Review Triggers

Re-evaluate this decision if:

1. **User request:** Someone asks for Morph in CLI/CI workflows
2. **Workflow change:** Development moves away from Claude Code
3. **CI/CD need:** Automated code search becomes valuable
4. **3+ months:** No requests = decision validated
5. **External integration:** Third-party tool needs search API

---

## Related Documentation

- **Implementation:** `scripts/search_hybrid.py` (Python search orchestrator)
- **CocoIndex Setup:** `docs/HYBRID_SEARCH_SETUP.md` (semantic doc search)
- **Morph Setup:** `docs/MORPH_SETUP_COMPLETE.md` (MCP integration)
- **Architecture:** `CLAUDE.md` (project instructions)

---

## Notes

- **Search hybrid.py already has placeholder** for Morph integration (lines 114-133)
- **Auto-detection logic works** - Correctly routes queries to comprehensive vs. quick mode
- **MCP tools are fast** - Morph searches complete in ~1-2 seconds via Claude Code
- **No urgency** - Current setup serves all active workflows perfectly

---

**Conclusion:** The verification confirms both systems work excellently in their current roles. There's no compelling reason to add complexity until actual CLI/CI/CD use cases emerge. The recommended phased approach allows future expansion without premature optimization.
