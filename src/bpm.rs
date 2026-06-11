use std::collections::{HashMap, VecDeque};

use crate::disk_manager::{DiskManager, Page};
use crate::ring_buffer::RingBuffer;

const K: usize = 8;
const POOL_SIZE: usize = 20;

#[derive(Debug, Clone)]
struct Frame {
    page: Page,
    dirty: bool,
    pin_count: u16,
    disk_manager_page_id: u64,
}

struct BufferPoolManager {
    acces_counter: usize,
    buffer_pool: Vec<Frame>,
    disk_manager: DiskManager,
    free_slots: VecDeque<usize>,
    pool_index: HashMap<usize, usize>,
    lru_k_history: HashMap<usize, RingBuffer<usize>>,
}

impl BufferPoolManager {
    fn new(db: &str) -> Self {
        let free_slots: VecDeque<_> = (0..POOL_SIZE).collect();

        let buffer_pool = (0..POOL_SIZE)
            .map(|_| Frame {
                page: Page::new(),
                dirty: false,
                pin_count: 0,
                disk_manager_page_id: 0,
            })
            .collect();

        BufferPoolManager {
            acces_counter: 0,
            free_slots: free_slots,
            buffer_pool: buffer_pool,
            pool_index: HashMap::new(),
            lru_k_history: HashMap::new(),
            disk_manager: DiskManager::new(db).unwrap(),
        }
    }

    fn evict_frame_index(&mut self, idx: &usize) -> () {
        if self.buffer_pool[*idx].dirty {
            self.disk_manager
                .write_page(
                    // The assumption here is, if some page is dirty and is being evicted, it's disk_manager_page_id
                    // at some point, has been set to the right value, so this will work
                    self.buffer_pool[*idx].disk_manager_page_id,
                    self.buffer_pool[*idx].page.as_bytes(),
                )
                .unwrap();
        }

        self.lru_k_history.remove(idx);
        self.pool_index
            .remove(&(self.buffer_pool[*idx].disk_manager_page_id as usize));
        self.buffer_pool[*idx] = Frame {
            page: Page::new(),
            dirty: false,
            pin_count: 0,
            disk_manager_page_id: 0,
        };
    }

    fn find_or_create_page_slot(&mut self) -> usize {
        match self.free_slots.pop_front() {
            None => {
                let unpinned_frame_indexes: Vec<usize> = self
                    .buffer_pool
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.pin_count == 0)
                    .map(|(i, _)| i)
                    .collect();

                let lru_k_unpinned = unpinned_frame_indexes
                    .iter()
                    .map(|&frame_idx| (frame_idx, &self.lru_k_history[&frame_idx]));

                let lru_k_under_capacity = lru_k_unpinned.clone().filter(|(i, rb)| rb.capacity < K);

                if lru_k_under_capacity.clone().collect::<Vec<_>>().len() > 0 {
                    let (frame_index_to_evict, _) = lru_k_under_capacity
                        .map(|(f_idx, r)| (f_idx, r.front().unwrap()))
                        .min_by_key(|(_, lru)| **lru)
                        .unwrap();

                    self.evict_frame_index(&frame_index_to_evict);

                    frame_index_to_evict
                } else {
                    let (frame_index_to_evict, _) = lru_k_unpinned
                        .map(|(i, rb)| (i, rb.front().unwrap()))
                        .min_by_key(|(_, lru)| **lru)
                        .unwrap();

                    self.evict_frame_index(&frame_index_to_evict);

                    frame_index_to_evict
                }
            }
            f_res => match f_res {
                Some(s) => s,
                None => {
                    panic!("unreachable")
                }
            },
        }
    }

    pub fn fetch_page(&mut self, page_idx: usize) -> &Page {
        let acces_counter = self.acces_counter;

        match self.pool_index.get(&page_idx) {
            Some(bp_idx) => {
                let idx = *bp_idx;
                self.buffer_pool[idx].pin_count += 1;
                self.lru_k_history
                    .get_mut(&page_idx)
                    .unwrap()
                    .push(acces_counter);
                self.acces_counter += 1;
                &self.buffer_pool[idx].page
            }
            None => {
                let free_slot_index = self.find_or_create_page_slot();
                let page = self.disk_manager.read_page(page_idx as u64).unwrap();
                self.buffer_pool[free_slot_index] = Frame {
                    page: page,
                    dirty: false,
                    pin_count: 1,
                    disk_manager_page_id: page_idx as u64,
                };
                self.pool_index.insert(page_idx, free_slot_index);

                let mut rb: RingBuffer<usize> = RingBuffer::new(K);
                rb.push(acces_counter);
                self.lru_k_history.insert(page_idx, rb);

                self.acces_counter += 1;

                &self.buffer_pool[free_slot_index].page
            }
        }
    }

    pub fn unpin_page(&mut self, page_idx: usize, dirty: bool) -> bool {
        let Some(&bp_idx) = self.pool_index.get(&page_idx) else {
            return false;
        };

        let Some(frame) = self.buffer_pool.get_mut(bp_idx) else {
            return false;
        };

        if frame.pin_count == 0 {
            return false;
        }

        frame.pin_count -= 1;
        frame.dirty |= dirty;

        true
    }
}
