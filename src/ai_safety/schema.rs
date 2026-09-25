//! Klang v2 runtime schemas (Phase 5).
//!
//! Deterministic, local validation only: no registry, no network. A
//! [`Schema`] describes nested records; [`validate`] returns the first
//! failure with its dotted path (e.g. `user.address.zip`). Schema identity
//! (`name@version:hash`) binds proofs to exactly one schema so a proof for
//! `A` never validates `B`.

use std::collections::HashMap;

/// Identity of one schema version.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SchemaId(pub String);

impl SchemaId {
    /// Canonical identity string `name@version:hash`.
    pub fn new(name: &str, version: &str, hash: u64) -> Self {
        Self(format!("{name}@{version}:{hash:016x}"))
    }
}

/// A runtime data value (local subset: ints, strings, bools, records).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataValue {
    /// Integer.
    Int(i64),
    /// String.
    Str(String),
    /// Boolean.
    Bool(bool),
    /// Nested record (`field -> value`).
    Record(HashMap<String, DataValue>),
}

impl DataValue {
    /// Short type name for diagnostics (`int`, `str`, `bool`, `record`).
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Int(_) => "int",
            Self::Str(_) => "str",
            Self::Bool(_) => "bool",
            Self::Record(_) => "record",
        }
    }
}

/// Expected type of one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldTy {
    /// Integer field.
    Int,
    /// String field.
    Str,
    /// Boolean field.
    Bool,
    /// Nested record with its own fields.
    Record(Vec<SchemaField>),
}

/// One named field in a [`Schema`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaField {
    /// Field name.
    pub name: String,
    /// Expected type.
    pub ty: FieldTy,
}

impl SchemaField {
    /// Build one field.
    pub fn new(name: &str, ty: FieldTy) -> Self {
        Self {
            name: name.to_string(),
            ty,
        }
    }
}

/// A named, versioned record schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    /// Schema name (e.g. `Profile`).
    pub name: String,
    /// Schema version (e.g. `1`).
    pub version: String,
    /// Top-level fields.
    pub fields: Vec<SchemaField>,
}

impl Schema {
    /// Build a schema.
    pub fn new(name: &str, version: &str, fields: Vec<SchemaField>) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            fields,
        }
    }

    /// Deterministic content hash (FNV-1a over name, version, fields).
    pub fn hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in self.name.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= 0xff;
        for b in self.version.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        fn field(h: &mut u64, f: &SchemaField) {
            for b in f.name.bytes() {
                *h ^= u64::from(b);
                *h = h.wrapping_mul(0x100000001b3);
            }
            match &f.ty {
                FieldTy::Int => *h ^= 0x01,
                FieldTy::Str => *h ^= 0x02,
                FieldTy::Bool => *h ^= 0x03,
                FieldTy::Record(inner) => {
                    *h ^= 0x04;
                    for g in inner {
                        field(h, g);
                    }
                }
            }
            *h = h.wrapping_mul(0x100000001b3);
        }
        for f in &self.fields {
            field(&mut h, f);
        }
        h
    }

    /// Canonical identity of this schema version.
    pub fn identity(&self) -> SchemaId {
        SchemaId::new(&self.name, &self.version, self.hash())
    }

    /// Validate `value` against this schema (first failure wins).
    pub fn validate(&self, value: &DataValue) -> Result<(), VerifyError> {
        let id = self.identity();
        match value {
            DataValue::Record(map) => validate_fields(&id, &self.fields, map, String::new()),
            other => Err(VerifyError {
                schema: id,
                path: String::new(),
                expected: "record".to_string(),
                found: other.type_name().to_string(),
                message: "top-level value must be a record".to_string(),
            }),
        }
    }
}

/// Structured schema failure with the dotted path of the bad value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError {
    /// Schema that rejected the value.
    pub schema: SchemaId,
    /// Dotted path (e.g. `user.address.zip`; empty = top level).
    pub path: String,
    /// What the schema wanted (`int`, `str`, `record`, ...).
    pub expected: String,
    /// What was found instead.
    pub found: String,
    /// Human-readable message.
    pub message: String,
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn validate_fields(
    id: &SchemaId,
    fields: &[SchemaField],
    map: &HashMap<String, DataValue>,
    prefix: String,
) -> Result<(), VerifyError> {
    for f in fields {
        let path = join(&prefix, &f.name);
        let v = match map.get(&f.name) {
            Some(v) => v,
            None => {
                return Err(VerifyError {
                    schema: id.clone(),
                    path,
                    expected: field_name(&f.ty),
                    found: "missing".to_string(),
                    message: format!("missing field `{}`", f.name),
                });
            }
        };
        validate_ty(id, &f.ty, v, &path)?;
    }
    Ok(())
}

fn field_name(ty: &FieldTy) -> String {
    match ty {
        FieldTy::Int => "int".to_string(),
        FieldTy::Str => "str".to_string(),
        FieldTy::Bool => "bool".to_string(),
        FieldTy::Record(_) => "record".to_string(),
    }
}

fn validate_ty(id: &SchemaId, ty: &FieldTy, v: &DataValue, path: &str) -> Result<(), VerifyError> {
    match (ty, v) {
        (FieldTy::Int, DataValue::Int(_))
        | (FieldTy::Str, DataValue::Str(_))
        | (FieldTy::Bool, DataValue::Bool(_)) => Ok(()),
        (FieldTy::Record(inner), DataValue::Record(map)) => {
            validate_fields(id, inner, map, path.to_string())
        }
        _ => Err(VerifyError {
            schema: id.clone(),
            path: path.to_string(),
            expected: field_name(ty),
            found: v.type_name().to_string(),
            message: format!("field `{path}` has the wrong type"),
        }),
    }
}
