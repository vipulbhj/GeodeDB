use std::collections::{HashMap, VecDeque};
use std::ops::{Deref, DerefMut};

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

type PoolIdx = usize;
type PageIdx = usize;

struct BufferPoolManager {
    acces_counter: usize,
    buffer_pool: Vec<Frame>,
    disk_manager: DiskManager,
    free_slots: VecDeque<PoolIdx>,
    pool_index: HashMap<PageIdx, PoolIdx>,
    lru_k_history: HashMap<PageIdx, RingBuffer<usize>>,
}

struct PageGuard<'a> {
    frame: &'a mut Frame,
}

impl<'a> Drop for PageGuard<'a> {
    fn drop(&mut self) {
        self.frame.pin_count -= 1;
    }
}

impl<'a> Deref for PageGuard<'a> {
    type Target = Page;

    fn deref(&self) -> &Self::Target {
        &self.frame.page
    }
}

impl<'a> DerefMut for PageGuard<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.frame.dirty = true;

        &mut self.frame.page
    }
}

impl BufferPoolManager {
    fn new(db: &str) -> Self {
        let free_slots: VecDeque<_> = (0..POOL_SIZE).collect();

        let buffer_pool = (0..POOL_SIZE)
            .map(|_| Frame {
                dirty: false,
                pin_count: 0,
                page: Page::new(),
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
                let unpinned_frame_page_indexes: Vec<PageIdx> = self
                    .buffer_pool
                    .iter()
                    .filter(|f| f.pin_count == 0)
                    .map(|f| f.disk_manager_page_id as usize)
                    .collect();

                let lru_k_unpinned = unpinned_frame_page_indexes
                    .iter()
                    .map(|&frame_idx| (frame_idx, &self.lru_k_history[&frame_idx]));

                let has_under_capacity = lru_k_unpinned.clone().any(|(_, rb)| rb.size() < K);
                let lru_k =
                    lru_k_unpinned.filter(move |(_, rb)| !has_under_capacity || rb.size() < K);

                let (frame_page_idx_to_evict, _) = lru_k
                    .map(|(f_idx, r)| (f_idx, r.front().unwrap()))
                    .min_by_key(|(_, lru)| **lru)
                    .unwrap();

                let pool_idx = self.pool_index[&frame_page_idx_to_evict];
                self.evict_frame_index(&pool_idx);

                pool_idx
            }
            f_res => match f_res {
                Some(s) => s,
                None => {
                    panic!("unreachable")
                }
            },
        }
    }

    pub fn fetch_page<'a>(&'a mut self, page_idx: usize) -> PageGuard<'a> {
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

                PageGuard {
                    frame: &mut self.buffer_pool[idx],
                }
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

                PageGuard {
                    frame: &mut self.buffer_pool[free_slot_index],
                }
            }
        }
    }
}
