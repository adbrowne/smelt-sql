pub mod cube_split;
pub mod incremental;

use crate::graph::ModelGraph;
use crate::types::Transformation;

/// The optimizer runs all registered rules against a model graph.
pub struct Optimizer {
    enable_cube_split: bool,
    enable_incremental: bool,
    #[cfg(feature = "python")]
    enable_python_rules: bool,
}

impl Optimizer {
    /// Create an optimizer with all rules enabled.
    pub fn new() -> Self {
        Self {
            enable_cube_split: true,
            enable_incremental: true,
            #[cfg(feature = "python")]
            enable_python_rules: true,
        }
    }

    /// Run all enabled rules, returning transformations and any errors.
    pub fn optimize(&self, graph: &ModelGraph) -> (Vec<Transformation>, Vec<String>) {
        let mut transformations = Vec::new();
        let mut errors = Vec::new();

        for model in graph.models() {
            if self.enable_cube_split {
                match cube_split::optimize(model) {
                    Ok(Some(t)) => transformations.push(t),
                    Ok(None) => {}
                    Err(e) => errors.push(e),
                }
            }

            if self.enable_incremental {
                match incremental::optimize(model) {
                    Ok(Some(t)) => transformations.push(t),
                    Ok(None) => {}
                    Err(e) => errors.push(e),
                }
            }
        }

        // Run Python rules after Rust rules
        #[cfg(feature = "python")]
        if self.enable_python_rules {
            let models: Vec<_> = graph.models().collect();
            let (py_transformations, py_errors) = crate::python_bridge::run_python_rules(&models);
            transformations.extend(py_transformations);
            errors.extend(py_errors);
        }

        (transformations, errors)
    }
}

impl Default for Optimizer {
    fn default() -> Self {
        Self::new()
    }
}
