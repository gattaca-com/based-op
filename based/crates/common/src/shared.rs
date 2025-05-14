use crate::db::DBFrag;

/// Shared state between Sequencer and RPC
/// Allows for access to the State and Receipts
/// Receipts and State are updated when a frag gets sealed
#[derive(Clone, Debug)]
pub struct SharedState<Db> {
    db: DBFrag<Db>,
}

impl<Db> SharedState<Db> {
    pub fn new(db: DBFrag<Db>) -> Self {
        Self { db }
    }

    /// Resets the pending state its holding for live blocks that are being built.
    /// Will be called when the block is committed as this state will now be available in the EL node.
    pub fn reset(&mut self) {
        self.db.reset();
    }
}

impl<Db: Clone> From<&SharedState<Db>> for DBFrag<Db> {
    fn from(value: &SharedState<Db>) -> Self {
        value.db.clone()
    }
}

impl<Db> AsMut<DBFrag<Db>> for SharedState<Db> {
    fn as_mut(&mut self) -> &mut DBFrag<Db> {
        &mut self.db
    }
}

impl<Db> AsRef<DBFrag<Db>> for SharedState<Db> {
    fn as_ref(&self) -> &DBFrag<Db> {
        &self.db
    }
}
