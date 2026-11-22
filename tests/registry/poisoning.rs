// SPDX-License-Identifier: MIT

use std::panic::catch_unwind;
use std::sync::Mutex;

use super::Registry;

#[test]
fn capacity_panic_leaves_healthy_shards_unpoisoned() {
    let registry = Registry::<3>::new();
    assert!(
        catch_unwind(|| {
            let _guard = registry.shards[1].lock().unwrap();
            panic!("simulated shard failure");
        })
        .is_err()
    );

    assert!(catch_unwind(|| registry.capacity()).is_err());
    assert_eq!(
        registry.shards.each_ref().map(Mutex::is_poisoned),
        [false, true, false]
    );
}
