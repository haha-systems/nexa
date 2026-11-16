//! Descriptor only: registering or executing these operations arrives in M1/M2.

use crate::{
    contracts::*,
    diagnostic::{Error, Result},
    digest::Digest,
    value::{Identifier, ValueType},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KernelOperation {
    pub name: String,
    pub contract: CallableContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KernelDescriptor {
    pub schema: KernelSchema,
    pub language: Language,
    pub profile: Profile,
    pub name: String,
    pub version: String,
    pub abi: String,
    pub implementation: Digest,
    pub operations: Vec<KernelOperation>,
}

impl KernelDescriptor {
    pub fn bundled() -> Result<Self> {
        let string_arg = |name| -> Result<Port> {
            Ok(Port {
                name: Identifier::new(name)?,
                r#type: ValueType::String,
            })
        };
        let mut descriptor = Self {
            schema: KernelSchema::V03,
            language: Language::V03,
            profile: Profile::Slice0,
            name: "std.slice0".into(),
            version: "0.3.0".into(),
            abi: "nexa.slice0.kernel/0.3".into(),
            implementation: env!("NEXA_IMPLEMENTATION_DIGEST").parse()?,
            operations: vec![
                KernelOperation {
                    name: "std.text.trim_ascii".into(),
                    contract: CallableContract {
                        export_name: Identifier::new("trim_ascii")?,
                        kind: CallableKind::Function,
                        parameters: vec![string_arg("text")?],
                        result_type: ValueType::String,
                        effects: vec![],
                        required_bindings: vec![],
                        capability_requirements: vec![],
                    },
                },
                KernelOperation {
                    name: "std.console.write_line".into(),
                    contract: CallableContract {
                        export_name: Identifier::new("write_line")?,
                        kind: CallableKind::Action,
                        parameters: vec![string_arg("text")?],
                        result_type: ValueType::Unit,
                        effects: vec!["io.stdout".into()],
                        required_bindings: vec!["std.console.write_line".into()],
                        capability_requirements: vec![CapabilityRequest {
                            capability: "console.write".into(),
                            resource: "stdout".into(),
                        }],
                    },
                },
            ],
        };
        descriptor.normalize()?;
        Ok(descriptor)
    }
}

impl WireDocument for KernelDescriptor {
    fn normalize(&mut self) -> Result<()> {
        let mut seen = BTreeSet::new();
        for operation in &mut self.operations {
            if !seen.insert(&operation.name) {
                return Err(Error::new("E_DUPLICATE", "duplicate kernel operation"));
            }
            operation.contract.normalize()?;
        }
        self.operations.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }
}
