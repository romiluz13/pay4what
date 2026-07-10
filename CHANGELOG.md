# Changelog

All notable changes to pay4what will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-08

### Added
- Initial release: token-spend-per-activity attribution for Claude Code.
- Discovery of top-level sessions + subagent transcripts (`~/.claude/projects/<enc-cwd>/<uuid>.jsonl` and `<uuid>/subagents/agent-*.jsonl`).
- Tolerant JSONL parser (handles volatile schema, Anthropic #53516); captures `input_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens` (incl. 5m/1h split) as separate buckets.
- Cache-aware cost computation: 1h cache-creation priced at `2.0× input` per Anthropic's pricing model. Bundled versioned pricing table (`asOf`-dated, sourced from docs.claude.com/pricing).
- Chunked-turn dedup: Claude Code emits one logical turn as multiple JSONL lines sharing cumulative usage; pay4what counts each logical turn once (verified: 113% inflation without dedup).
- Segmentation at user-turn boundaries (the unit of categorization).
- LLM-primary categorization (DeepSeek V4 Flash via OpenRouter by default; any OpenAI-compatible gateway works). Rules-pre-tagging sends only unattributed segments to the LLM (cuts calls ~5-10×). Falls back to deterministic rules when no API key is set.
- Two render views: cost-by-activity (the viral table + surprise-ratio footer) and cost-by-file.
- CLI: `--since` (`7d` default, `today`, absolute date), `--no-llm`, `--files`, `--format {table,json,markdown}`, `--model`.
- Subagent spend separation (no double-count — parent and subagent are physically separate files).
- File attribution excludes Claude Code memory/journal bookkeeping files.
- README with vs-ccusage and vs-CodeBurn comparison.

### Known Limitations
- Claude Code first. Cursor per-turn tokens aren't locally accessible (marked "limited"). Codex/Aider/Cline formats are roadmap.
- cost-by-commit / cost-by-issue are v1.1. v1.0 ships cost-by-activity + cost-by-file.
- Categorization accuracy is not 100%; an `unattributed` bucket is shown rather than faking precision.
- Bundled pricing is a snapshot; verify against `docs.claude.com/pricing` before publishing dollar claims.

[Unreleased]: https://github.com/romiluz13/pay4what/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/romiluz13/pay4what/releases/tag/v0.1.0
