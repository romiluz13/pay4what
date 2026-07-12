# pay4what

![CI](https://github.com/romiluz13/pay4what/actions/workflows/ci.yml/badge.svg)
![crates.io](https://img.shields.io/crates/v/pay4what.svg)
![license](https://img.shields.io/badge/license-MIT-blue.svg)

**You spent $3,000 on Claude Code last month. What did you actually ship?**

`pay4what` tells you exactly what each feature, bugfix, and refactor cost you — not "you used 47M tokens" or "Edit tool: $74." It reads your local Claude Code transcripts, computes real costs (cache-aware), and uses an LLM to categorize every segment of work by what you actually **did**.

![pay4what demo](pay4what-demo.gif)

## The problem

You open your AI bill and see **$3,000**. No breakdown by feature. No idea which task burned the most. Was it the auth migration? The debugging spiral? The 2-hour exploration that went nowhere?

Existing tools tell you **how much** you spent (ccusage) or **which tool** ran (CodeBurn). None tell you **what you shipped and what it cost**. You're left guessing whether that 3-day debugging session was worth $400 or $4.

78% of companies exceed their AI budgets. Uber burned their entire 2026 AI budget in 4 months. The money is spent — the question is **where it went and whether it was worth it**.

## The solution

pay4what attributes every dollar of token spend to a **development activity** — feature, bugfix, migration, refactor, debugging, exploration, planning — not a tool name or a session ID.

**How it works:**

1. **Reads** your local Claude Code transcripts (`~/.claude/projects/`) — nothing leaves your machine
2. **Computes** real cost per turn, cache-aware (separates cache reads from fresh input, prices each at its own rate)
3. **Segments** work by user request — the unit of development, not the unit of API calls
4. **Categorizes** each segment with an LLM that reads the full context (user message + tool verbs + files touched + branch + assistant replies) — not regex guessing
5. **Persists** rich records `{activity, tags, summary, cost}` into a local bucket store
6. **Answers questions**: "how much did the login bug cost?" → instant dollar sum, no re-processing

## The value

- **See where the money goes**: "65% of this week's spend was one feature" — the insight that changes how you build
- **Query by topic**: `pay4what query "auth"` → $47 across 12 segments, instantly
- **Zero config**: reads only local files, no account, no API key required for basic totals
- **Sharp categorization**: LLM reads what you actually did, not just keywords. Unattributed drops from ~60% (rules) to ~25% (LLM)
- **Incremental**: classifies new segments only. Second run is instant
- **Private**: your transcripts never leave your machine. The LLM categorizer is optional and uses your own API key

## Install

```bash
cargo install pay4what
```

## Usage

```bash
# fast cost totals (no API key needed — deterministic rules categorizer)
pay4what --since 7d --no-llm

# LLM-categorized (sharpens unattributed from ~60% → ~25%)
export OPENROUTER_API_KEY=...        # or any OpenAI-compatible gateway
pay4what --since 7d

# query the bucket store (instant, no re-reading sessions)
pay4what query "auth"

# also show cost-by-file
pay4what --since 7d --files
```

## vs other tools

| | ccusage | CodeBurn | **pay4what** |
| --- | --- | --- | --- |
| What it tells you | How much you spent | Which tool ran | **What you shipped and what it cost** |
| Activity categorization | ❌ | regex only | ✅ LLM (reads full context) |
| Inspects tool args / edited files | ❌ | ❌ | ✅ |
| Query by topic ("how much did auth cost?") | ❌ | ❌ | ✅ |
| Subagent spend separation | ❌ | ✅ | ✅ |
| Private (local files only) | ✅ | ✅ | ✅ |

> **ccusage** (16.9k★) is excellent for "how much did I spend today?" **CodeBurn** (8.5k★) categorizes by tool type with deterministic regex. **pay4what** is the only tool that reads what you actually did and tells you what it cost — per feature, per bugfix, per refactor.

## Categorization

8 activities: `feature` · `bugfix` · `migration` · `refactor` · `debugging` · `exploration` · `planning` · `unattributed`

**With an API key:** every segment goes through the LLM with full context (user message + tools + files + branch + assistant text). ~$0.005/session at DeepSeek V4 Flash rates. Falls back to rules on any error or timeout — never blocks.

**Without a key:** deterministic rules categorize via branch name (`feat/`→feature, `fix/`→bugfix), user-message keywords, and tool verbs. Instant, zero-cost, ~60% unattributed.

| Provider | Env | Default model |
| --- | --- | --- |
| OpenRouter | `OPENROUTER_API_KEY` | `deepseek/deepseek-v4-flash` |
| Any OpenAI-compatible gateway | `GROVE_API_KEY` + `GROVE_BASE_URL` | `DeepSeek-V4-Flash` |
| Rules fallback | (none) | — |

## Limitations

- **Claude Code only.** Reads `~/.claude/projects/` transcripts. Cursor, Codex, and Aider support are on the roadmap.
- **Cost-by-commit / cost-by-PR are v1.1.** v1.0 ships cost-by-activity + cost-by-file. Commit attribution binds to branch (not SHA — SHAs are fragile under squash/rebase).
- **Pricing is a bundled snapshot** (dated 2026-07-08, byte-verified against LiteLLM). Verify against [docs.claude.com/pricing](https://docs.claude.com/en/docs/about-claude/pricing) before publishing dollar claims.
- **Categorization is not 100%.** pay4what shows an `unattributed` bucket rather than faking precision.

## Technical details

- **Cache-aware cost**: prices each token bucket at its own rate (input, output, cache_read, cache_creation 5m/1h). 1h cache-creation priced at 2.0× input per Anthropic's model. Pricing byte-verified against LiteLLM.
- **Chunked-turn dedup**: Claude Code emits one logical turn as multiple JSONL lines (thinking + text + tool_use) sharing cumulative usage. pay4what counts each logical turn once — without this, costs are inflated ~2×.
- **Subagent spend**: discovers and separates subagent transcripts (`<uuid>/subagents/agent-*.jsonl`) — no double-counting between parent and child.
- **Partial-result handling**: when the LLM returns fewer records than segments (e.g. 18 for 20), pay4what uses the 18 that parsed and fills gaps with rules — never discards good data.

## License

MIT.
