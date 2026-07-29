# firstwatch

First eyes on upstream API changes.

The APIs your code depends on change without asking: endpoints get removed, parameters get renamed, defaults flip. Firstwatch finds the exact call sites a change hits, fixes what it can prove safe, and refuses the rest with a reason. It never guesses, and it never touches your tree without proof.

```
$ firstwatch check --events changefeed.ndjson
[BREAKS NOW] src/billing.ts:10 orders.create (Removed)
    return stripe.orders.create({ items });
[BREAKS NOW] src/billing.py:11 orders.create (Removed)
    return stripe.Order.create(items=items)
[DEPRECATED] src/billing.ts:6 charges.create (Deprecated)
    return stripe.charges.create({ amount, currency: "usd", customer: customerId });
firstwatch: 3 impacts across 7 call sites, 2 breaking.
```

## Install

```
curl -fsSL https://raw.githubusercontent.com/firstwatch-dev/firstwatch/main/install.sh | sh
```

One static binary, no runtime. Or build from source: `cargo build --release`.

## Sixty seconds

Run it in any repo. No config, no account. It detects which SDKs you use from your imports.

```
firstwatch check --events changefeed.ndjson     # what breaks, and where
firstwatch fix   --events changefeed.ndjson     # dry run: proposed patches and refusals
firstwatch fix   --events changefeed.ndjson --write    # apply, if proof passes
firstwatch pr    --events changefeed.ndjson     # same, as a branch and pull request
```

Changefeeds describe what changed upstream. Ready-made feeds live in [`registry/`](registry/); generate your own from any two OpenAPI specs with `firstwatch watch`, or from prose release notes with `firstwatch changelog`.

## Nothing ships without proof

Patches are applied to a staged copy of your repo first. Your own typecheck runs there, plus your test suite with `--with-tests` and a differential replay with `--replay <cmd>`. Only a passing verdict lets `--write` touch the real tree.

```
[prove] typecheck (tsc --noEmit) FAILED
src/billing.ts(8,40): error TS2741: Property 'confirm' is missing ...
firstwatch: refused to apply 1 patches, proof failed. Your tree is untouched.
```

Refusals are first-class. Anything firstwatch cannot fix or prove is reported with the reason, never skipped silently and never guessed at. A pull request from `firstwatch pr` carries the proof checks, evidence links per patch, and a "needs human attention" section for every refusal.

## How it works

Five blocks, each reading and writing newline-delimited JSON in a versioned schema. Use the whole pipeline or any block alone; they compose as Unix pipes.

```
watch -> scan -> match -> mend -> prove
```

| Block | Does |
|---|---|
| `watch` | Diffs two OpenAPI specs into ChangeEvents |
| `changelog` | Parses prose release notes into ChangeEvents, flagged low confidence |
| `scan` | Finds SDK call sites in your code (tree-sitter; TypeScript and Python today) |
| `match` | Joins changes against call sites, ranked by severity |
| `fix` / `mend` | Three patch tiers: deterministic rewrites, provider recipes, model synthesis |
| `prove` | Runs your verifiers in a staged copy; failed proof blocks the write |

```
firstwatch scan | firstwatch match --events changefeed.ndjson
```

## Fixes, in order of trust

1. **Deterministic**: renames with a known replacement. Applied only when the site still matches what the scanner saw.
2. **Recipes**: provider-authored fix steps shipped as data next to the changefeed, like "rename the surface, add `confirm: true`, rename `source` to `payment_method` where present". See [docs/registry.md](docs/registry.md).
3. **Synthesis** (`--llm`, off by default): a model writes the rewrite for changes that are not mechanical, like a flipped default that needs pinning. Synthesized patches are always labeled.

Every tier faces the same prove gate. A wrong recipe or a bad model cannot ship broken code; it can only get refused.

## Bring your own model

```
export ANTHROPIC_API_KEY=...                     # default: Claude
```

Or any OpenAI-compatible server (Ollama, vLLM, Groq, OpenRouter, company gateways):

```
export FIRSTWATCH_LLM_PROVIDER=openai
export FIRSTWATCH_LLM_BASE_URL=http://localhost:11434/v1/chat/completions
export FIRSTWATCH_LLM_MODEL=llama3.1
```

Or, for internal APIs with custom auth, any executable that reads a prompt on stdin and writes JSON to stdout:

```
export FIRSTWATCH_LLM_CMD=./my-llm-bridge.sh
```

Your credentials never enter firstwatch. Bridge commands run under a timeout (`FIRSTWATCH_LLM_TIMEOUT`, default 120s), and whichever model you bring, the proof gate stays the judge.

## Many repos

```
firstwatch blast --repos repos.txt --events changefeed.ndjson
```

Blast answers "who breaks if this ships" across a list of repos or clone URLs: per-repo counts, per-change site lists, nonzero exit for CI. Read-only.

```
firstwatch fleet --repos repos.txt --events changefeed.ndjson --llm
```

Fleet runs the whole pipeline per repo and opens PRs. Proven fixes propagate: the first proven patch for a change becomes the template for the same change in every later repo, so each unique change is synthesized at most once and the fleet gets cheaper as it runs. Every repo still gets its own proof.

## In CI

```yaml
- uses: firstwatch-dev/firstwatch/action@main
  with:
    events: registry/stripe/changefeed.ndjson
```

Fails the build when a changefeed breaks the repo.

## For API providers

Drop the publish action into the repo that holds your OpenAPI spec, and every release appends its own ChangeEvents to your changefeed. Ship recipes next to it and your customers get proven fixes instead of warnings.

```yaml
- uses: firstwatch-dev/firstwatch/action/publish@main
  with:
    spec: openapi/spec.json
    provider: yourname
```

Adding a provider to firstwatch itself is one line of registry data, no code. See [docs/registry.md](docs/registry.md).

## Library mode

The read side (scan, match, diff) ships as Node and Python bindings under [`bindings/`](bindings/), thin JSON contracts over the same Rust core.

## Status

v0.1. The schema is versioned but not yet stable. Stripe, OpenAI, and GitHub changefeeds ship in `registry/`; the Stripe feed is generated from real stripe/openapi tag history by `scripts/seed-stripe.sh`.

## Build

```
cargo build --release
cargo test
```

## License

Apache-2.0
