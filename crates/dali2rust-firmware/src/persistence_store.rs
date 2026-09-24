use std::sync::Arc;

use dali2rust_bsp::slice_store_raw::{layout_sectors, RawPartitionSliceStore};
use dali2rust_platform::slice_store::SliceStore;

pub fn mount_persistence_store() -> Option<Arc<dyn SliceStore>> {
    match RawPartitionSliceStore::mount() {
        Ok(store) => {
            log::info!(
                "persistence: raw slice store on `storage` ({} sectors reserved)",
                layout_sectors()
            );
            Some(Arc::new(store))
        }
        Err(e) => {
            log::error!("persistence: slice store unavailable ({e}) — registry stays in memory");
            None
        }
    }
}
