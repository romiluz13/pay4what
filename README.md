# pay4what

**See what each feature cost you in Claude Code — token spend per activity, not per session.**

`pay4what` reads Claude Code's local JSONL transcripts, computes cost per turn (cache-aware), segments by user request, and uses an LLM to categorize every segment by development activity — so you see *"the OAuth feature cost $47, the login bugfix cost $3"* instead of *"Edit tool: $74"*.

```
$ pay4what --since 7d
  ┌──────────────────────────────────────────────┬──────────┬────────┐
  │ Activity                                      │ Cost     │ Tokens │
  ├──────────────────────────────────────────────┼──────────┼────────┤
  │ 🚀 feature   OAuth refresh-token rotation      │  $47.20  │  3.1M  │
  │ 🐛 bugfix     login redirect loop              │   $3.40  │  220K  │
  │ 📦 migration  Prisma 5 → 6 schema bump         │  $12.80  │  880K  │
  │ ♻️  refactor   extract billing service          │   $6.10  │  410K  │
  │ ❓ unattributed (small / interrupted)           │   $2.60  │  175K  │
  ├──────────────────────────────────────────────┼──────────┼────────┤
  │ TOTAL                                         │  $72.10  │ 4.9M   │
  └──────────────────────────────────────────────┴──────────┴────────┘
  💸 1 feature = 65% of the week's spend.
```

## Why

The industry is shouting pay4what's exact question — *"Token Billing Exposes AI's Missing ROI"* (Forbes). 78% of companies hit surprise AI bills (Beri); Uber burned their 2026 AI budget in 4 months. Existing tools tell you **how much** you spent, or **which tool** ran. None tell you **what you shipped and what it cost**.

> **ccusage tells you HOW MUCH. CodeBurn tells you WHICH TOOL. pay4what tells you WHAT YOU SHIPPED and what it cost.**

The unit of attribution is a **shippable artifact** (a feature/PR/ticket), not a tool or a session. A 90-minute Edit/Bash/Edit retry loop becomes *"the auth migration that ate $47"* — not *"Edit tool calls."*

## Install

```bash
# cargo
cargo install pay4what

# npx (planned)
npx pay4what --since 7d

# brew (planned)
brew install pay4what
```

## Usage

```bash
# fast cost totals (no API key, deterministic rules categorizer)
pay4what --since 7d --no-llm

# LLM-categorized (sharpens unattributed from ~60% → ~25%)
export GROVE_API_KEY=...        # or OPENROUTER_API_KEY
pay4what --since 7d

# also show cost-by-file
pay4what --since 7d --files

# JSON output
pay4what --since 2026-07-01 --format json
```

**Zero-config:** no account, no config file, reads only local files (`~/.claude/projects/`). The LLM categorizer is optional — without a key, pay4what falls back to deterministic rules.

## vs ccusage

[ccusage](https://github.com/ccusage/ccusage) (16.9k★, Rust) is the viral zero-config Claude Code cost monitor. pay4what shares its transcript-discovery + tolerant-parse + pricing approach, but **ccusage reports per-session/per-project totals only** — it does not categorize spend by activity, does not touch git, and does not attribute cost to features/PRs/tickets. If you want *"how much did I spend today?"*, use ccusage. If you want *"what did that spend buy me?"*, use pay4what.

## vs CodeBurn

[CodeBurn](https://github.com/getagentseal/codeburn) (8.5k★, TypeScript) is the closest competitor — it categorizes by 13 activity *types* using **deterministic regex + tool-set heuristics** (explicitly "No LLM calls, fully deterministic"). It never inspects tool arguments, edited files, or what the assistant actually did — and its keyword-order regex misclassifies ambiguous prompts (the [documented #196 bug](https://github.com/getagentseal/codeburn/issues/196): *"add error handling"* tagged as Debugging because DEBUG regex checks before FEATURE). pay4what's LLM categorizer reads the full segment context (user message + tool verbs + touched files + branch) and closes exactly that gap. CodeBurn's `yield` command binds session→commit by SHA (fragile under squash/rebase); pay4what binds to PR/branch with file-footprint attribution (v1.1).

| | ccusage | CodeBurn | **pay4what** |
|---|---|---|---|
| Per-session cost | ✅ | ✅ | ✅ |
| Activity categorization | ❌ | ✅ deterministic regex | ✅ LLM (rules fallback) |
| Inspects tool args / edited files | ❌ | ❌ | ✅ |
| Cost per named feature/PR/ticket | ❌ | ❌ | ✅ (v1.1 commit/issue) |
| Subagent spend separation | ❌ | ✅ | ✅ |
| Cross-vendor (Cursor/Aider/Codex) | ❌ | partial | planned |

## How it works

```
~/.claude/projects/<enc-cwd>/<uuid>.jsonl
        │
   discover → parse (tolerant) → cost (cache-aware) → segment (user-turns)
        │                                              → categorize (LLM or rules)
        │                                              → file attribution
        ▼
   cost-by-activity table + cost-by-file table
```

- **Discovery:** resolves `CLAUDE_CONFIG_DIRS` + `~/.claude/projects`, finds top-level sessions **and** subagent transcripts (`<uuid>/subagents/agent-*.jsonl`). No double-count — parent and subagent are physically separate files.
- **Tolerant parse:** streams JSONL, handles the volatile schema (Anthropic #53516), captures `input_tokens` / `cache_read_input_tokens` / `cache_creation_input_tokens` **separately** (never double-counts), including the 5m/1h cache-creation split.
- **Cache-aware cost:** prices each token bucket at its own rate. 1h cache-creation priced at `2.0× input` (per Anthropic's pricing model). Bundled pricing table is versioned + `asOf`-dated, sourced from [docs.claude.com/pricing](https://docs.claude.com/en/docs/about-claude/pricing).
- **Chunked-turn dedup:** Claude Code emits one logical turn as multiple JSONL lines (thinking + text + tool_use) sharing cumulative usage. pay4what counts each logical turn once (verified: a 27K-turn session had 10,648 usage lines → 4,998 logical turns; without dedup, 113% inflation).
- **Segmentation:** splits at user-turn boundaries (the unit of categorization), with compact boundaries.
- **Categorization:** LLM-primary (DeepSeek V4 Flash via OpenRouter by default — any OpenAI-compatible gateway works). Batches 20 segments/call. Falls back to deterministic rules (branch name + keywords + tool verbs) when no API key is set.
- **File attribution:** from Edit/Write tool inputs; memory/session bookkeeping files excluded.

## Categorization: LLM-primary, rules fallback

- **With `OPENROUTER_API_KEY` (or `GROVE_API_KEY`):** every segment goes through the LLM with full context. ~$0.005/session at DeepSeek V4 Flash rates. In practice, unattributed drops from ~60% (rules) to ~25% (LLM).
- **Without a key (`--no-llm` or no env):** deterministic rules categorize via branch name (`feat/`→feature, `fix/`→bugfix), user-message keywords, and tool verbs. Instant, zero-cost, ~60% unattributed.

## Supported providers

| Provider | Env | Default model |
|---|---|---|
| OpenRouter (public default) | `OPENROUTER_API_KEY` | `deepseek/deepseek-v4-flash` |
| Any OpenAI-compatible gateway | `GROVE_API_KEY` + `GROVE_BASE_URL` | `DeepSeek-V4-Flash` |
| Rules fallback | (none) | — |

## Limitations (honest)

- **Claude Code first.** Cursor per-turn tokens aren't locally accessible (marked "limited"). Codex/Aider/Cline formats are roadmap.
- **cost-by-commit / cost-by-issue are v1.1.** v1.0 ships cost-by-activity + cost-by-file (high confidence). Commit attribution is probabilistic (binds to PR/branch, never SHA — SHAs are fragile under squash/rebase).
- **Pricing is a bundled snapshot.** Verify against `docs.claude.com/pricing` before publishing dollar claims (pay4what's rates are `asOf`-dated for this reason).
- **Categorization accuracy is not 100%.** pay4what shows an `unattributed` bucket rather than faking precision.

## License

MIT.
