use super::graph_builder::ModelSpec;
use rand::prelude::*;
use rand_chacha::ChaChaRng;

/// Generate Python model content for a model spec.
pub fn generate_python(rng: &mut ChaChaRng, spec: &ModelSpec) -> String {
    let template_idx = rng.gen_range(0..4);
    match template_idx {
        0 => simple_ref(spec),
        1 => union_tagged(spec),
        2 => multi_ref_join(spec),
        3 => conditional_sql(rng, spec),
        _ => simple_ref(spec),
    }
}

/// 1. Simple ref — returns SELECT from a single smelt.ref().
fn simple_ref(spec: &ModelSpec) -> String {
    let dep = &spec.dependencies[0];
    format!(
        r#"from smelt import model

@model
def {name}(project):
    """Generated model: simple ref."""
    return """
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    event_time,
    amount,
    status
FROM smelt.ref('{dep}')
WHERE status = 'active'
"""
"#,
        name = spec.name,
    )
}

/// 2. Union tagged — uses project.find_models(tag=...) for UNION ALL.
fn union_tagged(spec: &ModelSpec) -> String {
    let dep = &spec.dependencies[0];
    format!(
        r#"from smelt import model

@model
def {name}(project):
    """Generated model: union tagged."""
    parts = []
    for dep in ['{dep}']:
        parts.append(f"SELECT user_id, event_time, amount FROM smelt.ref('{{dep}}')")
    return """
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
""" + "\nUNION ALL\n".join(parts)
"#,
        name = spec.name,
    )
}

/// 3. Multi-ref join — hardcoded refs joined together.
fn multi_ref_join(spec: &ModelSpec) -> String {
    let deps: Vec<&str> = spec.dependencies.iter().map(|s| s.as_str()).collect();
    let first = deps[0];
    let second = if deps.len() > 1 { deps[1] } else { deps[0] };

    format!(
        r#"from smelt import model

@model
def {name}(project):
    """Generated model: multi-ref join."""
    return """
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.user_id,
    a.event_time,
    b.amount
FROM smelt.ref('{first}') a
LEFT JOIN smelt.ref('{second}') b ON a.user_id = b.user_id
"""
"#,
        name = spec.name,
    )
}

/// 4. Conditional SQL — if/else logic choosing SQL variants.
fn conditional_sql(rng: &mut ChaChaRng, spec: &ModelSpec) -> String {
    let dep = &spec.dependencies[0];
    let threshold = rng.gen_range(10..100);
    format!(
        r#"from smelt import model

@model
def {name}(project):
    """Generated model: conditional SQL."""
    threshold = {threshold}
    if threshold > 50:
        filter_clause = "WHERE amount > 100"
    else:
        filter_clause = "WHERE amount > 0"
    return f"""
---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    event_time,
    amount,
    category
FROM smelt.ref('{dep}')
{{filter_clause}}
"""
"#,
        name = spec.name,
    )
}
