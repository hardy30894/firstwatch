# firstwatch

First eyes on upstream API changes.

Firstwatch watches the APIs your code depends on, finds the exact call sites a change hits, and tells you before it breaks you. Patch generation and proof-gated auto-PRs are on the roadmap; detection ships first.

```
$ firstwatch check --events registry/stripe/sample.ndjson
[BREAKS NOW] src/billing.ts:10 orders.create (Removed)
    return stripe.orders.create({ items });
[DEPRECATED] src/billing.ts:6 charges.create (Deprecated)
    return stripe.charges.create({ amount, currency: "usd", customer: customerId });
firstwatch: 3 impacts across 4 call sites, 2 breaking.
```

## How it works

Five blocks, each with a typed ndjson contract. Use the whole pipeline or any block alone:

```
watch -> scan -> match -> mend -> prove
```

- **watch**: turns upstream changes (OpenAPI diffs, changelogs, deprecation headers) into ChangeEvents
- **scan**: tree-sitter scan of your repo, emits UsageSites
- **match**: pure join, emits Impacts
- **mend**: deterministic rewrites with preconditions. Anything it cannot fix mechanically it refuses, with the reason
- **prove**: patches are applied to a staged copy and verifiers run there. Proof fails, nothing is written:

```
[prove] typecheck (tsc --noEmit) FAILED
src/billing.ts(8,40): error TS2741: Property 'confirm' is missing ...
firstwatch: refused to apply 1 patches, proof failed. Your tree is untouched.
```

```
$ firstwatch fix --events registry/stripe/sample.ndjson --write
src/billing.ts:6
- return stripe.charges.create({ amount, currency: "usd", customer: customerId });
+ return stripe.payment_intents.create({ amount, currency: "usd", customer: customerId });
[refused] src/billing.ts:10 orders.create (no deterministic rewrite for this change kind)
firstwatch: applied 1 of 1 proposed patches, 2 refused. Run your tests.
```

Blocks compose as Unix pipes:

```
firstwatch scan | firstwatch match --events registry/stripe/sample.ndjson
```

When you want the change as a PR instead of a local edit:

```
$ firstwatch pr --events registry/stripe/changefeed.ndjson
[prove] typecheck (tsc --noEmit) ok
firstwatch: PR opened from firstwatch/stripe-fixes.
```

The PR body carries the proof checks, one line per patch with the effective version and an evidence link, and a "needs human attention" section for every refusal. No proof, no PR, unless you pass --allow-unproven.

When a deterministic fix is impossible, opt into model synthesis with `--llm`. Bring any model:

```
export ANTHROPIC_API_KEY=...                       # default: Claude
export FIRSTWATCH_LLM_PROVIDER=openai              # or any OpenAI-compatible server
export FIRSTWATCH_LLM_BASE_URL=http://localhost:11434/v1/chat/completions
export FIRSTWATCH_LLM_MODEL=llama3.1               # Ollama, vLLM, Groq, gateways
export FIRSTWATCH_LLM_CMD=./my-llm-bridge.sh       # escape hatch: your own script,
                                                   # prompt on stdin, json on stdout
```

Synthesized patches are labeled and face the same prove gate as everything else. Whichever model you bring, the proof gate stays the judge.

Across many repos:

```
firstwatch blast --repos repos.txt --events changefeed.ndjson   # who breaks, before it ships
firstwatch fleet --repos repos.txt --events changefeed.ndjson --llm
```

Fleet runs the whole pipeline per repo and opens PRs. With `--llm`, each unique change is synthesized at most once: the first proven fix becomes the template for the same change in every later repo, so the fleet gets cheaper and more reliable as it runs.

## Status

v0.1, early. Working today: OpenAPI spec diffing (validated on real 7.8MB Stripe specs, ~110ms per diff), TypeScript and Python scanning across providers, changefeed matching, `check` reports, `fix` with three patch tiers (deterministic rewrites, provider recipes, opt-in Claude synthesis via `--llm` using your own key), changelog parsing to structured events, the prove gate (typecheck, tests, differential replay in staged copies; failed proof blocks the write), `pr`, and `fleet` for many repos. Synthesized patches are always labeled and face the same proof gate. The Stripe changefeed in `registry/` is generated from real stripe/openapi tag history by `scripts/seed-stripe.sh`. Schema is versioned but not yet stable.

## Build

```
cargo build
cargo test
```

## License

Apache-2.0
