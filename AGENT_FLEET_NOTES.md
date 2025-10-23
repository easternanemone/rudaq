# Agent Fleet Multi-Model Orchestration

## Vision

Fleet of remote agents with various models and roles working together autonomously:
- **Models**: Haiku 4.5, gemini-flash-latest, grok-code-fast-1, Codex, etc
- **Roles**: Code architect, code writer, code reviewer, tester, documenter
- **Coordination**: Beads issue tracker + Jules/OpenCode + custom orchestrator

## Recommended Approach

### Simple Coordinator on Current Stack

Build lightweight orchestrator using existing infrastructure:

**Components:**
- **Beads** - Work queue and issue tracking
- **Jules MCP** - GitHub integration, session management
- **OpenCode Server** - Multi-model session execution
- **Zen MCP** - Direct model access (Gemini, Codex, etc)

**Workflow:**
```bash
scripts/agent-fleet.sh:
  1. Query beads for ready tasks (no blockers)
  2. Route by task type:
     - "architecture" → Gemini 2.5 Pro (via Zen)
     - "implementation" → Haiku 4.5 (via OpenCode)  
     - "review" → Codex (via Zen clink)
     - "testing" → Grok (via API)
  3. Create sessions with appropriate model
  4. Store session IDs in beads metadata
  5. Monitor completion via polling
  6. Update beads status on completion
  7. Trigger dependent tasks
```

**Benefits:**
- Multi-model orchestration ✓
- Role specialization ✓
- Autonomous collaboration ✓
- Builds on existing infrastructure ✓
- No heavyweight framework needed ✓

## Alternative: AutoGen Framework

If custom orchestration becomes too complex, consider **AutoGen (Microsoft)**:

**Pros:**
- Multi-model support (OpenAI, Anthropic, Google, xAI)
- Built-in role-based agent patterns
- Conversation and handoff management
- Active development

**Cons:**
- Python-centric (we're in Rust ecosystem)
- Adds dependency complexity
- May be overkill for our needs

## Implementation Priority

**Phase 1 (Current):**
- Manual delegation via OpenCode/Jules
- Learn patterns from daq-15, daq-16 execution
- Identify common handoff patterns

**Phase 2 (Next):**
- Simple bash/Python coordinator script
- Beads integration for task routing
- Session monitoring and status updates

**Phase 3 (Future):**
- Advanced coordination patterns
- Error recovery and retry logic
- Performance metrics and optimization

## References

- awesome-ai-agents: https://github.com/e2b-dev/awesome-ai-agents
- AutoGen: Microsoft's multi-agent framework
- Jules MCP: Session-based GitHub automation
- OpenCode: Multi-model execution server
