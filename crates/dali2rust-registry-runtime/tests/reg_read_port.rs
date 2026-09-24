use dali2rust_domain::registry::RegistryReadPort;
use dali2rust_registry_runtime::RegistryStore;

#[test]
fn fresh_store_virtual_lamp_defaults_reg001() {
    let store = RegistryStore::with_adapter_count(1);
    let snap = store.virtual_lamp_snapshot(0, 7);
    assert_eq!(snap.name, "");
    assert_eq!(snap.runtime_level, 0);
}
