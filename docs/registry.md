# The registry

The registry is the data half of firstwatch: machine-readable descriptions of
what changed upstream and how to fix it. The tool is the engine; the registry
is the fuel. Everything in it is ndjson in the versioned schema, contributable
without touching Rust.

```
registry/
  providers.ndjson          how scan recognizes each provider in code
  <provider>/
    changefeed.ndjson       what changed, one ChangeEvent per line
    recipes.ndjson          how to fix it, one Recipe per line
```

## Adding a provider

One line in `providers.ndjson`:

```json
{"schema_version":"0.1","name":"github","constructors":["Octokit"],"roots":["octokit","github"]}
```

- `constructors`: class names whose `new X()` / `X()` binding marks a variable
  as this provider's client
- `roots`: module-level identifiers that mark a call chain as this provider

Scan then resolves call chains of any depth from those anchors:
`octokit.teams.listMembersLegacy(...)` becomes surface
`teams.listMembersLegacy`. No core changes, no rebuild.

## Authoring a changefeed

One ChangeEvent per line. The load-bearing fields:

- `surface`: SDK-style path, `charges.create`, `chat.completions.create`.
  Language-agnostic on purpose: the scanners normalize every SDK style
  (legacy Python classes, modern clients, TS members) to the same surfaces,
  so one feed serves every language.
- `kind`: `removed | renamed | type_changed | required_added | deprecated | added`
- `field`: set when the change is to a parameter rather than the operation
- `to`: replacement surface for renames and deprecations. This alone enables
  a tier 1 automatic fix.
- `effective_version` / `effective_date`: when it bites. A date makes the
  severity `breaks_later` instead of `breaks_now`.
- `confidence`: `high` for spec diffs, lower for changelog parsing
- `evidence_url`: where a human can verify the claim. Always include one.

Generate rather than hand-write when a spec exists:

```
firstwatch watch --old v1.json --new v2.json --provider stripe >> changefeed.ndjson
```

## Authoring recipes

A recipe is an ordered list of steps matched to impacts by provider and
surface, optionally narrowed by `field` and `kind`:

```json
{"schema_version":"0.1","provider":"stripe","surface":"charges.create","kind":"deprecated","steps":[
  {"op":"rename_surface","to":"payment_intents.create"},
  {"op":"set_field","name":"confirm","value":"true"},
  {"op":"rename_field","from":"source","to":"payment_method","optional":true}
]}
```

Steps: `rename_surface`, `rename_field`, `set_field`, `remove_field`.

Two rules that keep recipes safe:

1. A required step that cannot apply cleanly fails the whole recipe with a
   reason. Nothing partial is ever proposed.
2. Mark a step `optional` when not every call site passes that field. Optional
   steps apply where they can and skip where they cannot.

The recipe only proposes. The prove gate still decides: patches apply to a
staged copy, verifiers run there, and a failed proof means nothing is written.
A wrong recipe cannot ship broken code, it can only get refused.

## Validating a contribution

```
firstwatch check <repo-using-the-sdk> --events registry/<p>/changefeed.ndjson --providers registry/providers.ndjson
firstwatch fix <repo> --events ... --recipes registry/<p>/recipes.ndjson
```

A good contribution comes with a minimal example file in `examples/` that the
feed lights up, so reviewers can run it and see the impacts.

## For API providers: publish your own changefeed

Providers do not need to wait for the community. Drop the publish action into
the repo that holds your OpenAPI spec and every release appends its own
ChangeEvents to your changefeed:

```yaml
on:
  push:
    branches: [main]
    paths: [openapi/spec.json]
jobs:
  changefeed:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 2 }
      - uses: firstwatch-dev/firstwatch/action/publish@main
        with:
          spec: openapi/spec.json
          provider: yourname
```

Ship recipes next to it and your customers get proven fixes, not just
warnings. This is the per-provider variant: your changefeed plus your recipes,
consumed by every repo running firstwatch.
