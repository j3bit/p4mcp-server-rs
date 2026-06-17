use std::collections::BTreeSet;

use crate::{
    config::Toolset,
    error::{P4McpError, Result},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

#[derive(Debug, Clone)]
pub struct SafetyPolicy {
    readonly: bool,
    enabled_toolsets: BTreeSet<Toolset>,
}

impl SafetyPolicy {
    pub fn new(readonly: bool, enabled_toolsets: BTreeSet<Toolset>) -> Self {
        Self {
            readonly,
            enabled_toolsets,
        }
    }

    pub fn check(&self, access: Access, toolset: Toolset, _tool_name: &str) -> Result<()> {
        if !self.enabled_toolsets.contains(&toolset) {
            return Err(P4McpError::ToolsetDisabled {
                toolset: toolset.as_str(),
            });
        }
        if self.readonly && access == Access::Write {
            return Err(P4McpError::Readonly);
        }
        Ok(())
    }
}
