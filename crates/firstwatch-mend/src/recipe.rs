use firstwatch_schema::{Impact, Recipe, RecipeStep};

pub fn find_recipe<'a>(recipes: &'a [Recipe], impact: &Impact) -> Option<&'a Recipe> {
    recipes.iter().find(|r| {
        r.provider == impact.change.provider
            && r.surface == impact.change.surface
            && r.field
                .as_ref()
                .is_none_or(|f| Some(f) == impact.change.field.as_ref())
            && r.kind.as_ref().is_none_or(|k| *k == impact.change.kind)
    })
}

/// Runs every step against the site line. A required step that cannot
/// apply cleanly fails the whole recipe with a reason, nothing partial.
/// Optional steps skip instead.
pub fn apply_recipe(line: &str, surface: &str, recipe: &Recipe) -> Result<String, String> {
    let mut out = line.to_string();
    let mut surface = surface.to_string();
    for step in &recipe.steps {
        match apply_step(&out, &surface, step) {
            Ok(next) => out = next,
            Err(_) if step.is_optional() => continue,
            Err(reason) => return Err(reason),
        }
        if let RecipeStep::RenameSurface { to } = step {
            surface = to.clone();
        }
    }
    Ok(out)
}

fn apply_step(line: &str, surface: &str, step: &RecipeStep) -> Result<String, String> {
    match step {
        RecipeStep::RenameSurface { to } => {
            if !line.contains(surface) {
                return Err(format!("line does not contain surface {surface}"));
            }
            Ok(line.replace(surface, to))
        }
        RecipeStep::RenameField { from, to, .. } => {
            let token = format!("{from}:");
            if !line.contains(&token) {
                return Err(format!("field {from} not found on the site line"));
            }
            Ok(line.replace(&token, &format!("{to}:")))
        }
        RecipeStep::SetField { name, value } => {
            let call_start = line
                .find(surface)
                .ok_or_else(|| format!("line does not contain surface {surface}"))?;
            let brace = line[call_start..]
                .find('{')
                .map(|i| call_start + i)
                .ok_or_else(|| {
                    "call arguments are not an object literal on this line".to_string()
                })?;
            let mut out = String::with_capacity(line.len() + name.len() + value.len() + 4);
            out.push_str(&line[..=brace]);
            out.push_str(&format!(" {name}: {value},"));
            out.push_str(&line[brace + 1..]);
            Ok(out)
        }
        RecipeStep::RemoveField { name, .. } => remove_field(line, name),
    }
}

fn remove_field(line: &str, name: &str) -> Result<String, String> {
    let token = format!("{name}:");
    let start = line
        .find(&token)
        .ok_or_else(|| format!("field {name} not found on the site line"))?;
    let value_start = start + token.len();
    let rest = &line[value_start..];
    let mut end = None;
    let mut has_trailing_comma = false;
    for (i, c) in rest.char_indices() {
        match c {
            '{' | '[' | '(' => {
                return Err(format!("field {name} has a nested value, needs a human"))
            }
            ',' => {
                end = Some(i + 1);
                has_trailing_comma = true;
                break;
            }
            '}' | ')' => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let end = end.ok_or_else(|| format!("could not find the end of field {name} on this line"))?;
    let after = line[value_start + end..].trim_start_matches(' ');
    if has_trailing_comma {
        Ok(format!("{}{after}", &line[..start]))
    } else {
        let before = line[..start].trim_end();
        let before = before.strip_suffix(',').unwrap_or(before);
        Ok(format!("{before} {after}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use firstwatch_schema::SCHEMA_VERSION;

    fn recipe(steps: Vec<RecipeStep>) -> Recipe {
        Recipe {
            schema_version: SCHEMA_VERSION.into(),
            provider: "stripe".into(),
            surface: "charges.create".into(),
            field: None,
            kind: None,
            steps,
        }
    }

    #[test]
    fn rename_surface_and_set_field() {
        let line = r#"  return stripe.charges.create({ amount, currency: "usd" });"#;
        let out = apply_recipe(
            line,
            "charges.create",
            &recipe(vec![
                RecipeStep::RenameSurface {
                    to: "payment_intents.create".into(),
                },
                RecipeStep::SetField {
                    name: "confirm".into(),
                    value: "true".into(),
                },
            ]),
        )
        .unwrap();
        assert_eq!(
            out,
            r#"  return stripe.payment_intents.create({ confirm: true, amount, currency: "usd" });"#
        );
    }

    #[test]
    fn rename_field() {
        let line = "stripe.charges.create({ amount: 100 });";
        let out = apply_recipe(
            line,
            "charges.create",
            &recipe(vec![RecipeStep::RenameField {
                from: "amount".into(),
                to: "amount_total".into(),
                optional: false,
            }]),
        )
        .unwrap();
        assert_eq!(out, "stripe.charges.create({ amount_total: 100 });");
    }

    #[test]
    fn remove_middle_and_last_field() {
        let line = "stripe.charges.create({ amount: 100, source: tok, capture: false });";
        let out = apply_recipe(
            line,
            "charges.create",
            &recipe(vec![RecipeStep::RemoveField {
                name: "source".into(),
                optional: false,
            }]),
        )
        .unwrap();
        assert_eq!(
            out,
            "stripe.charges.create({ amount: 100, capture: false });"
        );

        let out = apply_recipe(
            &out,
            "charges.create",
            &recipe(vec![RecipeStep::RemoveField {
                name: "capture".into(),
                optional: false,
            }]),
        )
        .unwrap();
        assert_eq!(out, "stripe.charges.create({ amount: 100 });");
    }

    #[test]
    fn refuses_nested_value_removal() {
        let line = "stripe.charges.create({ metadata: { a: 1 }, amount: 100 });";
        let err = apply_recipe(
            line,
            "charges.create",
            &recipe(vec![RecipeStep::RemoveField {
                name: "metadata".into(),
                optional: false,
            }]),
        )
        .unwrap_err();
        assert!(err.contains("needs a human"));
    }

    #[test]
    fn refuses_shorthand_field_rename() {
        let line = "stripe.charges.create({ amount });";
        let err = apply_recipe(
            line,
            "charges.create",
            &recipe(vec![RecipeStep::RenameField {
                from: "amount".into(),
                to: "amount_total".into(),
                optional: false,
            }]),
        )
        .unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn failed_step_fails_whole_recipe() {
        let line = "stripe.charges.create(params);";
        let err = apply_recipe(
            line,
            "charges.create",
            &recipe(vec![
                RecipeStep::RenameSurface {
                    to: "payment_intents.create".into(),
                },
                RecipeStep::SetField {
                    name: "confirm".into(),
                    value: "true".into(),
                },
            ]),
        )
        .unwrap_err();
        assert!(err.contains("not an object literal"));
    }
}

#[cfg(test)]
mod optional_tests {
    use super::*;
    use firstwatch_schema::SCHEMA_VERSION;

    #[test]
    fn optional_step_skips_when_field_absent() {
        let recipe = Recipe {
            schema_version: SCHEMA_VERSION.into(),
            provider: "stripe".into(),
            surface: "charges.create".into(),
            field: None,
            kind: None,
            steps: vec![
                RecipeStep::RenameSurface {
                    to: "payment_intents.create".into(),
                },
                RecipeStep::RenameField {
                    from: "source".into(),
                    to: "payment_method".into(),
                    optional: true,
                },
            ],
        };
        let line = r#"stripe.charges.create({ amount, currency: "usd" });"#;
        let out = apply_recipe(line, "charges.create", &recipe).unwrap();
        assert_eq!(
            out,
            r#"stripe.payment_intents.create({ amount, currency: "usd" });"#
        );

        let line = "stripe.charges.create({ source: tok });";
        let out = apply_recipe(line, "charges.create", &recipe).unwrap();
        assert_eq!(
            out,
            "stripe.payment_intents.create({ payment_method: tok });"
        );
    }
}
