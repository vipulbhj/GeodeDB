use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
};

const SLOT_SIZE: u16 = 4;
const HEADER_SIZE: u16 = 6;
const PAGE_SIZE: u16 = 4096;

pub struct DiskManager {
    db: File,
    page_count: u64,
}

struct Page {
    memory: [u8; PAGE_SIZE as usize],
}

struct PageHeader {
    slot_count: u16,
    front_pointer: u16,
    back_pointer: u16,
}

struct Slot {
    offset: u16,
    length: u16,
}

impl Page {
    fn new() -> Self {
        let mut memory = [0; PAGE_SIZE as usize];
        memory[2..4].copy_from_slice(&u16::to_le_bytes(HEADER_SIZE));
        memory[4..6].copy_from_slice(&u16::to_le_bytes(PAGE_SIZE));
        Page { memory: memory }
    }

    fn from_bytes(buffer: &[u8]) -> Self {
        let mut memory = [0; PAGE_SIZE as usize];
        memory.copy_from_slice(buffer);
        Page { memory: memory }
    }

    fn header(&self) -> PageHeader {
        let slot_count = u16::from_le_bytes(self.memory[0..2].try_into().unwrap());
        let front_pointer = u16::from_le_bytes(self.memory[2..4].try_into().unwrap());
        let back_pointer = u16::from_le_bytes(self.memory[4..6].try_into().unwrap());

        PageHeader {
            slot_count: slot_count,
            front_pointer: front_pointer,
            back_pointer: back_pointer,
        }
    }

    fn write_header(&mut self, page_header: &PageHeader) -> &mut Self {
        let slot_count = u16::to_le_bytes(page_header.slot_count);
        let front_pointer = u16::to_le_bytes(page_header.front_pointer);
        let back_pointer = u16::to_le_bytes(page_header.back_pointer);

        self.memory[0..2].copy_from_slice(&slot_count);
        self.memory[2..4].copy_from_slice(&front_pointer);
        self.memory[4..6].copy_from_slice(&back_pointer);

        self
    }

    fn insert_tuple(&mut self, data: &[u8]) -> Option<u16> {
        let data_length = data.len();
        let mut header = self.header();
        let space_left = header.back_pointer - header.front_pointer - SLOT_SIZE;

        if data_length > space_left as usize {
            return None;
        }

        let front = header.front_pointer as usize;
        let back = header.back_pointer as usize;

        // Write Slot
        self.memory[front..(front + 2)]
            .copy_from_slice(&u16::to_le_bytes(back as u16 - data_length as u16));
        self.memory[(front + 2)..(front + 4)]
            .copy_from_slice(&u16::to_le_bytes(data_length as u16));

        // Write Tuple
        self.memory[(back - data_length)..back].copy_from_slice(data);

        let slot_idx = header.slot_count;

        header.slot_count += 1;
        header.front_pointer += SLOT_SIZE;
        header.back_pointer -= data_length as u16;

        self.write_header(&header);

        Some(slot_idx)
    }

    fn get_tuple(&mut self, slot_idx: u16) -> Option<&[u8]> {
        let header = self.header();

        if slot_idx >= header.slot_count {
            return None;
        }

        let slot_page_offset: usize = (HEADER_SIZE + (SLOT_SIZE * slot_idx)).try_into().unwrap();
        let slot = &self.memory[slot_page_offset..(slot_page_offset + SLOT_SIZE as usize)];

        let slot_tuple_page_offset = u16::from_le_bytes(slot[0..2].try_into().unwrap());
        let slot_tuple_length = u16::from_le_bytes(slot[2..4].try_into().unwrap());

        if slot_tuple_page_offset == 0 {
            // deleted tuple
            return None;
        }

        let tuple = &self.memory[slot_tuple_page_offset as usize
            ..((slot_tuple_page_offset + slot_tuple_length) as usize)];

        Some(tuple)
    }

    fn delete_tuple(&mut self, slot_idx: u16) -> Option<()> {
        let header = self.header();

        if slot_idx >= header.slot_count {
            return None;
        }

        let slot_page_offset: usize = (HEADER_SIZE + (SLOT_SIZE * slot_idx)).try_into().unwrap();
        let slot = &self.memory[slot_page_offset..(slot_page_offset + SLOT_SIZE as usize)];

        let slot_tuple_page_offset = u16::from_le_bytes(slot[0..2].try_into().unwrap());
        let slot_tuple_length = u16::from_le_bytes(slot[2..4].try_into().unwrap());

        if slot_tuple_page_offset == 0 {
            // deleted tuple
            return None;
        }

        self.memory[slot_tuple_page_offset as usize
            ..((slot_tuple_page_offset + slot_tuple_length) as usize)]
            .fill(0);

        self.memory[slot_page_offset..(slot_page_offset + SLOT_SIZE as usize)].fill(0);

        Some(())
    }

    fn as_bytes(&self) -> &[u8] {
        &self.memory
    }
}

impl DiskManager {
    pub fn new(db: &str) -> io::Result<Self> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .open(db)
            .unwrap();

        let metadata = file.metadata();
        let db_len = metadata.unwrap().len();

        if db_len % PAGE_SIZE as u64 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DB files has incorrect data",
            ));
        }

        let page_count = db_len / PAGE_SIZE as u64;

        Ok(DiskManager {
            db: file,
            page_count: page_count.try_into().unwrap(),
        })
    }

    pub fn read_page(&mut self, page_idx: u64) -> io::Result<Page> {
        if page_idx >= self.page_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Page index ({}) exceeds page count ({})",
                    page_idx, self.page_count
                ),
            ));
        }

        let mut buffer = [0u8; PAGE_SIZE as usize];

        let page_offset = page_idx * PAGE_SIZE as u64;

        self.db.seek(SeekFrom::Start(page_offset))?;

        self.db.read_exact(&mut buffer)?;

        Ok(Page::from_bytes(&buffer))
    }

    pub fn write_page(&mut self, page_idx: u64, buffer: &[u8]) -> io::Result<()> {
        if page_idx >= self.page_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Page index ({}) exceeds page count ({})",
                    page_idx, self.page_count
                ),
            ));
        }

        if buffer.len() > PAGE_SIZE as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Buffer size ({}) exceeds PAGE_SIZE ({})",
                    buffer.len(),
                    PAGE_SIZE
                ),
            ));
        }

        let page_offset = page_idx * PAGE_SIZE as u64;

        self.db.seek(SeekFrom::Start(page_offset as u64))?;

        self.db.write(buffer)?;

        self.db.flush()?;

        Ok(())
    }

    pub fn allocate_page(&mut self) -> io::Result<u64> {
        let page = self.page_count;

        self.write_page(page, &[0; PAGE_SIZE as usize])?;
        self.page_count += 1;

        Ok(page)
    }
}
