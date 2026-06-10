//! Benchmarks for variable resolution and template rendering.
//!
//! Run with:
//! ```bash
//! cargo bench --bench variable_resolution
//! ```

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::collections::HashMap;
use std::sync::Arc;

use ansiblers_core::{Inventory, Value};
use ansiblers_templates::AnsibleTemplateEngine;
use ansiblers_vars::VariableResolver;

// ---------------------------------------------------------------------------
// Variable resolution benchmarks
// ---------------------------------------------------------------------------

fn bench_var_resolution_simple(c: &mut Criterion) {
    c.bench_function("var_resolve_simple_interpolation", |b| {
        let ctx = ansiblers_core::ExecutionContext::new(
            Arc::new(Inventory::default()),
            HashMap::from([
                ("greeting".to_string(), Value::String("hello".to_string())),
                ("target".to_string(), Value::String("world".to_string())),
            ]),
        );
        let resolver = VariableResolver::new(&ctx);
        let vars = resolver.merged("localhost");
        let engine = AnsibleTemplateEngine::new();

        b.iter(|| {
            engine
                .render(black_box("{{ greeting }}, {{ target }}!"), &vars)
                .unwrap()
        })
    });
}

fn bench_var_resolution_nested(c: &mut Criterion) {
    c.bench_function("var_resolve_nested_dict", |b| {
        let ctx = ansiblers_core::ExecutionContext::new(
            Arc::new(Inventory::default()),
            HashMap::from([(
                "config".to_string(),
                serde_json::json!({
                    "database": {
                        "host": "db.example.com",
                        "port": 5432,
                        "name": "myapp"
                    }
                }),
            )]),
        );
        let resolver = VariableResolver::new(&ctx);
        let vars = resolver.merged("localhost");
        let engine = AnsibleTemplateEngine::new();

        b.iter(|| {
            engine
                .render(
                    black_box("{{ config.database.host }}:{{ config.database.port }}"),
                    &vars,
                )
                .unwrap()
        })
    });
}

fn bench_var_resolution_loop_context(c: &mut Criterion) {
    c.bench_function("var_resolve_loop_with_filter", |b| {
        let ctx = ansiblers_core::ExecutionContext::new(
            Arc::new(Inventory::default()),
            HashMap::from([("item".to_string(), Value::String("alpha".to_string()))]),
        );
        let resolver = VariableResolver::new(&ctx);
        let vars = resolver.merged("localhost");
        let engine = AnsibleTemplateEngine::new();

        b.iter(|| {
            engine
                .render(black_box("Processing {{ item | upper }}"), &vars)
                .unwrap()
        })
    });
}

// ---------------------------------------------------------------------------
// Template rendering benchmarks
// ---------------------------------------------------------------------------

fn bench_template_complex(c: &mut Criterion) {
    c.bench_function("template_render_complex", |b| {
        let ctx = ansiblers_core::ExecutionContext::new(
            Arc::new(Inventory::default()),
            HashMap::from([
                ("name".to_string(), Value::String("ansiblers".to_string())),
                ("version".to_string(), Value::String("0.6.0".to_string())),
                (
                    "features".to_string(),
                    Value::Array(vec![
                        Value::String("fast".to_string()),
                        Value::String("safe".to_string()),
                        Value::String("compatible".to_string()),
                    ]),
                ),
            ]),
        );
        let resolver = VariableResolver::new(&ctx);
        let vars = resolver.merged("localhost");
        let engine = AnsibleTemplateEngine::new();

        b.iter(|| {
            engine
                .render(
                    black_box("{{ name }} v{{ version }} — features: {{ features | join(', ') }}"),
                    &vars,
                )
                .unwrap()
        })
    });
}

fn bench_merged_vars_100(c: &mut Criterion) {
    c.bench_function("var_merge_100_vars", |b| {
        let vars: HashMap<String, Value> = (0..100)
            .map(|i| (format!("var_{i}"), Value::String(format!("value_{i}"))))
            .collect();
        let ctx =
            ansiblers_core::ExecutionContext::new(Arc::new(Inventory::default()), vars.clone());

        b.iter(|| {
            let resolver = VariableResolver::new(&ctx);
            resolver.merged(black_box("localhost"))
        })
    });
}

criterion_group!(
    var_benches,
    bench_var_resolution_simple,
    bench_var_resolution_nested,
    bench_var_resolution_loop_context,
    bench_template_complex,
    bench_merged_vars_100,
);

criterion_main!(var_benches);
