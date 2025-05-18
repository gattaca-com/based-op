use std::collections::HashMap;

#[auto_impl(&, Rc, Arc, Box)]
pub trait HasKey {
    type Key: std::hash::Hash + Clone + Eq + for<'a> Deserialize<'a> + Serialize;
    fn key(&self) -> &Self::Key;
}

impl<T: HasKey> HasKey for Vec<T> {
    type Key = T::Key;

    fn key(&self) -> &Self::Key {
        self[0].key()
    }
}

use auto_impl::auto_impl;
use serde::{Deserialize, Serialize};

/// Basically a ringbuffer to be used with data that gets streamed in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircularBuffer<T> {
    data: Vec<T>,
    mask: usize,
    // Keep track of which pos is current begin
    filled: bool,
    // Which slot to fill NEXT, i.e. id of the first element in buffer
    pos: usize,
}

impl<T: Default + Clone> CircularBuffer<T> {
    pub fn new(size: usize) -> Self {
        let realsize = size.next_power_of_two();
        Self { data: vec![T::default(); realsize], mask: realsize - 1, filled: false, pos: 0 }
    }
}
impl<T> CircularBuffer<T> {
    /// If the ringbuffer is full this returns the first element, i.e. that will be discarded
    #[inline]
    pub fn push(&mut self, v: T) -> Option<T> {
        let o = if self.filled {
            Some(std::mem::replace(unsafe { self.data.get_unchecked_mut(self.pos) }, v))
        } else {
            unsafe { *self.data.get_unchecked_mut(self.pos) = v };
            None
        };
        self.pos = (self.pos + 1) & self.mask;
        self.filled |= self.pos == 0;
        o
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        Iter::new(self)
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut::new(self)
    }

    #[inline]
    pub fn len(&self) -> usize {
        if self.filled { self.data.len() } else { self.pos }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.pos = 0;
        self.filled = false;
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn last(&self) -> Option<&T> {
        (!self.is_empty()).then(|| {
            let pos = if self.pos > 0 { self.pos - 1 } else { self.mask };
            unsafe { self.data.get_unchecked(pos) }
        })
    }

    #[inline]
    pub fn last_mut(&mut self) -> Option<&mut T> {
        (!self.is_empty()).then(|| {
            let pos = if self.pos > 0 { self.pos - 1 } else { self.mask };
            unsafe { self.data.get_unchecked_mut(pos) }
        })
    }

    #[inline]
    pub fn first(&self) -> Option<&T> {
        (!self.is_empty()).then(|| {
            let pos = if self.filled { self.pos } else { 0 };
            unsafe { self.data.get_unchecked(pos) }
        })
    }

    #[inline]
    pub fn first_mut(&mut self) -> Option<&mut T> {
        (!self.is_empty()).then(|| {
            let pos = if self.filled { self.pos } else { 0 };
            unsafe { self.data.get_unchecked_mut(pos) }
        })
    }

    fn first_pos(&self) -> usize {
        if self.filled { self.pos } else { 0 }
    }

    fn last_pos(&self) -> usize {
        if self.pos > 0 { self.pos - 1 } else { self.mask }
    }

    #[inline]
    pub fn nth_back(&self, id: usize) -> &T {
        if id == 0 {
            return self.last().unwrap();
        } else if id >= self.len() {
            panic!("out of bounds");
        }
        let last_filled_id = if self.pos == 0 { self.mask } else { self.pos - 1 };
        if last_filled_id < id {
            &self.data[self.len() - (id - last_filled_id)]
        } else {
            &self.data[last_filled_id - id]
        }
    }

    #[inline]
    pub fn nth_mut(&mut self, id: usize) -> Option<&mut T> {
        if id == 0 {
            return self.first_mut();
        } else if id >= self.len() {
            tracing::info!("id out of bounds");
            return None;
        }
        let pos = if self.filled { (self.pos + id) & self.mask } else { id };
        Some(&mut self.data[pos])
    }

    #[inline]
    pub fn nth(&mut self, id: usize) -> Option<&T> {
        if id == 0 {
            return self.first();
        } else if id >= self.len() {
            tracing::info!("id out of bounds");
            return None;
        }
        let pos = if self.filled { (self.pos + id) & self.mask } else { id };
        Some(&self.data[pos])
    }

    #[inline]
    pub fn filled(&self) -> bool {
        self.filled
    }

    pub(crate) fn n_slots(&self) -> usize {
        self.data.len()
    }

    pub(crate) fn set_index(&mut self, id: usize, val: T) {
        self.data[id] = val
    }

    pub(crate) fn get(&self, id: usize) -> &T {
        &self.data[id]
    }

    pub(crate) fn get_mut(&mut self, id: usize) -> &mut T {
        &mut self.data[id]
    }
}

impl<T: Ord + Clone> CircularBuffer<T> {
    #[inline]
    pub fn median(&mut self) -> T {
        let l = self.len();
        self.data[..l].sort_unstable();
        self.data[self.len() / 2].clone()
    }
}
#[derive(Clone, Debug)]
pub struct Iter<'a, T> {
    // This guy goes around
    pos: usize,
    count: usize,
    back: usize,
    // How many
    buffer: &'a CircularBuffer<T>,
}

impl<'a, T> Iter<'a, T> {
    fn new(buffer: &'a CircularBuffer<T>) -> Self {
        let back = buffer.last_pos();
        Self { pos: buffer.first_pos(), count: 0, buffer, back }
    }
}

#[derive(Debug)]
pub struct IterMut<'a, T> {
    pos: usize,
    count: usize,
    // How many
    buffer: &'a mut CircularBuffer<T>,
}

impl<'a, T> IterMut<'a, T> {
    fn new(buffer: &'a mut CircularBuffer<T>) -> Self {
        Self { pos: buffer.first_pos(), count: 0, buffer }
    }
}

impl<'a, T: Copy> IntoIterator for &'a CircularBuffer<T> {
    type IntoIter = Iter<'a, T>;
    type Item = &'a T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T: Copy> IntoIterator for &'a mut CircularBuffer<T> {
    type IntoIter = IterMut<'a, T>;
    type Item = &'a mut T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}
impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.count == self.buffer.len() {
            return None;
        }

        let out = unsafe { self.buffer.data.get_unchecked(self.pos) };
        self.pos = (self.pos + 1) & self.buffer.mask;
        self.count += 1;
        Some(out)
    }
}
impl<T> ExactSizeIterator for Iter<'_, T> {
    fn len(&self) -> usize {
        self.buffer.len()
    }
}

impl<T> DoubleEndedIterator for Iter<'_, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.buffer.is_empty() || self.count == self.buffer.len() {
            None
        } else {
            let out = unsafe { self.buffer.data.get_unchecked(self.back) };
            self.back = if self.back == 0 { self.buffer.mask } else { self.back - 1 };
            self.count += 1;
            Some(out)
        }
    }
}

impl<'a, T> Iterator for IterMut<'a, T>
where
    T: 'a,
{
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.count == self.buffer.len() {
            return None;
        }

        let out = unsafe { self.buffer.data.get_unchecked_mut(self.pos) };
        self.pos = (self.pos + 1) & self.buffer.mask;
        self.count += 1;
        unsafe { Some(&mut *(out as *mut T)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circular_buffer_basic() {
        let mut buf = CircularBuffer::new(32);
        let mut tot = 0;
        for i in 0..30 {
            buf.push(i);
            tot += i;
        }

        assert_eq!(tot, buf.iter().sum::<i32>());

        let mut buf = CircularBuffer::new(32);
        let mut tot = 0;
        for i in 1..36 {
            buf.push(i);
            if i > 3 {
                tot += i;
            }
        }
        assert_eq!(tot, buf.iter().sum::<i32>());
    }

    #[test]
    fn circular_buffer_nth_back() {
        let mut buf = CircularBuffer::new(32);
        buf.push(1u64);
        assert_eq!(*buf.nth_back(0), 1u64);
        buf.push(2u64);
        assert_eq!(*buf.nth_back(0), 2u64);
        assert_eq!(*buf.nth_back(1), 1u64);
    }

    #[test]
    fn circular_buffer_nth_mut() {
        let mut buf = CircularBuffer::new(4);
        buf.push(1u64);
        assert_eq!(*buf.nth_mut(0).unwrap(), 1u64);
        buf.push(2u64);
        assert_eq!(*buf.nth_mut(0).unwrap(), 1u64);
        assert_eq!(*buf.nth_mut(1).unwrap(), 2u64);
        buf.push(3u64);
        buf.push(4u64);
        assert_eq!(*buf.nth_mut(2).unwrap(), 3u64);
        assert_eq!(*buf.nth_mut(3).unwrap(), 4u64);
        buf.push(5u64);
        assert_eq!(*buf.nth_mut(0).unwrap(), 2u64);
    }

    #[test]
    fn circular_buffer_circular_iterators() {
        let mut buf = CircularBuffer::new(32);
        buf.push(1u64);

        assert_eq!(buf.iter().sum::<u64>(), 1u64);
        buf.push(2u64);
        assert_eq!(buf.iter().sum::<u64>(), 3u64);

        buf.iter_mut().for_each(|v| *v = 2);
        assert_eq!(buf.iter().sum::<u64>(), 4u64);
        assert_eq!(buf.iter().rev().sum::<u64>(), 4u64);
        let mut buf = CircularBuffer::new(2);
        buf.push(2u64);
        buf.push(2u64);
        buf.push(1u64);
        assert_eq!(buf.iter().sum::<u64>(), 3u64);
        assert_eq!(buf.iter().next(), Some(&2u64));
        assert_eq!(buf.iter().rev().next(), Some(&1u64));
        assert_eq!(buf.iter().rev().sum::<u64>(), 3u64);

        buf.iter_mut().for_each(|v| *v = 2);
        assert_eq!(buf.iter().sum::<u64>(), 4u64);
        assert_eq!(buf.iter().rev().sum::<u64>(), 4u64);
    }
}
/// CircularBuffer which can be accessed with hashed keys as well.
#[derive(Debug, Clone)]
pub struct KeyedCircularBuffer<V: HasKey> {
    data: CircularBuffer<V>,
    ids: HashMap<V::Key, usize>,
    count: usize,
}

impl<T: Default + Clone + HasKey> KeyedCircularBuffer<T> {
    pub fn new(size: usize) -> Self {
        Self { data: CircularBuffer::new(size), ids: HashMap::new(), count: 0 }
    }

}

impl<T: HasKey> KeyedCircularBuffer<T> {
    /// Optionally returns the removed item from the ringbuffer
    #[inline]
    pub fn insert(&mut self, v: T) -> Option<T> {
        let key = v.key();
        if let Some(slot) = self.ids.get_mut(key) {
            self.data.set_index(*slot % self.data.n_slots(), v);
            None
        } else {
            let id = self.count;
            self.ids.insert(key.clone(), id);
            if self.count >= self.data.n_slots() - 1 {
                self.ids.remove(self.data.first().unwrap().key());
            }
            let o = self.data.push(v);
            self.count += 1;
            o
        }
    }

    #[inline]
    pub fn iter(&self) -> Iter<'_, T> {
        self.data.iter()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    #[inline]
    pub fn get(&self, k: &T::Key) -> Option<&T> {
        self.ids.get(k).map(|id| self.data.get(*id % self.data.n_slots()))
    }

    #[inline]
    pub fn get_mut(&mut self, k: &T::Key) -> Option<&mut T> {
        self.ids.get(k).map(|id| self.data.get_mut(*id % self.data.n_slots()))
    }

    #[inline]
    pub fn contains_key(&self, key: &T::Key) -> bool {
        self.ids.contains_key(key)
    }
}
