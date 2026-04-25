---
name: asta-research
description: Search academic papers via Asta (Semantic Scholar API). Finds peer-reviewed papers, arXiv preprints, and paper snippets. Use for literature review, finding academic evidence for claims, citation network exploration, or surveying the state of research on any technical topic. Returns structured results with titles, abstracts, TLDRs, venues, and URLs.
---

# Asta Academic Research

Search peer-reviewed papers and arXiv preprints via Semantic Scholar.

**Output:** Structured academic summary with key papers, snippets, and citation insights.

## Prerequisites

- `asta` MCP server configured

**Load Asta tools:**
```
ToolSearch("select:mcp__asta__search_papers_by_relevance,mcp__asta__snippet_search,mcp__asta__get_paper,mcp__asta__search_paper_by_title,mcp__asta__get_citations,mcp__asta__get_author_papers,mcp__asta__search_authors_by_name,mcp__asta__get_paper_batch")
```

---

## Search Strategies

Choose the right tool for your goal:

| Goal | Tool | Best For |
|------|------|----------|
| Find relevant excerpts | `snippet_search` | Natural language questions, specific claims |
| Discover papers by topic | `search_papers_by_relevance` | Broad surveys, keyword-based discovery |
| Get full paper details | `get_paper` | Deep-dive a specific paper (abstract, citations, refs) |
| Find a known paper | `search_paper_by_title` | When you have the exact or partial title |
| Follow citation trails | `get_citations` | Map the research frontier from a seminal paper |
| Explore an author's work | `get_author_papers` / `search_authors_by_name` | Author-centric research |
| Batch lookup | `get_paper_batch` | Retrieve multiple papers at once by ID |

---

## Step 1: Snippet Search (start here)

The most powerful discovery tool. Uses natural language to find paper excerpts:

```
mcp__asta__snippet_search(
  query="<describe what you're looking for in natural language>",
  limit=20
)
```

**Good queries** (natural language, specific):
```
"multi-agent coding systems with deterministic quality gates and compilation feedback"
"git worktree isolation for concurrent autonomous coding agents"
"retrieval augmented generation evaluation metrics and benchmarks"
"GPU memory management for large language model inference"
```

**Bad queries** (too vague):
```
"machine learning"
"coding"
"LLMs"
```

## Step 2: Keyword Paper Search

Structured discovery with metadata:

```
mcp__asta__search_papers_by_relevance(
  keyword="<3-5 structured keywords>",
  fields="title,abstract,tldr,url,year,venue,isOpenAccess,citationCount",
  limit=20,
  publication_date_range="2023:"
)
```

**Date range formats:**
- `"2024:"` — 2024 onward
- `"2023:2025"` — 2023 through 2025
- `":2022"` — up to 2022

## Step 3: Deep-Dive Key Papers

For the most relevant papers from Steps 1-2:

```
mcp__asta__get_paper(
  paper_id="<paper_id or DOI or ARXIV:id>",
  fields="title,abstract,tldr,url,year,venue,authors,citations,references,isOpenAccess,citationCount"
)
```

**Paper ID formats:**
- Semantic Scholar ID: `"649def34f8be52c8b66281af98ae884c09aef38b"`
- DOI: `"DOI:10.18653/v1/2023.acl-long.123"`
- arXiv: `"ARXIV:2303.08774"`
- URL: `"URL:https://arxiv.org/abs/2303.08774"`

## Step 4: Follow Citation Trails

Map the research frontier from a seminal paper:

```
mcp__asta__get_citations(
  paper_id="<paper_id>",
  fields="title,abstract,year,url,tldr,citationCount",
  limit=20
)
```

**When to use:** After finding a key paper, see who built on it. High citation count on citing papers = important follow-up work.

## Step 5: Compile Results

Create a structured summary:

```markdown
# Academic Research: <Topic>

## Key Papers
1. **<Title>** (<Year>, <Venue>) — <TLDR>
   - URL: <url>
   - Citations: <count>
   - Relevance: <why this matters to the user's question>

2. ...

## Key Findings from Snippets
- <finding 1 with paper attribution>
- <finding 2>
...

## Citation Network Insights
- <paper X> is cited by N papers, suggesting <insight>
- <paper Y> cites <paper Z>, connecting <domains>

## Open Access Papers
- <links to papers available for full-text reading>
```

**Offer to save** to a local file or import into NotebookLM via `/notebooklm-manage`.

---

## Error Handling

| Problem | Solution |
|---------|----------|
| No results from snippet_search | Broaden the query; use more general terms |
| No results from keyword search | Try fewer keywords; remove date filter |
| Rate limited (429) | Wait 30s and retry; reduce `limit` parameter |
| Paper not found by ID | Try alternative ID format (DOI, arXiv, URL) |
| TLDR field empty | Use abstract instead; not all papers have TLDRs |
| Too many results | Add date range, use more specific keywords |

## Tips

- **Start with snippet_search** — it's the most flexible and often finds the best results
- **Combine approaches**: snippet_search for discovery, then get_paper for depth
- **Check open access**: filter for `isOpenAccess=true` to find freely readable papers
- **Follow citation chains**: seminal paper → get_citations → find the state of the art
- **Use with Gemini**: Asta provides academic rigor to complement Gemini's web breadth
