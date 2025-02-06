/// Basically a ringbuffer to be used with data that gets streamed in.
#[derive(Debug, Clone)]
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
        Self { data: vec![T::default(); size], mask: realsize - 1, filled: false, pos: 0 }
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
        if self.filled {
            self.data.len()
        } else {
            self.pos
        }
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
        if self.filled {
            self.pos
        } else {
            0
        }
    }

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
    // How many
    buffer: &'a CircularBuffer<T>,
}

impl<'a, T> Iter<'a, T> {
    fn new(buffer: &'a CircularBuffer<T>) -> Self {
        Self { pos: buffer.first_pos(), count: 0, buffer }
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
        let out = unsafe { self.buffer.data.get_unchecked(self.pos & self.buffer.mask) };
        self.pos = (self.pos + 1) & self.buffer.mask;
        self.count += 1;
        Some(out)
    }
}
impl<'a, T> ExactSizeIterator for Iter<'a, T> {
    fn len(&self) -> usize {
        self.buffer.len()
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

        let out = unsafe { self.buffer.data.get_unchecked_mut(self.pos & self.buffer.mask) };
        self.pos = (self.pos + 1) & self.buffer.mask;
        self.count += 1;
        unsafe { Some(&mut *(out as *mut T)) }
    }
}

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test() {
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
    fn nth_back() {
        let mut buf = CircularBuffer::new(32);
        buf.push(1u64);
        assert_eq!(*buf.nth_back(0), 1u64);
        buf.push(2u64);
        assert_eq!(*buf.nth_back(0), 2u64);
        assert_eq!(*buf.nth_back(1), 1u64);
    }

    #[test]
    fn circular_iterators() {
        let mut buf = CircularBuffer::new(32);
        buf.push(1u64);

        assert_eq!(buf.iter().sum::<u64>(), 1u64);
        buf.push(2u64);
        assert_eq!(buf.iter().sum::<u64>(), 3u64);

        buf.iter_mut().for_each(|v| *v = 2);
        assert_eq!(buf.iter().sum::<u64>(), 4u64);
    }
}
