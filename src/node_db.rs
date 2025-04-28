use reth_db::{mdbx::{Env, EnvironmentKind, WriteMap}, DatabaseEnv};
use reth_db::tables;
use reth_db::Table;
use std::path::Path;
use anyhow::Result;

pub struct NodeDB {
    env: DatabaseEnv<>,
}

impl NodeDB {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let env = DatabaseEnv::<WriteMap>::open(
            path,
            reth_db::mdbx::EnvironmentFlags::empty(),
            None,
        )?;

        Ok(Self { env })
    }

    pub fn env(&self) -> &DatabaseEnv<> {
        &self.env
    }
}

// Example type your code was using
#[derive(Debug, Clone, Copy)]
pub enum InsertionType {
    Replace,
    Merge,
}
