# Roadmap

## v0.1, done

Watch (OpenAPI diffing), scan (TypeScript, Stripe), match, mend tier 1,
prove gate (staged typecheck), pr command, registry seeding from real
stripe/openapi history, install.sh, CI, release workflow, GitHub Action.

## Launch chores, manual

- Create the GitHub org, push, tag v0.1.0 (activates releases and install.sh)
- Register the npm and PyPI package names
- Optional: domain, Homebrew tap

## v0.2, recipes and reach, done

- Tier 2 recipes: mend consumes provider-authored fix recipes as data,
  with required and optional steps. Unlocks the per-provider variant.
- Python scanning behind per-language dispatch, legacy and modern
  stripe-python styles normalized to shared surfaces
- Differential replay proving: same command in unpatched and patched
  stagings, identical output required (--replay)
- Opt-in test-suite verifier, npm test or pytest (--with-tests)

## v0.3, ecosystem

- Done: OpenAI (second provider) and GitHub (third, pure registry data,
  zero core changes via the data-driven provider table)
- Done: Node and Python bindings, thin JSON contracts over scan, match,
  and diff, mend and prove stay CLI-only
- Blocked on launch chores: split firstwatch-schema and
  firstwatch-registry into their own repos, spec v1
- Community contribution docs for changefeeds and recipes

## v0.4, agent tier and provider variant, done

- Done: provider publishing flow, action/publish diffs the spec on every
  release and appends to the provider's changefeed
- Done: multi-repo runner, firstwatch fleet fans scan-match-mend-prove-pr
  across a list of repos or clone URLs. The per-provider variant is
  assembled: publish action upstream, fleet downstream.
- Done: mend tier 3, Claude synthesis for non-mechanical changes behind
  --llm. User's own ANTHROPIC_API_KEY, off by default, patches labeled
  llm and always behind the prove gate.
- Done: changelog subcommand, prose release notes to ChangeEvents via
  structured outputs, everything flagged low confidence

## v1.0, network effects

- Cross-repo fix propagation: the first proven patch for a change becomes
  a template for every other affected repo
- Blast-radius prediction: usage graph answers "who breaks if this ships"
  before the vendor ships it
