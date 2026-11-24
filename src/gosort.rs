// Copyright 2022 The Go Authors. All rights reserved.
// Use of this source code is governed by a BSD-style
// license that can be found in the LICENSE-GO file.

//! Go's `sort.Slice`: a pattern-defeating quicksort that does not keep equal elements in their
//! order. TALA sorts with it, so where two elements are equal the order Go leaves them in is the
//! order TALA works in. Ported from Go's `sort` package (`zsortfunc.go`, `sort.go`, `slice.go`).

/// Go `sort.Slice`: sorts `data` so that no element is `less` than the one before it.
pub fn sort_slice<T>(data: &mut [T], less: impl Fn(&T, &T) -> bool) {
    let limit = (usize::BITS - data.len().leading_zeros()) as usize;
    let mut sorter = Sorter { data, less };
    let end = sorter.data.len();
    sorter.pdqsort(0, end, limit);
}

#[derive(PartialEq)]
enum Hint {
    Unknown,
    Increasing,
    Decreasing,
}

struct Sorter<'a, T, F: Fn(&T, &T) -> bool> {
    data: &'a mut [T],
    less: F,
}

impl<T, F: Fn(&T, &T) -> bool> Sorter<'_, T, F> {
    fn less(&self, i: usize, j: usize) -> bool {
        (self.less)(&self.data[i], &self.data[j])
    }

    fn insertion_sort(&mut self, a: usize, b: usize) {
        for i in a + 1..b {
            let mut j = i;
            while j > a && self.less(j, j - 1) {
                self.data.swap(j, j - 1);
                j -= 1;
            }
        }
    }

    fn sift_down(&mut self, lo: usize, hi: usize, first: usize) {
        let mut root = lo;
        loop {
            let mut child = 2 * root + 1;
            if child >= hi {
                break;
            }
            if child + 1 < hi && self.less(first + child, first + child + 1) {
                child += 1;
            }
            if !self.less(first + root, first + child) {
                return;
            }
            self.data.swap(first + root, first + child);
            root = child;
        }
    }

    fn heap_sort(&mut self, a: usize, b: usize) {
        let hi = b - a;
        for i in (0..=(hi - 1) / 2).rev() {
            self.sift_down(i, hi, a);
        }
        for i in (0..hi).rev() {
            self.data.swap(a, a + i);
            self.sift_down(0, i, a);
        }
    }

    fn pdqsort(&mut self, mut a: usize, mut b: usize, mut limit: usize) {
        const MAX_INSERTION: usize = 12;
        let mut was_balanced = true;
        let mut was_partitioned = true;
        loop {
            let length = b - a;
            if length <= MAX_INSERTION {
                self.insertion_sort(a, b);
                return;
            }
            // Too many bad pivots: fall back to a sort that cannot go quadratic.
            if limit == 0 {
                self.heap_sort(a, b);
                return;
            }
            if !was_balanced {
                self.break_patterns(a, b);
                limit -= 1;
            }
            let (mut pivot, mut hint) = self.choose_pivot(a, b);
            if hint == Hint::Decreasing {
                self.data[a..b].reverse();
                pivot = (b - 1) - (pivot - a);
                hint = Hint::Increasing;
            }
            if was_balanced && was_partitioned && hint == Hint::Increasing && self.partial_insertion_sort(a, b) {
                return;
            }
            // The element before this stretch equals the pivot: everything equal to it is done.
            if a > 0 && !self.less(a - 1, pivot) {
                a = self.partition_equal(a, b, pivot);
                continue;
            }
            let (mid, already_partitioned) = self.partition(a, b, pivot);
            was_partitioned = already_partitioned;
            let (left, right) = (mid - a, b - mid);
            let balance_threshold = length / 8;
            if left < right {
                was_balanced = left >= balance_threshold;
                self.pdqsort(a, mid, limit);
                a = mid + 1;
            } else {
                was_balanced = right >= balance_threshold;
                self.pdqsort(mid + 1, b, limit);
                b = mid;
            }
        }
    }

    fn partition(&mut self, a: usize, b: usize, pivot: usize) -> (usize, bool) {
        self.data.swap(a, pivot);
        // Go counts with signed numbers; `j` may fall one below `a + 1`.
        let (mut i, mut j) = (a as isize + 1, b as isize - 1);
        let a_ = a as isize;
        let less = |sorter: &Self, x: isize, y: isize| sorter.less(x as usize, y as usize);
        while i <= j && less(self, i, a_) {
            i += 1;
        }
        while i <= j && !less(self, j, a_) {
            j -= 1;
        }
        if i > j {
            self.data.swap(j as usize, a);
            return (j as usize, true);
        }
        self.data.swap(i as usize, j as usize);
        i += 1;
        j -= 1;
        loop {
            while i <= j && less(self, i, a_) {
                i += 1;
            }
            while i <= j && !less(self, j, a_) {
                j -= 1;
            }
            if i > j {
                break;
            }
            self.data.swap(i as usize, j as usize);
            i += 1;
            j -= 1;
        }
        self.data.swap(j as usize, a);
        (j as usize, false)
    }

    fn partition_equal(&mut self, a: usize, b: usize, pivot: usize) -> usize {
        self.data.swap(a, pivot);
        let (mut i, mut j) = (a as isize + 1, b as isize - 1);
        loop {
            while i <= j && !self.less(a, i as usize) {
                i += 1;
            }
            while i <= j && self.less(a, j as usize) {
                j -= 1;
            }
            if i > j {
                break;
            }
            self.data.swap(i as usize, j as usize);
            i += 1;
            j -= 1;
        }
        i as usize
    }

    fn partial_insertion_sort(&mut self, a: usize, b: usize) -> bool {
        const MAX_STEPS: usize = 5;
        const SHORTEST_SHIFTING: usize = 50;
        let mut i = a + 1;
        for _ in 0..MAX_STEPS {
            while i < b && !self.less(i, i - 1) {
                i += 1;
            }
            if i == b {
                return true;
            }
            if b - a < SHORTEST_SHIFTING {
                return false;
            }
            self.data.swap(i, i - 1);
            if i - a >= 2 {
                let mut j = i - 1;
                while j >= 1 {
                    if !self.less(j, j - 1) {
                        break;
                    }
                    self.data.swap(j, j - 1);
                    j -= 1;
                }
            }
            if b - i >= 2 {
                for j in i + 1..b {
                    if !self.less(j, j - 1) {
                        break;
                    }
                    self.data.swap(j, j - 1);
                }
            }
        }
        false
    }

    fn break_patterns(&mut self, a: usize, b: usize) {
        let length = b - a;
        if length >= 8 {
            let mut random = length as u64;
            let modulus = 1_usize << (usize::BITS - length.leading_zeros());
            let middle = a + (length / 4) * 2;
            for index in middle - 1..=middle + 1 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let mut other = random as usize & (modulus - 1);
                if other >= length {
                    other -= length;
                }
                self.data.swap(index, a + other);
            }
        }
    }

    fn choose_pivot(&mut self, a: usize, b: usize) -> (usize, Hint) {
        const SHORTEST_NINTHER: usize = 50;
        const MAX_SWAPS: usize = 4 * 3;
        let l = b - a;
        let mut swaps = 0;
        let (mut i, mut j, mut k) = (a + l / 4, a + l / 4 * 2, a + l / 4 * 3);
        if l >= 8 {
            if l >= SHORTEST_NINTHER {
                i = self.median(i - 1, i, i + 1, &mut swaps);
                j = self.median(j - 1, j, j + 1, &mut swaps);
                k = self.median(k - 1, k, k + 1, &mut swaps);
            }
            j = self.median(i, j, k, &mut swaps);
        }
        match swaps {
            0 => (j, Hint::Increasing),
            MAX_SWAPS => (j, Hint::Decreasing),
            _ => (j, Hint::Unknown),
        }
    }

    fn order2(&self, a: usize, b: usize, swaps: &mut usize) -> (usize, usize) {
        if self.less(b, a) {
            *swaps += 1;
            return (b, a);
        }
        (a, b)
    }

    fn median(&self, a: usize, b: usize, c: usize, swaps: &mut usize) -> usize {
        let (a, b) = self.order2(a, b, swaps);
        let (b, _) = self.order2(b, c, swaps);
        let (_, b) = self.order2(a, b, swaps);
        b
    }
}
