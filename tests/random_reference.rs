// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Go's random numbers, as recorded from Go (`docs/reference.md`).

use tidy_layout::gorand::GoRand;
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    seed: i64,
    first: Vec<i64>,
    later: Vec<i64>,
    floats: Vec<f64>,
    uint32s: Vec<u32>,
    shuffled: Vec<usize>,
}

#[test]
fn the_same_seed_gives_the_numbers_go_gives() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/random.go.json")).unwrap();
    assert_eq!(cases.len(), 10);
    for case in &cases {
        let mut source = GoRand::new(case.seed);
        let numbers: Vec<i64> = (0..704).map(|_| source.int63()).collect();
        assert_eq!(numbers[..5], case.first, "seed {}", case.seed);
        assert_eq!(numbers[699..], case.later, "seed {}", case.seed);
        assert_eq!(source.drawn, 704);

        let mut random = GoRand::new(case.seed);
        let floats: Vec<f64> = (0..3).map(|_| random.float64()).collect();
        assert_eq!(floats, case.floats, "seed {}", case.seed);
        let uint32s: Vec<u32> = (0..3).map(|_| random.uint32()).collect();
        assert_eq!(uint32s, case.uint32s, "seed {}", case.seed);
        let mut shuffled: Vec<usize> = (0..12).collect();
        random.shuffle(&mut shuffled);
        assert_eq!(shuffled, case.shuffled, "seed {}", case.seed);
    }
}
