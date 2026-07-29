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
- **mend**: generates patch candidates (planned)
- **prove**: a patch ships only if it typechecks, passes tests, and survives replay. Anything unprovable is refused and reported, never applied (planned)

Blocks compose as Unix pipes:

```
firstwatch scan | firstwatch match --events registry/stripe/sample.ndjson
```

## Status

v0.1, early. Working today: TypeScript scanning for Stripe SDK usage, changefeed matching, `check` reports. Schema is versioned but not yet stable.

## Build

```
cargo build
cargo test
```

## License

Apache-2.0
