//! Klang v2 schema-boundary checking (Phase 6).
//!
//! Connects type-level schema names to runtime [`Schema`]s: a registry maps
//! names to definitions, and [`check_boundary`] validates a value,
//! converting failures into `E-SCHEMA-INVALID` diagnostics.

use crate::ai_safety::provenance::Provenance;
use crate::ai_safety::schema::{DataValue, Schema};
use crate::diagnostics::Diagnostic;
use std::collections::HashMap;

/// Name → schema registry (one scope's worth of declarations).
#[derive(Debug, Default)]
pub struct SchemaRegistry {
    /// Registered schemas by name.
    schemas: HashMap<String, Schema>,
}

impl SchemaRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) a schema under its own name.
    pub fn register(&mut self, schema: Schema) {
        self.schemas.insert(schema.name.clone(), schema);
    }

    /// Look up a schema by name, or `E-SCHEMA-NOT-FOUND`.
    pub fn lookup(
        &self,
        name: &str,
        file: &str,
        start: usize,
        end: usize,
    ) -> Result<&Schema, Diagnostic> {
        self.schemas
            .get(name)
            .ok_or_else(|| crate::ai_safety::diagnostics::schema_not_found(file, start, end, name))
    }

    /// Validate `value` at a `tune`/`verify` boundary for schema `name`.
    ///
    /// Success mints an identity-bound [`Provenance`]; failure is the
    /// structured `E-SCHEMA-INVALID` diagnostic with the nested path.
    pub fn check_boundary(
        &self,
        name: &str,
        value: &DataValue,
        file: &str,
        start: usize,
        end: usize,
    ) -> Result<Provenance, Diagnostic> {
        let schema = self.lookup(name, file, start, end)?;
        match super::tuner::tune(schema, value, file, (start, end)) {
            Ok(proof) => Ok(proof),
            Err(e) => Err(crate::ai_safety::diagnostics::schema_invalid(
                file, start, end, &e,
            )),
        }
    }
}
