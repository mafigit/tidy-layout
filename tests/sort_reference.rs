// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Go's `sort.Slice`, as recorded from Go (`docs/reference.md`): where keys are equal, the port
//! must leave them in the order Go leaves them.

use tidy_layout::gosort::sort_slice;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    keys: Vec<i64>,
    sorted: Vec<usize>,
}

#[test]
fn equal_keys_end_in_the_order_go_leaves_them_in() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/sort.go.json")).unwrap();
    assert_eq!(cases.len(), 90);
    let mut moved_equal = 0;
    for (number, case) in cases.iter().enumerate() {
        let mut places: Vec<usize> = (0..case.keys.len()).collect();
        sort_slice(&mut places, |a, b| case.keys[*a] < case.keys[*b]);
        assert_eq!(places, case.sorted, "case {number}: {} keys", case.keys.len());
        // A sort that keeps equal keys in place would give another order here.
        let mut stable: Vec<usize> = (0..case.keys.len()).collect();
        stable.sort_by_key(|place| case.keys[*place]);
        moved_equal += usize::from(stable != places);
    }
    assert_eq!(moved_equal, 42);
}
