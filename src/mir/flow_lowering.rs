//! Klang v2 Flow lowering (Phase 7).
//!
//! A Flow lowers to an explicit environment plus ordered steps — no opaque
//! closures, no implicit capture. `env` lists params then dependencies by
//! name; `steps` binds each one before running the body.

use crate::ast::flow::FlowExpr;

/// One explicit lowering step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowStep {
    /// Bind an ordinary parameter into the Flow frame.
    BindParam {
        /// Parameter name.
        name: String,
    },
    /// Bind an explicit dependency into the Flow frame.
    BindDep {
        /// Dependency name.
        name: String,
    },
    /// Run the body source with the frame bound.
    RunBody {
        /// Raw body source.
        body: String,
    },
    /// Return from the Flow.
    Return {
        /// Declared return type, if any.
        ty: Option<String>,
    },
}

/// A lowered Flow: explicit environment plus steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredFlow {
    /// Bound name, if let-bound.
    pub name: Option<String>,
    /// Explicit environment: params in order, then deps in order.
    pub env: Vec<String>,
    /// Ordered steps.
    pub steps: Vec<FlowStep>,
}

/// Lower one Flow expression deterministically.
pub fn lower_flow(flow: &FlowExpr, name: Option<&str>) -> LoweredFlow {
    let mut env: Vec<String> = Vec::new();
    let mut steps: Vec<FlowStep> = Vec::new();
    for p in &flow.params {
        env.push(p.name.clone());
        steps.push(FlowStep::BindParam {
            name: p.name.clone(),
        });
    }
    for d in &flow.deps {
        env.push(d.name.clone());
        steps.push(FlowStep::BindDep {
            name: d.name.clone(),
        });
    }
    steps.push(FlowStep::RunBody {
        body: flow.body.clone(),
    });
    steps.push(FlowStep::Return {
        ty: flow.return_ty.clone(),
    });
    LoweredFlow {
        name: name.map(str::to_string),
        env,
        steps,
    }
}
