#![feature(let_chains)]

use std::collections::BTreeMap;
use std::ops::{Range, RangeBounds, Bound};

#[derive(Debug)]
pub struct RangeTree<V: Value> {
    inner: BTreeMap<V::K, Slot<V>>,
}

#[derive(Debug)]
struct Slot<V: Value> {
    len: V::K,
    value: V,
}

impl<K: Key + 'static, V: Value<K = K> + 'static> RangeTree<V> {
    pub const fn new() -> Self {
        Self {
            inner: BTreeMap::new(),
        }
    }
    fn real_start_bound(&self, start_bound: Bound<K>) -> Bound<K> {
        self.inner.range((Bound::Unbounded, start_bound))
            .next_back()
            .filter(|(k, v)| match start_bound {
                // already included in the next range
                Bound::Unbounded => false,
                Bound::Included(i) => **k + v.len > i,
                Bound::Excluded(e) => **k + v.len >= e,
            })
            .map_or(start_bound, |(k, _)| Bound::Included(*k))
    }
    pub fn conflicts(&self, range: impl RangeBounds<K>) -> impl Iterator<Item = (Range<K>, &V)> + '_ {
        let real_start_bound = self.real_start_bound(range.start_bound().cloned());
        let is_empty = real_start_bound == range.end_bound().cloned();
        //dbg!(range.start_bound(), range.end_bound(), is_empty, real_start_bound);

        (!is_empty).then(|| self.inner.range((real_start_bound, range.end_bound().cloned())).map(|(k, v)| (*k..*k + v.len, &v.value))).into_iter().flatten()
    }
    pub fn conflicts_mut(&mut self, range: impl RangeBounds<K>) -> impl Iterator<Item = (Range<K>, &mut V)> + '_ {
        let real_start_bound = self.real_start_bound(range.start_bound().cloned());

        self.inner.range_mut((real_start_bound, range.end_bound().cloned())).map(|(k, v)| (*k..*k + v.len, &mut v.value))
    }
    pub fn contains(&self, key: K) -> bool {
        self.conflicts(key..=key).next().is_some()
    }
    pub fn insert(&mut self, mut start: K, mut len: K, mut value: V) -> Result<(), Occupied> {
        if self.conflicts(start..start + len).next().is_some() {
            return Err(Occupied);
        }

        if let Some((prev_base, prev)) = self.inner.range(..start).next_back() && *prev_base + prev.len == start {
            match value.try_merge_backwards(&prev.value) {
                Ok(v) => {
                    len += prev.len;
                    start = *prev_base;
                    value = v;

                    let _ = self.inner.remove(&start).unwrap();
                }
                Err(v) => value = v,
            }
        }
        let end = start + len;
        if let Some(next) = self.inner.get(&end) {
            match value.try_merge_forward(&next.value) {
                Ok(v) => {
                    len += next.len;
                    value = v;

                    let _ = self.inner.remove(&end).unwrap();
                }
                Err(v) => value = v,
            }
        }
        let _ = self.inner.insert(start, Slot { len, value });


        Ok(())
    }
    pub fn remove(&mut self, range: Range<K>) -> Vec<(Range<K>, V)> {
        let mut ret = Vec::new();

        let mut start = range.start_bound().cloned();
        let end = range.end_bound().cloned();
        //dbg!(self.conflicts((start, end)).collect::<Vec<_>>());

        while let Some((part_range_ref, _)) = { let x = self.conflicts((start, end)).next(); x } {
            let k = part_range_ref.start;
            let Slot { len, value } = self.inner.remove(&k).unwrap();
            let part_range = k..k+len;

            let (range_before, range_to_remove, range_after) = extract(range.clone(), part_range.clone());
            let (v_before, value, v_after) = value.split(range_before.clone(), range_to_remove.clone(), range_after.clone());

            if let (Some(range_before), Some(v_before)) = (range_before, v_before) {
                let k = range_before.start;
                let len = range_before.end - range_before.start;
                let _ = self.insert(k, len, v_before);
            }
            if let (Some(range_after), Some(v_after)) = (range_after, v_after) {
                let k = range_after.start;
                let len = range_after.end - range_after.start;
                let _ = self.insert(k, len, v_after);
            }

            ret.push((range_to_remove, value));

            start = part_range.end_bound().cloned();
        }
        //dbg!(&ret);

        ret
    }
    pub fn remove_and_unused(&mut self, range: Range<K>) -> Vec<(Range<K>, Option<V>)> {
        let removed = self.remove(range.clone());
        let mut ret = Vec::new();
        let mut iter = removed.into_iter().peekable();

        let Some((first_range, first_v)) = iter.next() else {
            return vec!((range, None));
        };

        let mut prev_end = first_range.start;

        if first_range.start != range.start {
            ret.push((K::from(0)..first_range.start, None));
        }
        ret.push((first_range.clone(), Some(first_v)));

        while let Some((entry_range, v)) = iter.next() {
            if prev_end != entry_range.start {
                ret.push((prev_end..entry_range.start, None));
            }

            prev_end = entry_range.end;
            ret.push((entry_range, Some(v)));

            if iter.peek().is_none() {
                ret.push((prev_end..range.end, None));
            }
        }

        ret
    }
    pub fn end(&self) -> K {
        self.inner.last_key_value().map_or(K::from(0), |(k, v)| *k + v.len)
    }
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn iter(&self) -> impl Iterator<Item = (Range<K>, &V)> + '_ {
        self.inner.iter().map(|(k, Slot { len, value })| (*k..*k + *len, value))
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Range<K>, &mut V)> + '_ {
        self.inner.iter_mut().map(|(k, Slot { len, value })| (*k..*k + *len, value))
    }
}

fn extract<K: Ord + Copy + std::fmt::Debug>(outer: Range<K>, inner: Range<K>) -> (Option<Range<K>>, Range<K>, Option<Range<K>>) {
    //dbg!(&outer, &inner);
    let smallest_start = core::cmp::min(outer.start, inner.start);
    let greatest_end = core::cmp::max(outer.end, inner.end);

    let middle_start = core::cmp::max(inner.start, outer.start);
    let middle_end = core::cmp::min(inner.end, outer.end);

    let middle_start = core::cmp::min(middle_start, middle_end);
    let middle_end = core::cmp::max(middle_start, middle_end);

    (
        Some(smallest_start..middle_start).filter(|r| !r.is_empty()),
        middle_start..middle_end,
        Some(middle_end..greatest_end).filter(|r| !r.is_empty()),
    )
}

#[derive(Debug)]
pub struct Occupied;

pub trait Key: std::ops::Add<Output = Self> + std::ops::Sub<Output = Self> + std::ops::AddAssign + Copy + Ord + From<u8> + std::fmt::Debug {}
impl Key for u64 {}
impl Key for usize {}

pub trait Value: Sized + std::fmt::Debug {
    type K: Key;

    fn try_merge_backwards(self, other: &Self) -> Result<Self, Self>;
    fn try_merge_forward(self, other: &Self) -> Result<Self, Self>;

    fn split(self, prev_range: Option<Range<Self::K>>, range: Range<Self::K>, next_range: Option<Range<Self::K>>) -> (Option<Self>, Self, Option<Self>);
}
