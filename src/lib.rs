use std::collections::BTreeMap;
use std::ops::{Bound, Range, RangeBounds};

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
        self.inner
            .range((Bound::Unbounded, start_bound))
            .next_back()
            .filter(|(k, v)| match start_bound {
                // already included in the next range
                Bound::Unbounded => false,
                Bound::Included(i) => **k + v.len > i,
                Bound::Excluded(e) => **k + v.len >= e,
            })
            .map_or(start_bound, |(k, _)| Bound::Included(*k))
    }
    pub fn conflicts(
        &self,
        range: impl RangeBounds<K>,
    ) -> impl Iterator<Item = (Range<K>, &V)> + '_ {
        let real_start_bound = self.real_start_bound(range.start_bound().cloned());
        let is_empty = real_start_bound == range.end_bound().cloned();
        //dbg!(range.start_bound(), range.end_bound(), is_empty, real_start_bound);

        (!is_empty)
            .then(|| {
                self.inner
                    .range((real_start_bound, range.end_bound().cloned()))
                    .map(|(k, v)| (*k..*k + v.len, &v.value))
            })
            .into_iter()
            .flatten()
    }
    pub fn conflicts_mut(
        &mut self,
        range: impl RangeBounds<K>,
    ) -> impl Iterator<Item = (Range<K>, &mut V)> + '_ {
        let real_start_bound = self.real_start_bound(range.start_bound().cloned());

        self.inner
            .range_mut((real_start_bound, range.end_bound().cloned()))
            .map(|(k, v)| (*k..*k + v.len, &mut v.value))
    }
    pub fn contains(&self, key: K) -> bool {
        self.conflicts(key..=key).next().is_some()
    }
    pub fn insert(&mut self, mut start: K, mut len: K, mut value: V) -> Result<(), Occupied> {
        if self.conflicts(start..start + len).next().is_some() {
            return Err(Occupied);
        }

        if let Some((prev_base, prev)) = self.inner.range(..start).next_back() {
            if *prev_base + prev.len == start {
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

        //dbg!(self.conflicts(range.clone()).collect::<Vec<_>>());
        loop {
            let first_conflict_key = {
                self.conflicts(range.clone())
                    .next()
                    .map(|(part_range_ref, _)| part_range_ref.start)
            };

            let k = match first_conflict_key {
                Some(k) => k,
                None => break,
            };

            let Slot { len, value } = self.inner.remove(&k).unwrap();
            let part_range = k..k + len;

            let (range_before, range_to_remove, range_after) =
                extract(range.clone(), part_range.clone());
            let (v_before, value, v_after) = value.split(
                range_before.clone(),
                range_to_remove.clone(),
                range_after.clone(),
            );

            if let (Some(rb), Some(vb)) = (range_before, v_before) {
                let _ = self.insert(rb.start, rb.end - rb.start, vb);
            }
            if let (Some(ra), Some(va)) = (range_after, v_after) {
                let _ = self.insert(ra.start, ra.end - ra.start, va);
            }

            ret.push((range_to_remove, value));
        }
        //dbg!(&ret);

        ret
    }
    pub fn remove_and_unused(&mut self, range: Range<K>) -> Vec<(Range<K>, Option<V>)> {
        let removed = self.remove(range.clone());
        let mut ret = Vec::new();

        let mut offset = range.start;

        for (r, v) in removed {
            if r.start > offset {
                ret.push((offset..r.start, None));
            }

            offset = r.end;
            ret.push((r, Some(v)));
        }

        if offset < range.end {
            ret.push((offset..range.end, None));
        }

        ret
    }
    pub fn end(&self) -> K {
        self.inner
            .last_key_value()
            .map_or(K::from(0), |(k, v)| *k + v.len)
    }
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn iter(&self) -> impl Iterator<Item = (Range<K>, &V)> + '_ {
        self.inner
            .iter()
            .map(|(k, Slot { len, value })| (*k..*k + *len, value))
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Range<K>, &mut V)> + '_ {
        self.inner
            .iter_mut()
            .map(|(k, Slot { len, value })| (*k..*k + *len, value))
    }
}

fn extract<K: Ord + Copy + std::fmt::Debug>(
    outer: Range<K>,
    inner: Range<K>,
) -> (Option<Range<K>>, Range<K>, Option<Range<K>>) {
    //dbg!(&outer, &inner);
    let before = if inner.start < outer.start {
        Some(inner.start..outer.start)
    } else {
        None
    };

    let middle = core::cmp::max(inner.start, outer.start)..core::cmp::min(inner.end, outer.end);

    let after = if inner.end > outer.end {
        Some(outer.end..inner.end)
    } else {
        None
    };

    (before, middle, after)
}

#[derive(Debug)]
pub struct Occupied;

pub trait Key:
    std::ops::Add<Output = Self>
    + std::ops::Sub<Output = Self>
    + std::ops::AddAssign
    + Copy
    + Ord
    + From<u8>
    + std::fmt::Debug
{
}
impl Key for u64 {}
impl Key for usize {}

pub trait Value: Sized + std::fmt::Debug {
    type K: Key;

    fn try_merge_backwards(self, other: &Self) -> Result<Self, Self>;
    fn try_merge_forward(self, other: &Self) -> Result<Self, Self>;

    fn split(
        self,
        prev_range: Option<Range<Self::K>>,
        range: Range<Self::K>,
        next_range: Option<Range<Self::K>>,
    ) -> (Option<Self>, Self, Option<Self>);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    #[derive(Debug, Clone, PartialEq)]
    struct TestFmap {
        rc: usize,
    }

    impl Value for TestFmap {
        type K = u64;

        fn try_merge_forward(self, other: &Self) -> Result<Self, Self> {
            if self.rc == other.rc {
                Ok(self)
            } else {
                Err(self)
            }
        }

        fn try_merge_backwards(self, other: &Self) -> Result<Self, Self> {
            if self.rc == other.rc {
                Ok(self)
            } else {
                Err(self)
            }
        }

        fn split(
            self,
            prev_range: Option<Range<Self::K>>,
            _range: Range<Self::K>,
            next_range: Option<Range<Self::K>>,
        ) -> (Option<Self>, Self, Option<Self>) {
            (
                prev_range.map(|_| self.clone()),
                self.clone(),
                next_range.map(|_| self.clone()),
            )
        }
    }

    #[test]
    fn test_unmap_middle() {
        let mut tree = RangeTree::new();

        // Map 0..3000
        tree.insert(0, 3000, TestFmap { rc: 1 }).unwrap();

        // Unmap 1000..2000
        let removed = tree.remove(1000..2000);
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, 1000..2000);

        // Must be 0..1000 and 2000..3000
        let ranges: Vec<_> = tree.iter().map(|(r, v)| (r, v.rc)).collect();
        assert_eq!(ranges, vec![(0..1000, 1), (2000..3000, 1)]);
    }

    #[test]
    fn test_refcount_split_and_merge() {
        // Map 0..3000
        let mut tree = RangeTree::new();
        tree.insert(0, 3000, TestFmap { rc: 1 }).unwrap();

        // Split 1000..2000
        let removed = tree.remove(1000..2000);
        for (range, mut val) in removed {
            val.rc += 1;
            tree.insert(range.start, range.end - range.start, val)
                .unwrap();
        }

        let ranges: Vec<_> = tree.iter().map(|(r, v)| (r, v.rc)).collect();
        assert_eq!(ranges, vec![(0..1000, 1), (1000..2000, 2), (2000..3000, 1)]);

        // Merge 1000..2000
        let removed = tree.remove(1000..2000);
        for (range, mut val) in removed {
            val.rc -= 1;
            tree.insert(range.start, range.end - range.start, val)
                .unwrap();
        }

        let ranges: Vec<_> = tree.iter().map(|(r, v)| (r, v.rc)).collect();
        assert_eq!(ranges, vec![(0..3000, 1)]);
    }

    #[test]
    fn test_remove_and_unused_with_holes() {
        // Map 1000..3000
        let mut tree = RangeTree::new();
        tree.insert(1000, 2000, TestFmap { rc: 1 }).unwrap();

        // Unmap 0..4000
        let affected = tree.remove_and_unused(0..4000);

        assert_eq!(affected.len(), 3);

        assert_eq!(affected[0].0, 0..1000);
        assert!(affected[0].1.is_none());

        assert_eq!(affected[1].0, 1000..3000);
        assert_eq!(affected[1].1.as_ref().unwrap().rc, 1);

        assert_eq!(affected[2].0, 3000..4000);
        assert!(affected[2].1.is_none());
    }

    #[test]
    fn test_overlapping_removals() {
        // Map 0..2000, 2000, 4000
        let mut tree = RangeTree::new();
        tree.insert(0, 2000, TestFmap { rc: 1 }).unwrap();
        tree.insert(2000, 2000, TestFmap { rc: 2 }).unwrap();

        // Unmap 1000..3000
        let removed = tree.remove(1000..3000);

        assert_eq!(removed.len(), 2);
        assert_eq!(removed[0].0, 1000..2000);
        assert_eq!(removed[0].1.rc, 1);
        assert_eq!(removed[1].0, 2000..3000);
        assert_eq!(removed[1].1.rc, 2);

        let ranges: Vec<_> = tree.iter().map(|(r, v)| (r, v.rc)).collect();
        assert_eq!(ranges, vec![(0..1000, 1), (3000..4000, 2)]);
    }
}
