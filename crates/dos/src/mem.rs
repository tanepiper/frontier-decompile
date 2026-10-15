//! Real-mode memory: 1 MB of conventional/upper memory addressed in 16 KB pages, plus a pool of
//! expanded-memory (EMS) pages that can be mapped into the page frame at `E000:0000`.
//!
//! Linear addresses wrap at 1 MB (A20 disabled), as on a plain real-mode DOS machine.

/// Size of one mapping page. This is also the LIM EMS page size.
pub const PAGE: usize = 0x4000;
/// Conventional + upper memory, in pages.
pub const LOW_PAGES: usize = 0x10_0000 / PAGE;
/// First page of the EMS page frame (`E000:0000`).
pub const FRAME_PAGE: usize = 0xE_0000 / PAGE;
/// Segment of the EMS page frame.
pub const FRAME_SEGMENT: u16 = 0xE000;

pub struct Memory {
    ram: Vec<u8>,
    /// Physical offset into `ram` for each 16 KB page of the 1 MB address space.
    map: [usize; LOW_PAGES],
    ems_pages: usize,
    /// When set, every write appends `(linear address, previous byte)` per byte written. For
    /// parity tests and shadow verification of native ports.
    pub write_log: Option<Vec<(u32, u8)>>,
}

impl Memory {
    pub fn new(ems_pages: usize) -> Self {
        let mut map = [0; LOW_PAGES];
        for (i, m) in map.iter_mut().enumerate() {
            *m = i * PAGE;
        }
        Memory { ram: vec![0; (LOW_PAGES + ems_pages) * PAGE], map, ems_pages, write_log: None }
    }

    pub fn ems_pages(&self) -> usize {
        self.ems_pages
    }

    /// Map EMS logical page `page` (or `None` to restore the plain upper memory) into frame
    /// slot `slot` (0–3).
    pub fn map_ems(&mut self, slot: usize, page: Option<usize>) {
        let target = FRAME_PAGE + slot;
        self.map[target] = match page {
            Some(p) => (LOW_PAGES + p) * PAGE,
            None => target * PAGE,
        };
    }

    #[inline(always)]
    fn phys(&self, a: u32) -> usize {
        let a = (a & 0xF_FFFF) as usize;
        self.map[a / PAGE] + (a % PAGE)
    }

    #[inline(always)]
    pub fn rb(&self, a: u32) -> u8 {
        self.ram[self.phys(a)]
    }

    #[cold]
    fn log(&mut self, a: u32, n: u32) {
        for i in 0..n {
            let b = a.wrapping_add(i) & 0xF_FFFF;
            let old = self.ram[self.phys(b)];
            if let Some(l) = self.write_log.as_mut() {
                l.push((b, old));
            }
        }
    }

    #[inline(always)]
    pub fn wb(&mut self, a: u32, v: u8) {
        if self.write_log.is_some() {
            self.log(a, 1);
        }
        let p = self.phys(a);
        self.ram[p] = v;
    }

    #[inline(always)]
    pub fn rw(&self, a: u32) -> u16 {
        if (a as usize & (PAGE - 1)) < PAGE - 1 {
            let p = self.phys(a);
            u16::from_le_bytes([self.ram[p], self.ram[p + 1]])
        } else {
            u16::from_le_bytes([self.rb(a), self.rb(a.wrapping_add(1))])
        }
    }

    #[inline(always)]
    pub fn ww(&mut self, a: u32, v: u16) {
        if self.write_log.is_some() {
            self.log(a, 2);
        }
        if (a as usize & (PAGE - 1)) < PAGE - 1 {
            let p = self.phys(a);
            self.ram[p..p + 2].copy_from_slice(&v.to_le_bytes());
        } else {
            let b = v.to_le_bytes();
            self.wb(a, b[0]);
            self.wb(a.wrapping_add(1), b[1]);
        }
    }

    #[inline(always)]
    pub fn rd(&self, a: u32) -> u32 {
        if (a as usize & (PAGE - 1)) < PAGE - 3 {
            let p = self.phys(a);
            u32::from_le_bytes([self.ram[p], self.ram[p + 1], self.ram[p + 2], self.ram[p + 3]])
        } else {
            self.rw(a) as u32 | (self.rw(a.wrapping_add(2)) as u32) << 16
        }
    }

    #[inline(always)]
    pub fn wd(&mut self, a: u32, v: u32) {
        if (a as usize & (PAGE - 1)) < PAGE - 3 {
            if self.write_log.is_some() {
                self.log(a, 4);
            }
            let p = self.phys(a);
            self.ram[p..p + 4].copy_from_slice(&v.to_le_bytes());
        } else {
            self.ww(a, v as u16);
            self.ww(a.wrapping_add(2), (v >> 16) as u16);
        }
    }

    pub fn read_bytes(&self, a: u32, out: &mut [u8]) {
        for (i, b) in out.iter_mut().enumerate() {
            *b = self.rb(a.wrapping_add(i as u32));
        }
    }

    pub fn write_bytes(&mut self, a: u32, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.wb(a.wrapping_add(i as u32), *b);
        }
    }

    /// Read a NUL-terminated string (at most `max` bytes).
    pub fn read_asciiz(&self, a: u32, max: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..max {
            let b = self.rb(a.wrapping_add(i as u32));
            if b == 0 {
                break;
            }
            v.push(b);
        }
        v
    }

    /// Direct view of a run of conventional memory that does not cross a remapped page
    /// (used for the VGA frame at `A000:0000`).
    pub fn slice(&self, a: u32, len: usize) -> &[u8] {
        let p = self.phys(a);
        &self.ram[p..p + len]
    }
}

#[inline(always)]
pub fn lin(seg: u16, off: u32) -> u32 {
    ((seg as u32) << 4).wrapping_add(off)
}
