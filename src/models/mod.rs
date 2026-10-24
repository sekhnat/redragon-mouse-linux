//! Model descriptors and the registry that maps device IDs to one.

pub mod descriptor;
pub mod fixture;
pub mod registry;
pub mod validate;

pub use descriptor::{
    ActionKind, Descriptor, DescriptorError, Footprint, Provenance, SUPPORTED_SCHEMA,
};
pub use registry::{LoadError, Registry, Resolution};
