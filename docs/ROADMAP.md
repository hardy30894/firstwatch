# Roadmap

## v0.1, done

Watch (OpenAPI diffing), scan (TypeScript, Stripe), match, mend tier 1,
prove gate (staged typecheck), pr command, registry seeding from real
stripe/openapi history, install.sh, CI, release workflow, GitHub Action.

## Launch chores, manual

- Create the GitHub org, push, tag v0.1.0 (activates releases and install.sh)
- Register the npm and PyPI package names
- Optional: domain, Homebrew tap

## v0.2, recipes and reach

- Tier 2 recipes: mend consumes provider-authored fix recipes as data.
  Critical path for both the second provider and the per-provider variant.
- Python scanning, second language for scan
- Replay proving: record request/response fixtures, replay old vs new code
- Opt-in test-suite verifier for prove (typecheck only today)

## v0.3, ecosystem

- Second and third providers (OpenAI, GitHub) added through recipes and
  changefeeds only, no core changes. This is the extensibility test.
- Split firstwatch-schema and firstwatch-registry into their own repos, spec v1
- Community contribution docs for changefeeds and recipes
- Node and Python bindings (napi-rs, PyO3)

## v0.4, agent tier and provider variant

- Mend tier 3: LLM synthesis for non-mechanical changes. Pluggable provider,
  user's own key, off by default, always behind the prove gate.
- Changelog parser in watch: prose release notes to ChangeEvents, flagged
  lower confidence
- Provider publishing flow: Action template that diffs the spec on release
  and emits a changefeed with recipes
- Multi-repo runner: fan pr out across repos under a GitHub App install.
  With this, "install Stripe's update agent" is assembled.

## v1.0, network effects

- Cross-repo fix propagation: the first proven patch for a change becomes
  a template for every other affected repo
- Blast-radius prediction: usage graph answers "who breaks if this ships"
  before the vendor ships it
