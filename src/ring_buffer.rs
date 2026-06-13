use std::collections::VecDeque;

#[derive(Debug)]
pub struct RingBuffer<T> {
    pub buf: VecDeque<T>,
    pub capacity: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, value: T) {
        if self.buf.len() == self.capacity {
            self.buf.pop_front();
        }
        self.buf.push_back(value);
    }

    pub fn front(&self) -> Option<&T> {
        self.buf.front()
    }

    pub fn size(&self) -> usize {
        self.buf.len()
    }
}
