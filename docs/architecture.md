# How firstwatch works

This is the long read. The [README](../README.md) gets you running in a minute; this explains what is actually happening, why it is built this way, and where you can plug in.

## The problem

Your code calls APIs you do not control. Stripe removes an endpoint, OpenAI deprecates one, a default flips from true to false. The vendor announces it in a blog post nobody reads, and weeks later your integration breaks in production.

The information needed to prevent this exists the whole time. The vendor knows what changed. Your code knows what it calls. Nothing connects the two. Firstwatch is that connection: it turns "what changed" into structured data, finds the exact lines in your code that data hits, and fixes what it can prove safe.

## One idea, five blocks

Firstwatch is a pipeline of five small programs. Each one reads and writes newline-delimited JSON in a versioned schema, so they compose like Unix pipes and any block can be replaced with your own.

```
watch -> scan -> match -> mend -> prove
```

The contract between blocks matters more than the blocks. If you can produce a ChangeEvent, you are a watcher. If you can consume an Impact, you can build your own mender. The schema is the product; the binary is one implementation of it.

### watch: what changed upstream

Input: two versions of an OpenAPI spec. Output: ChangeEvents.

```json
{"provider":"stripe","surface":"charges.create","field":"amount","kind":"type_changed","from":"integer","to":"string","confidence":"high","source":"openapi_diff"}
```

The differ walks paths, operations, and request properties, and emits an event for every removal, rename, type change, newly required field, deprecation, and addition. Diffing two real 7.8MB Stripe specs takes about 110ms.

`surface` is the load-bearing field. It is the SDK-style path (`charges.create`, `chat.completions.create`), not the REST path. Watch translates `POST /v1/charges` into `charges.create` so events join directly against what the scanner finds in code. This normalization is also why one changefeed serves every language: Python's legacy `stripe.Charge.create`, its modern `client.charges.create`, and TypeScript's `stripe.charges.create` all normalize to the same surface.

When there is no spec, `firstwatch changelog` parses prose release notes into the same events using a language model, and everything it produces is flagged `confidence: low`. Spec diffs are truth; changelog parsing is a useful guess, and the schema keeps the two honest about which is which.

### scan: what your code calls

Input: your repo. Output: UsageSites.

```json
{"provider":"stripe","surface":"charges.create","file":"src/billing.ts","line":6,"language":"typescript"}
```

Scan parses your code with tree-sitter and walks member-call chains of any depth (`client.chat.completions.create` works the same as `stripe.charges.create`). It knows a call belongs to a provider two ways: the variable was constructed from a known client class (`new Stripe(...)`, `OpenAI()`), or the root identifier matches a known module name.

What counts as "a known provider" is data, not code. The provider table ships in `registry/providers.ndjson`; adding a provider is one line naming its constructors and module roots. GitHub support was added this way, with zero Rust changes.

There is no configuration. Scan detects what SDKs you use from the code itself.

### match: who breaks, where

Input: ChangeEvents and UsageSites. Output: Impacts.

Match is a pure join on provider and surface, with a severity attached: `breaks_now`, `breaks_later` (the change has a future effective date), `deprecated`, or `opportunity` (something new you could adopt). It has no I/O and no side effects, which makes it trivially testable and reusable; the `blast` command is essentially match applied across a fleet of repos.

### mend: three tiers of fixing, in order of trust

Input: Impacts. Output: PatchCandidates and Refusals.

1. **Deterministic.** The change carries a known replacement (`charges.create` is deprecated, use `payment_intents.create`) and the site line still matches what the scanner saw. Pure string rewrite, no judgment involved.
2. **Recipes.** The provider (or community) shipped fix steps as data next to the changefeed: rename the surface, insert a field, rename a field where present. A required step that cannot apply cleanly fails the whole recipe; nothing partial is ever proposed. Steps for fields not every caller passes are marked optional and skip when absent.
3. **Model synthesis** (`--llm`, off by default). For changes that are not mechanical. Example: a default flipped from true to false. No rewrite rule can know the fix is to pin `auto_advance: true` explicitly; a model can. Synthesized patches are labeled `llm` everywhere they appear.

Everything mend cannot handle becomes a Refusal with a reason:

```json
{"reason":"no recipe and no deterministic rewrite for this change kind","impact":{...}}
```

Refusals are a feature, not a failure mode. A tool that guesses when it should refuse gets uninstalled the first time it guesses wrong.

### prove: the gate everything faces

Input: PatchCandidates. Output: a verdict.

Prove copies your repo to a temporary directory, applies the patches there, and runs verifiers in the copy:

- **Typecheck**, detected from the project (`tsc --noEmit` when there is a tsconfig)
- **Tests**, opt-in with `--with-tests` (`npm test` or `pytest`)
- **Differential replay**, opt-in with `--replay <cmd>`: the same command runs in an unpatched copy and a patched copy, and the outputs must be identical

Only a passing verdict lets `--write` touch your real tree or `pr` open a pull request. A failed proof prints the compiler output, leaves everything untouched, and exits nonzero.

This is the design's center of gravity. The three mend tiers differ in how much judgment they involve, but none of them is trusted: a wrong recipe or a bad model produces refusals, not broken code, because prove has the last word. That is also what makes bring-your-own-model safe. A small local model just gets refused more often.

## The registry: changes as shared data

Everything upstream-shaped lives in [`registry/`](../registry/) as plain ndjson:

```
registry/
  providers.ndjson        how scan recognizes each provider
  stripe/changefeed.ndjson    what changed (generated from real spec history)
  stripe/recipes.ndjson       how to fix it
  openai/...
  github/...
```

The tool is the engine; the registry is the fuel, and it is community-shaped on purpose. A vendor or contributor who adds a changefeed and recipes makes every firstwatch install smarter without anyone touching the code. API providers can automate their side entirely: the [publish action](../action/publish/action.yml) diffs their spec on every release and appends to their own changefeed. Authoring details are in [registry.md](registry.md).

## Many repos: blast and fleet

`blast` answers "who breaks if this ships" across a list of repos: scan and match only, nothing modified, nonzero exit for CI. It is the vendor's view of their own blast radius before a release.

`fleet` runs the full pipeline per repo and opens PRs. Its interesting property is propagation: when a patch is proven in one repo, that exact rewrite becomes the template for the same change in every later repo, re-indented per site and still proven per repo. With `--llm`, this means each unique change is synthesized at most once per run, no matter how many repos are affected. The fleet gets cheaper and more reliable as it runs.

## The graph: solving each change once, ecosystem-wide

Normal operation produces graph-shaped data as a byproduct, in two record types that were in the schema from day one.

**Proven fixes.** Every patch that passes the prove gate is recorded to a local store (`~/.firstwatch/proven.ndjson`; `FIRSTWATCH_HOME` overrides the directory) as a ProvenFix: the change identity, the trimmed old and new lines, and the checks that proved it. Later runs of `fix` and `fleet` load the store and reuse matching fixes as templates before trying anything else, so a change solved once on your machine is never synthesized again. `firstwatch share --proven` dumps the store for contribution; a fix proven by one person becomes a template for everyone, and every repo that receives it still runs its own proof.

**Usage edges.** `firstwatch share` aggregates a scan into UsageEdges: provider, surface, language, count, and structurally nothing else. No file paths and no repo names, so the record is anonymous by construction rather than by scrubbing. Shared edges accumulate in the registry and power `blast --usage`, which answers the ecosystem-wide question with no repos at hand: this change breaks this many known call sites, in these languages.

Nothing is ever uploaded automatically. Contribution is a pull request you can read before sending. The graph therefore grows the way advisory databases grew: through a schema worth standardizing on and contributions worth trusting, not through telemetry.

## Where you can plug in

Every extension point is data or an executable. None require touching Rust.

| To add | You provide |
|---|---|
| A provider | One line in `providers.ndjson` |
| A changefeed | ndjson events, hand-written or generated by `watch` / `changelog` |
| Fix recipes | ndjson steps next to the changefeed |
| A model | An API key, an OpenAI-compatible URL, or any executable via `FIRSTWATCH_LLM_CMD` |
| A verifier | Your project's own typecheck and tests, or a `--replay` command |
| A consumer | Anything that reads the ndjson contracts; Node and Python bindings ship in `bindings/` |

## Design rules the code follows

- **Provable, not guessed.** Nothing is applied without passing the gate. Anything unprovable is refused with the reason.
- **Refusals are first-class.** They have a schema, they appear in PRs, and they are the honest answer more often than a guess would be.
- **Deterministic before probabilistic.** The model is tier 3, not tier 1, and it is off by default.
- **Contracts over components.** Blocks talk ndjson so anyone can replace one.
- **Data over code.** Providers, changefeeds, and recipes are registry files, so the ecosystem can grow without releases.
- **Works offline.** The core pipeline needs no network, no account, and no model.
