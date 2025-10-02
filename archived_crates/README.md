# Archived Crates

These crates were part of the initial ChatSafe design but are no longer in use.
They have been moved here for historical reference.

## infer-runtime
- **Purpose**: Alternative inference runtime implementation
- **Status**: Superseded by the runtime crate with llama_adapter
- **Reason for archival**: The project migrated to using llama.cpp server via HTTP instead of managing processes directly

## store
- **Purpose**: Persistent storage trait for conversation history
- **Status**: Stub implementation only (NoOpStore)
- **Reason for archival**: Feature not yet needed; ChatSafe is currently stateless

These crates can be restored if needed in the future by:
1. Moving them back to `crates/`
2. Re-adding them to the workspace in `Cargo.toml`