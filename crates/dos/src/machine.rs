//! The virtual PC: CPU, memory, devices and the DOS/BIOS/EMS/mouse services `FRONTIER.EXE` uses,
//! implemented in the host (high-level emulation) behind real interrupt vectors.
//!
//! Every vector in the IVT initially points at a stub in the BIOS segment (`F000:n*8`) that
//! executes `0F FF nn` (host service `nn`) and then `IRET`. Programs that hook a vector and chain
//! to the previous handler therefore behave exactly as on a real machine.
//!
//! The DOS layer is the subset the game needs: file I/O on the [`Vfs`], memory blocks, EXEC of
//! the `EL2*.OVL` overlays and their TSR return (`31h`), which is how the program's overlay
//! loader works (docs/OVERLAYS.md §1).

use std::collections::BTreeSet;

use crate::cpu::{Cpu, Event, CF, CS, DS, EAX, EBP, EBX, ECX, EDI, EDX, ES, ESI, ESP, SS, ZF};
use crate::devices::Devices;
use crate::mem::{lin, Memory, FRAME_SEGMENT};
use crate::vfs::{normalize, wildcard, Vfs};

const BIOS_SEG: u16 = 0xF000;
const EMS_SEG: u16 = 0xF100;
/// First paragraph handed out by the DOS allocator.
const FIRST_PARA: u16 = 0x0200;
const TOP_PARA: u16 = 0xA000;
/// Private host services (beyond the 256 vectors, reached by patched code, not the IVT).
pub const HLE_SETUP_SKIP: u8 = 0xF8;
/// Return address of host-initiated calls ([`Machine::call`]).
const HLE_CALL_RETURN: u8 = 0xF9;
const CALL_RETURN_OFF: u16 = 0x0F00;

/// A native routine standing in for original code at a hooked address. It receives the CPU at
/// the routine's first instruction and must leave it as the original's return would.
pub type HookFn = Box<dyn FnMut(&mut Cpu)>;

struct Hook {
    module: String,
    offset: u16,
    /// Linear address it is armed at while the module is loaded.
    armed: Option<u32>,
    f: HookFn,
}

/// A program image loaded by EXEC and still resident (the main program or an overlay).
#[derive(Debug, Clone)]
pub struct Image {
    /// File name, upper case (`EL2MATH3.OVL`).
    pub name: String,
    pub psp: u16,
    /// Load segment (PSP + 10h): Ghidra's segment `1000` in that file's analysis.
    pub seg: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The instruction budget for this slice ran out.
    Budget,
    /// The program exited through `int 21h AH=4Ch` with this code.
    Exited(u8),
    /// The CPU hit something it cannot run.
    Fault(Event),
    /// `HLT` with interrupts disabled: nothing can wake it.
    Hung,
}

struct Block {
    seg: u16,
    paras: u16,
    owner: u16,
}

struct OpenFile {
    key: String,
    pos: usize,
    /// A character device (`CON`, `NUL`, `EMMXXXX0`).
    device: bool,
}

struct ExecFrame {
    parent_psp: u16,
    regs: [u32; 8],
    segs: [u16; 6],
    ip: u32,
    flags: u32,
}

struct FindState {
    matches: Vec<(String, bool, u32)>,
    next: usize,
}

pub struct EmsState {
    /// Pages owned by each handle (indices into the EMS pool), `None` = free handle.
    handles: Vec<Option<Vec<usize>>>,
    used: Vec<bool>,
    /// What is mapped in each of the 4 frame slots: (handle, logical page).
    mapped: [Option<(u16, u16)>; 4],
    saved: Vec<(u16, [Option<(u16, u16)>; 4])>,
}

#[derive(Default)]
pub struct Mouse {
    pub x: i32,
    pub y: i32,
    pub buttons: u16,
    /// Buttons pressed since the program last read the state, so a press shorter than the
    /// program's polling interval is still seen (as a real driver's press counters allow).
    pub latched: u16,
    pub press_count: [u16; 3],
    pub min_x: i32,
    pub max_x: i32,
    pub min_y: i32,
    pub max_y: i32,
    pub mickey_x: i32,
    pub mickey_y: i32,
    pub visible: i32,
    pub handler: Option<(u16, u16, u16)>,
    pub pending_events: u16,
}

pub struct Machine {
    pub cpu: Cpu,
    pub dev: Devices,
    pub vfs: Vfs,
    pub ems: EmsState,
    pub mouse: Mouse,
    blocks: Vec<Block>,
    files: Vec<Option<OpenFile>>,
    exec_stack: Vec<ExecFrame>,
    images: Vec<Image>,
    hooks: Vec<Hook>,
    call_returned: bool,
    /// Instructions skipped by native hooks are not counted; this counts hook invocations.
    pub hook_calls: u64,
    find: Option<FindState>,
    pub psp: u16,
    dta: (u16, u16),
    cwd: String,
    pub video_mode: u8,
    /// Messages for the host's log (DOS `09h` text, unimplemented services, …).
    pub log: Vec<String>,
    warned: BTreeSet<String>,
    /// Host time in seconds of the last [`Machine::run_for`] call.
    exit_code: Option<u8>,
    /// `$`-string output from DOS (text mode messages).
    pub console: String,
    /// Set when the setup overlay is loaded; its check routine is replaced (DEVIATIONS.md).
    pub skip_setup: bool,
    pub sound_selection: u8,
}

impl Machine {
    pub fn new(vfs: Vfs) -> Self {
        let mem = Memory::new(256); // 4 MB of EMS
        let cpu = Cpu::new(mem);
        let ems_pages = cpu.mem.ems_pages();
        let mut m = Machine {
            cpu,
            dev: Devices::new(25_000_000.0),
            vfs,
            ems: EmsState { handles: vec![Some(Vec::new())], used: vec![false; ems_pages], mapped: [None; 4], saved: Vec::new() },
            mouse: Mouse { max_x: 639, max_y: 199, ..Default::default() },
            blocks: Vec::new(),
            files: Vec::new(),
            exec_stack: Vec::new(),
            images: Vec::new(),
            hooks: Vec::new(),
            call_returned: false,
            hook_calls: 0,
            find: None,
            psp: 0,
            dta: (0, 0x80),
            cwd: String::new(),
            video_mode: 3,
            log: Vec::new(),
            warned: BTreeSet::new(),
            exit_code: None,
            console: String::new(),
            skip_setup: true,
            sound_selection: 0,
        };
        m.install_bios();
        for name in ["CON", "CON", "CON", "AUX", "PRN"] {
            m.files.push(Some(OpenFile { key: name.into(), pos: 0, device: true }));
        }
        m
    }

    fn warn(&mut self, what: String) {
        if self.warned.insert(what.clone()) {
            self.log.push(what);
        }
    }

    fn install_bios(&mut self) {
        let mem = &mut self.cpu.mem;
        for n in 0..256u32 {
            let off = n * 8;
            let a = lin(BIOS_SEG, off);
            if n == 8 {
                // Timer: host tick bookkeeping, then INT 1Ch, then IRET.
                mem.write_bytes(a, &[0x0F, 0xFF, 0x08, 0xCD, 0x1C, 0xCF]);
            } else if n >= HLE_SETUP_SKIP as u32 {
                mem.write_bytes(a, &[0xCF]);
            } else {
                mem.write_bytes(a, &[0x0F, 0xFF, n as u8, 0xCF]);
            }
            mem.ww(n * 4, off as u16);
            mem.ww(n * 4 + 2, BIOS_SEG);
        }
        // EMS driver: the device name sits at offset 0Ah of the int 67h handler's segment,
        // which is how programs detect an expanded memory manager.
        mem.write_bytes(lin(EMS_SEG, 0x0A), b"EMMXXXX0");
        mem.write_bytes(lin(EMS_SEG, 0x12), &[0x0F, 0xFF, 0x67, 0xCF]);
        mem.ww(0x67 * 4, 0x12);
        mem.ww(0x67 * 4 + 2, EMS_SEG);
        // BIOS data area.
        mem.ww(0x410, 0x0021); // equipment: FPU absent, colour 80×25, 1 floppy
        mem.ww(0x413, 640);
        mem.ww(0x41A, 0x1E);
        mem.ww(0x41C, 0x1E);
        mem.ww(0x480, 0x1E);
        mem.ww(0x482, 0x3E);
        mem.wb(0x449, 3);
        mem.ww(0x44A, 80);
        mem.ww(0x463, 0x3D4);
        mem.wb(0x484, 24);
        // A plausible model byte.
        mem.wb(lin(0xF000, 0xFFFE), 0xFC);
        mem.write_bytes(lin(BIOS_SEG, CALL_RETURN_OFF as u32), &[0x0F, 0xFF, HLE_CALL_RETURN]);
    }

    // ---- DOS memory -----------------------------------------------------------------------

    fn largest_free(&self) -> (u16, u16) {
        let mut best = (0u16, 0u16);
        let mut at = FIRST_PARA;
        for b in &self.blocks {
            if b.seg > at && b.seg - at > best.1 {
                best = (at, b.seg - at);
            }
            at = at.max(b.seg + b.paras);
        }
        if TOP_PARA > at && TOP_PARA - at > best.1 {
            best = (at, TOP_PARA - at);
        }
        best
    }

    /// First-fit allocation of `paras` paragraphs.
    fn alloc(&mut self, paras: u16, owner: u16) -> Result<u16, u16> {
        let mut at = FIRST_PARA;
        let mut idx = 0;
        for (i, b) in self.blocks.iter().enumerate() {
            if b.seg >= at && b.seg - at >= paras {
                idx = i;
                break;
            }
            at = at.max(b.seg + b.paras);
            idx = i + 1;
        }
        if idx == self.blocks.len() && (TOP_PARA < at || TOP_PARA - at < paras) {
            return Err(self.largest_free().1);
        }
        self.blocks.insert(idx, Block { seg: at, paras, owner });
        Ok(at)
    }

    fn free(&mut self, seg: u16) -> bool {
        match self.blocks.iter().position(|b| b.seg == seg) {
            Some(i) => {
                self.blocks.remove(i);
                if self.images.iter().any(|im| im.psp == seg) {
                    self.images.retain(|im| im.psp != seg);
                    self.arm_hooks();
                }
                true
            }
            None => false,
        }
    }

    // ---- images, hooks and host calls ------------------------------------------------------

    /// Resident program images (main program first).
    pub fn images(&self) -> &[Image] {
        &self.images
    }

    /// Load segment of a resident image by file name (`FRONTIER.EXE`, `EL2MATH3.OVL`).
    pub fn image_segment(&self, name: &str) -> Option<u16> {
        self.images.iter().find(|im| im.name == name).map(|im| im.seg)
    }

    /// Replace the original routine at `module:offset` (Ghidra `1000:offset` in that file) with
    /// `f` whenever the module is resident.
    pub fn hook(&mut self, module: &str, offset: u16, f: HookFn) {
        self.hooks.push(Hook { module: module.to_string(), offset, armed: None, f });
        self.arm_hooks();
    }

    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// Set CPU breakpoints for every hook whose module is resident, clear stale ones.
    fn arm_hooks(&mut self) {
        for h in self.hooks.iter_mut() {
            let want = self.images.iter().find(|im| im.name == h.module).map(|im| lin(im.seg, h.offset as u32) & 0xF_FFFF);
            if h.armed != want {
                if let Some(old) = h.armed {
                    self.cpu.set_breakpoint(old, false);
                }
                if let Some(new) = want {
                    self.cpu.set_breakpoint(new, true);
                }
                h.armed = want;
            }
        }
    }

    fn run_hook(&mut self, l: u32) -> bool {
        match self.hooks.iter_mut().find(|h| h.armed == Some(l)) {
            Some(h) => {
                (h.f)(&mut self.cpu);
                self.hook_calls += 1;
                true
            }
            None => false,
        }
    }

    /// Run the original routine at `seg:off` as a far call with the current registers until it
    /// returns, with interrupts held off. Hooks are bypassed for the called routine's entry, so
    /// this always runs original code there. For parity tests and tracing.
    pub fn call(&mut self, seg: u16, off: u16, budget: u64) -> Result<(), Stop> {
        let saved_if = self.cpu.flags & crate::cpu::IF;
        self.cpu.flags &= !crate::cpu::IF;
        self.cpu.far_call(seg, off, BIOS_SEG, CALL_RETURN_OFF);
        self.cpu.skip_breakpoint = true;
        self.call_returned = false;
        let mut left = budget;
        let r = loop {
            if left == 0 {
                break Err(Stop::Budget);
            }
            left -= 1;
            match self.cpu.step(&mut self.dev) {
                Ok(()) => {}
                Err(Event::Hle(n)) => {
                    self.hle(n);
                    if self.call_returned {
                        break Ok(());
                    }
                }
                Err(Event::Breakpoint(l)) => {
                    self.run_hook(l);
                }
                Err(e) => break Err(Stop::Fault(e)),
            }
        };
        self.cpu.flags |= saved_if;
        r
    }

    fn resize(&mut self, seg: u16, paras: u16) -> Result<(), u16> {
        let i = self.blocks.iter().position(|b| b.seg == seg).ok_or(0u16)?;
        let limit = self.blocks.get(i + 1).map(|b| b.seg).unwrap_or(TOP_PARA);
        if seg as u32 + paras as u32 > limit as u32 {
            return Err(limit - seg);
        }
        self.blocks[i].paras = paras;
        Ok(())
    }

    // ---- program loading ------------------------------------------------------------------

    /// Build a PSP at `psp` for a program whose memory block ends at `end`.
    fn make_psp(&mut self, psp: u16, end: u16, env: u16, tail: &[u8], parent: u16) {
        let mem = &mut self.cpu.mem;
        let base = lin(psp, 0);
        for i in 0..256 {
            mem.wb(base + i, 0);
        }
        mem.write_bytes(base, &[0xCD, 0x20]);
        mem.ww(base + 2, end);
        mem.ww(base + 0x16, parent);
        mem.ww(base + 0x2C, env);
        mem.write_bytes(base + 0x50, &[0xCD, 0x21, 0xCB]);
        for i in 0..20 {
            mem.wb(base + 0x18 + i, if i < 5 { i as u8 } else { 0xFF });
        }
        mem.ww(base + 0x32, 20);
        mem.ww(base + 0x34, 0x18);
        mem.ww(base + 0x36, psp);
        let n = tail.len().min(126);
        mem.wb(base + 0x80, n as u8);
        mem.write_bytes(base + 0x81, &tail[..n]);
        mem.wb(base + 0x81 + n as u32, 0x0D);
    }

    fn make_env(&mut self, program: &str, owner: u16) -> u16 {
        let mut env = b"PATH=C:\\\0COMSPEC=C:\\COMMAND.COM\0\0".to_vec();
        env.extend_from_slice(&1u16.to_le_bytes());
        env.extend_from_slice(format!("C:\\{program}").as_bytes());
        env.push(0);
        let paras = (env.len() as u16 + 15) / 16;
        let seg = self.alloc(paras, owner).unwrap_or(FIRST_PARA);
        self.cpu.mem.write_bytes(lin(seg, 0), &env);
        seg
    }

    /// Load an MZ image into a fresh block. Returns (psp, cs, ip, ss, sp).
    fn load_mz(&mut self, key: &str, tail: &[u8], parent: u16) -> Result<(u16, u16, u16, u16, u16), u16> {
        let data = self.vfs.get(key).ok_or(2u16)?.clone();
        let hdr = frontier_formats::mz::parse(&data).map_err(|_| 11u16)?;
        let image = hdr.image_size();
        let start = hdr.header_size();
        let need = ((image + 15) / 16) as u32 + 0x10 + hdr.min_alloc as u32;
        let program = key.rsplit('\\').next().unwrap_or(key).to_string();
        let env = self.make_env(&program, 0);
        let (_, avail) = self.largest_free();
        if (avail as u32) < need {
            self.free(env);
            return Err(8);
        }
        let want = ((image + 15) / 16) as u32 + 0x10 + hdr.max_alloc as u32;
        let paras = want.min(avail as u32) as u16;
        let psp = self.alloc(paras, 0).map_err(|_| 8u16)?;
        for b in self.blocks.iter_mut() {
            if b.seg == psp || b.seg == env {
                b.owner = psp;
            }
        }
        let load = psp + 0x10;
        let end = data.len().min(start + image);
        self.cpu.mem.write_bytes(lin(load, 0), &data[start..end]);
        for &(seg, off) in &hdr.relocations {
            let a = lin(load.wrapping_add(seg), off as u32);
            let v = self.cpu.mem.rw(a).wrapping_add(load);
            self.cpu.mem.ww(a, v);
        }
        self.make_psp(psp, psp + paras, env, tail, parent);
        Ok((psp, load.wrapping_add(hdr.initial_cs), hdr.initial_ip, load.wrapping_add(hdr.initial_ss), hdr.initial_sp))
    }

    /// Load and start the main program.
    pub fn boot(&mut self, program: &str) -> Result<(), String> {
        let key = normalize(program.as_bytes(), "");
        let (psp, cs, ip, ss, sp) = self.load_mz(&key, b"", 0).map_err(|e| format!("cannot load {program}: DOS error {e}"))?;
        self.psp = psp;
        self.dta = (psp, 0x80);
        let name = key.rsplit('\\').next().unwrap_or(&key).to_string();
        self.images.push(Image { name, psp, seg: psp + 0x10 });
        self.arm_hooks();
        let c = &mut self.cpu;
        c.s = [psp, cs, ss, psp, 0, 0];
        c.ip = ip as u32;
        c.r = [0; 8];
        c.r[ESP] = sp as u32;
        c.flags = 0x0202;
        Ok(())
    }

    // ---- the run loop ---------------------------------------------------------------------

    pub fn now(&self) -> f64 {
        self.cpu.icount as f64 / self.dev.ips
    }

    /// Run up to `budget` instructions.
    pub fn run(&mut self, budget: u64) -> Stop {
        if let Some(c) = self.exit_code {
            return Stop::Exited(c);
        }
        let end = self.cpu.icount + budget;
        while self.cpu.icount < end {
            self.dev.now = self.now();
            if self.dev.now >= self.dev.next_tick {
                self.dev.raise_irq(0);
                self.dev.next_tick += self.dev.pit.period();
                if self.dev.next_tick < self.dev.now {
                    self.dev.next_tick = self.dev.now + self.dev.pit.period();
                }
            }
            self.dev.pump_keyboard();
            if self.cpu.flag(crate::cpu::IF) && !self.cpu.inhibit_irq {
                if let Some(irq) = self.dev.pic.next() {
                    self.dev.pic.ack(irq);
                    self.cpu.interrupt(8 + irq);
                }
            }
            if self.cpu.halted {
                if !self.cpu.flag(crate::cpu::IF) {
                    return Stop::Hung;
                }
                // Skip ahead to the next timer tick.
                let target = (self.dev.next_tick * self.dev.ips) as u64 + 1;
                self.cpu.icount = target.max(self.cpu.icount + 1).min(end);
                continue;
            }
            // Run a short burst between device checks.
            for _ in 0..64 {
                match self.cpu.step(&mut self.dev) {
                    Ok(()) => {}
                    Err(Event::Hle(n)) => {
                        self.hle(n);
                        if let Some(c) = self.exit_code {
                            return Stop::Exited(c);
                        }
                        break;
                    }
                    Err(Event::Halt) => break,
                    Err(Event::Breakpoint(l)) => {
                        if !self.run_hook(l) {
                            self.cpu.skip_breakpoint = true;
                        }
                    }
                    Err(e) => return Stop::Fault(e),
                }
                if self.cpu.inhibit_irq {
                    continue;
                }
            }
        }
        Stop::Budget
    }

    /// Queue a PC/XT set-1 scancode (bit 7 set = release).
    pub fn key(&mut self, scancode: u8) {
        self.dev.kbd_queue.push_back(scancode);
    }

    /// The 320×200 frame at `A000:0000` and the DAC palette as 8-bit RGB.
    pub fn frame_rgba(&self, out: &mut [u8]) {
        let vram = self.cpu.mem.slice(0xA0000, 64000);
        let pal: Vec<[u8; 3]> = self.dev.vga.dac.iter().map(|c| [c[0] << 2 | c[0] >> 4, c[1] << 2 | c[1] >> 4, c[2] << 2 | c[2] >> 4]).collect();
        for (i, &p) in vram.iter().enumerate() {
            let c = pal[p as usize];
            out[i * 4..i * 4 + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }

    // ---- host services --------------------------------------------------------------------

    fn hle(&mut self, n: u8) {
        match n {
            0x08 => {
                let t = self.cpu.mem.rd(0x46C).wrapping_add(1);
                self.cpu.mem.wd(0x46C, if t >= 0x1800B0 { 0 } else { t });
                self.dev.outb_eoi();
            }
            0x09 => {
                let code = { use crate::cpu::Io; self.dev.inb(0x60) };
                self.bios_key_event(code);
                self.dev.outb_eoi();
            }
            0x0A..=0x0F | 0x70..=0x77 => self.dev.outb_eoi(),
            0x10 => self.int10(),
            0x11 => {
                let v = self.cpu.mem.rw(0x410);
                self.cpu.set_ax(v);
            }
            0x12 => self.cpu.set_ax(640),
            0x15 => {
                let ah = self.cpu.ah();
                if ah == 0x88 {
                    self.cpu.set_ax(0);
                    self.ret_cf(false);
                } else {
                    self.cpu.set_ah(0x86);
                    self.ret_cf(true);
                }
            }
            0x16 => self.int16(),
            0x1A => self.int1a(),
            0x1C | 0x1B | 0x23 => {}
            0x21 => self.int21(),
            0x24 => self.cpu.set_al(3),
            0x2F => {
                // Multiplex: nothing installed (no XMS, no Windows).
            }
            0x33 => self.int33(),
            0x67 => self.int67(),
            HLE_SETUP_SKIP => self.setup_skip(),
            HLE_CALL_RETURN => self.call_returned = true,
            _ => self.warn(format!("unhandled interrupt {n:02x} AX={:04x}", self.cpu.ax())),
        }
    }

    /// Set the carry flag that the interrupt's IRET will restore.
    fn ret_cf(&mut self, cf: bool) {
        let a = lin(self.cpu.s[SS], (self.cpu.r[ESP] + 4) & 0xFFFF);
        let f = self.cpu.mem.rw(a);
        self.cpu.mem.ww(a, if cf { f | CF as u16 } else { f & !(CF as u16) });
    }

    fn ret_zf(&mut self, zf: bool) {
        let a = lin(self.cpu.s[SS], (self.cpu.r[ESP] + 4) & 0xFFFF);
        let f = self.cpu.mem.rw(a);
        self.cpu.mem.ww(a, if zf { f | ZF as u16 } else { f & !(ZF as u16) });
    }

    fn dos_err(&mut self, code: u16) {
        self.cpu.set_ax(code);
        self.ret_cf(true);
    }

    /// Deviation (DEVIATIONS.md): `EL2SETUP:0496`, the DOS/CPU/memory/EMS checks and the sound
    /// prompt, is replaced. It records the sound choice in `DS:8920` and returns carry clear.
    fn setup_skip(&mut self) {
        let ds = self.cpu.s[DS];
        self.cpu.mem.wb(lin(ds, 0x8920), self.sound_selection);
        self.cpu.flags &= !CF;
    }

    fn bios_key_event(&mut self, code: u8) {
        const ASCII: &[u8; 58] = b"\x00\x1b1234567890-=\x08\tqwertyuiop[]\r\x00asdfghjkl;'`\x00\\zxcvbnm,./\x00*\x00 ";
        if code & 0x80 != 0 {
            return;
        }
        let ascii = ASCII.get(code as usize).copied().unwrap_or(0);
        let head = self.cpu.mem.rw(0x41A);
        let tail = self.cpu.mem.rw(0x41C);
        let next = if tail + 2 >= 0x3E { 0x1E } else { tail + 2 };
        if next != head {
            self.cpu.mem.ww(0x400 + tail as u32, (code as u16) << 8 | ascii as u16);
            self.cpu.mem.ww(0x41C, next);
        }
    }

    fn int16(&mut self) {
        let head = self.cpu.mem.rw(0x41A);
        let tail = self.cpu.mem.rw(0x41C);
        match self.cpu.ah() & 0xEF {
            0x00 => {
                if head == tail {
                    // Block: re-run the INT after interrupts have had a chance to fill the buffer.
                    let ip = self.cpu.mem.rw(lin(self.cpu.s[SS], self.cpu.r[ESP] & 0xFFFF));
                    self.cpu.mem.ww(lin(self.cpu.s[SS], self.cpu.r[ESP] & 0xFFFF), ip.wrapping_sub(2));
                    let a = lin(self.cpu.s[SS], (self.cpu.r[ESP] + 4) & 0xFFFF);
                    let f = self.cpu.mem.rw(a);
                    self.cpu.mem.ww(a, f | 0x200);
                    return;
                }
                let v = self.cpu.mem.rw(0x400 + head as u32);
                self.cpu.set_ax(v);
                let next = if head + 2 >= 0x3E { 0x1E } else { head + 2 };
                self.cpu.mem.ww(0x41A, next);
            }
            0x01 => {
                if head == tail {
                    self.ret_zf(true);
                } else {
                    let v = self.cpu.mem.rw(0x400 + head as u32);
                    self.cpu.set_ax(v);
                    self.ret_zf(false);
                }
            }
            0x02 => {
                let v = self.cpu.mem.rb(0x417);
                self.cpu.set_al(v);
            }
            _ => self.warn(format!("int 16h AH={:02x}", self.cpu.ah())),
        }
    }

    fn int1a(&mut self) {
        match self.cpu.ah() {
            0x00 => {
                let t = self.cpu.mem.rd(0x46C);
                self.cpu.set_cx((t >> 16) as u16);
                self.cpu.set_dx(t as u16);
                self.cpu.set_al(0);
            }
            0x02 => {
                self.cpu.set_cx(0x1200);
                self.cpu.set_dx(0);
                self.ret_cf(false);
            }
            0x04 => {
                self.cpu.set_cx(0x3200);
                self.cpu.set_dx(0x0101);
                self.ret_cf(false);
            }
            _ => self.warn(format!("int 1Ah AH={:02x}", self.cpu.ah())),
        }
    }

    fn int10(&mut self) {
        match self.cpu.ah() {
            0x00 => {
                let mode = self.cpu.al() & 0x7F;
                self.video_mode = mode;
                self.cpu.mem.wb(0x449, mode);
                if self.cpu.al() & 0x80 == 0 {
                    if mode == 0x13 {
                        for i in 0..0x10000u32 {
                            self.cpu.mem.wb(0xA0000 + i, 0);
                        }
                    } else {
                        for i in 0..4000u32 {
                            self.cpu.mem.wb(0xB8000 + i, if i & 1 == 0 { 0x20 } else { 7 });
                        }
                    }
                }
                if mode == 0x13 {
                    self.dev.vga.seq[4] = 0x0E;
                    self.default_palette();
                }
                self.log.push(format!("video mode {mode:02x}h"));
            }
            0x0F => {
                let m = self.video_mode;
                self.cpu.set_al(m);
                self.cpu.set_ah(if m == 0x13 { 40 } else { 80 });
                self.cpu.set_reg(EBX, 1, 0);
                self.cpu.set_reg(4 + EBX, 1, 0);
            }
            0x10 => {
                let al = self.cpu.al();
                match al {
                    0x10 => {
                        let (bx, dh, ch, cl) = (self.cpu.bx(), (self.cpu.dx() >> 8) as u8, (self.cpu.cx() >> 8) as u8, self.cpu.cx() as u8);
                        self.dev.set_dac(bx as u8, [dh, ch, cl]);
                    }
                    0x12 => {
                        let (start, count) = (self.cpu.bx(), self.cpu.cx());
                        let a = lin(self.cpu.s[ES], self.cpu.dx() as u32);
                        for i in 0..count as u32 {
                            let rgb = [self.cpu.mem.rb(a + i * 3), self.cpu.mem.rb(a + i * 3 + 1), self.cpu.mem.rb(a + i * 3 + 2)];
                            self.dev.set_dac((start as u32 + i) as u8, rgb);
                        }
                    }
                    0x15 => {
                        let c = self.dev.vga.dac[self.cpu.bx() as usize & 255];
                        self.cpu.set_dx((c[0] as u16) << 8);
                        self.cpu.set_cx((c[1] as u16) << 8 | c[2] as u16);
                    }
                    0x17 => {
                        let (start, count) = (self.cpu.bx(), self.cpu.cx());
                        let a = lin(self.cpu.s[ES], self.cpu.dx() as u32);
                        for i in 0..count as u32 {
                            let c = self.dev.vga.dac[(start as u32 + i) as usize & 255];
                            self.cpu.mem.write_bytes(a + i * 3, &c);
                        }
                    }
                    _ => self.warn(format!("int 10h AX={:04x}", self.cpu.ax())),
                }
            }
            0x02 | 0x01 | 0x05 | 0x06 | 0x07 | 0x08 | 0x09 | 0x0B => {}
            0x03 => {
                self.cpu.set_dx(0);
                self.cpu.set_cx(0x0607);
            }
            0x0E => {
                let c = self.cpu.al() as char;
                self.console.push(c);
            }
            0x1A => {
                self.cpu.set_al(0x1A);
                self.cpu.set_bx(0x0008);
            }
            0x12 => {
                // EGA/VGA info: colour, 256 KB.
                if self.cpu.bx() & 0xFF == 0x10 {
                    self.cpu.set_bx(0x0003);
                    self.cpu.set_cx(0x0009);
                }
            }
            _ => self.warn(format!("int 10h AH={:02x}", self.cpu.ah())),
        }
    }

    fn default_palette(&mut self) {
        // The first 16 entries as the BIOS sets them; the rest grey. Programs set their own.
        const EGA: [[u8; 3]; 16] = [
            [0, 0, 0], [0, 0, 42], [0, 42, 0], [0, 42, 42], [42, 0, 0], [42, 0, 42], [42, 21, 0], [42, 42, 42],
            [21, 21, 21], [21, 21, 63], [21, 63, 21], [21, 63, 63], [63, 21, 21], [63, 21, 63], [63, 63, 21], [63, 63, 63],
        ];
        for (i, c) in EGA.iter().enumerate() {
            self.dev.set_dac(i as u8, *c);
        }
    }

    // ---- int 21h ---------------------------------------------------------------------------

    fn ds_dx(&self) -> u32 {
        lin(self.cpu.s[DS], self.cpu.dx() as u32)
    }

    fn path_at(&self, a: u32) -> String {
        normalize(&self.cpu.mem.read_asciiz(a, 128), &self.cwd)
    }

    fn new_handle(&mut self, f: OpenFile) -> u16 {
        for (i, slot) in self.files.iter_mut().enumerate().skip(5) {
            if slot.is_none() {
                *slot = Some(f);
                return i as u16;
            }
        }
        self.files.push(Some(f));
        (self.files.len() - 1) as u16
    }

    fn is_device(key: &str) -> bool {
        matches!(key.rsplit('\\').next().unwrap_or(key), "CON" | "NUL" | "EMMXXXX0" | "PRN" | "AUX")
    }

    fn int21(&mut self) {
        let ah = self.cpu.ah();
        match ah {
            0x02 => {
                let c = self.cpu.dx() as u8 as char;
                self.console.push(c);
            }
            0x06 => {
                if self.cpu.dx() as u8 == 0xFF {
                    self.cpu.set_al(0);
                    self.ret_zf(true);
                } else {
                    let c = self.cpu.dx() as u8 as char;
                    self.console.push(c);
                }
            }
            0x09 => {
                let a = self.ds_dx();
                let mut s = String::new();
                for i in 0..4096 {
                    let b = self.cpu.mem.rb(a + i);
                    if b == b'$' {
                        break;
                    }
                    s.push(b as char);
                }
                self.console.push_str(&s);
                self.log.push(format!("DOS print: {}", s.trim()));
            }
            0x0B => self.cpu.set_al(0),
            0x0E => self.cpu.set_al(3),
            0x19 => self.cpu.set_al(2),
            0x1B | 0x1C => {
                // Allocation info: 4 sectors/cluster, 512 bytes/sector, 0xFFF0 clusters, media
                // descriptor F8h (fixed disk) at DS:BX.
                self.cpu.mem.wb(lin(BIOS_SEG, 0xFFF0), 0xF8);
                self.cpu.set_al(4);
                self.cpu.set_cx(512);
                self.cpu.set_dx(0xFFF0);
                self.cpu.s[DS] = BIOS_SEG;
                self.cpu.set_bx(0xFFF0);
            }
            0x1A => self.dta = (self.cpu.s[DS], self.cpu.dx()),
            0x25 => {
                let n = self.cpu.al() as u32;
                let (seg, off) = (self.cpu.s[DS], self.cpu.dx());
                self.cpu.mem.ww(n * 4, off);
                self.cpu.mem.ww(n * 4 + 2, seg);
            }
            0x2A => {
                self.cpu.set_cx(1993);
                self.cpu.set_dx(0x0101);
                self.cpu.set_al(5);
            }
            0x2C => {
                let t = self.now();
                let cs = (t * 100.0) as u64;
                self.cpu.set_cx((((cs / 360000) % 24) << 8 | (cs / 6000) % 60) as u16);
                self.cpu.set_dx((((cs / 100) % 60) << 8 | cs % 100) as u16);
            }
            0x2F => {
                let (s, o) = self.dta;
                self.cpu.s[ES] = s;
                self.cpu.set_bx(o);
            }
            0x30 => {
                self.cpu.set_ax(0x0005); // DOS 5.0
                self.cpu.set_bx(0);
                self.cpu.set_cx(0);
            }
            0x31 => self.terminate(true),
            0x33 => {
                if self.cpu.al() == 0 {
                    self.cpu.set_reg(EDX, 1, 0);
                } else if self.cpu.al() == 6 {
                    self.cpu.set_bx(0x0005);
                }
            }
            0x35 => {
                let n = self.cpu.al() as u32;
                let off = self.cpu.mem.rw(n * 4);
                let seg = self.cpu.mem.rw(n * 4 + 2);
                self.cpu.set_bx(off);
                self.cpu.s[ES] = seg;
            }
            0x36 => {
                self.cpu.set_ax(64);
                self.cpu.set_bx(0x4000);
                self.cpu.set_cx(512);
                self.cpu.set_dx(0xFFFF);
            }
            0x3B => {
                let p = self.path_at(self.ds_dx());
                if self.vfs.is_dir(&p) {
                    self.cwd = p;
                    self.ret_cf(false);
                } else {
                    self.dos_err(3);
                }
            }
            0x3C | 0x5B => {
                let key = self.path_at(self.ds_dx());
                if ah == 0x5B && self.vfs.get(&key).is_some() {
                    return self.dos_err(80);
                }
                self.vfs.create(&key);
                self.log.push(format!("create {key}"));
                let h = self.new_handle(OpenFile { key, pos: 0, device: false });
                self.cpu.set_ax(h);
                self.ret_cf(false);
            }
            0x3D => {
                let key = self.path_at(self.ds_dx());
                if Self::is_device(&key) {
                    let h = self.new_handle(OpenFile { key, pos: 0, device: true });
                    self.cpu.set_ax(h);
                    self.ret_cf(false);
                } else if self.vfs.get(&key).is_some() {
                    self.log.push(format!("open {key}"));
                    let h = self.new_handle(OpenFile { key, pos: 0, device: false });
                    self.cpu.set_ax(h);
                    self.ret_cf(false);
                } else {
                    self.log.push(format!("open {key}: not found"));
                    self.dos_err(2);
                }
            }
            0x3E => {
                let h = self.cpu.bx() as usize;
                if h < self.files.len() && self.files[h].is_some() {
                    if h >= 5 {
                        self.files[h] = None;
                    }
                    self.ret_cf(false);
                } else {
                    self.dos_err(6);
                }
            }
            0x3F => {
                let h = self.cpu.bx() as usize;
                let n = self.cpu.cx() as usize;
                let a = self.ds_dx();
                let Some(Some(f)) = self.files.get_mut(h) else { return self.dos_err(6) };
                if f.device {
                    self.cpu.set_ax(0);
                    return self.ret_cf(false);
                }
                let data = self.vfs.get(&f.key).map(|d| d.as_slice()).unwrap_or(&[]);
                let start = f.pos.min(data.len());
                let end = (start + n).min(data.len());
                let chunk = data[start..end].to_vec();
                f.pos = end;
                self.cpu.mem.write_bytes(a, &chunk);
                self.cpu.set_ax(chunk.len() as u16);
                self.ret_cf(false);
            }
            0x40 => {
                let h = self.cpu.bx() as usize;
                let n = self.cpu.cx() as usize;
                let mut buf = vec![0u8; n];
                self.cpu.mem.read_bytes(self.ds_dx(), &mut buf);
                let Some(Some(f)) = self.files.get_mut(h) else { return self.dos_err(6) };
                if f.device {
                    let s = String::from_utf8_lossy(&buf).to_string();
                    self.console.push_str(&s);
                } else {
                    let pos = f.pos;
                    let key = f.key.clone();
                    f.pos = pos + n;
                    if let Some(d) = self.vfs.get_mut(&key) {
                        if n == 0 {
                            d.truncate(pos);
                        } else {
                            if d.len() < pos + n {
                                d.resize(pos + n, 0);
                            }
                            d[pos..pos + n].copy_from_slice(&buf);
                        }
                    }
                }
                self.cpu.set_ax(n as u16);
                self.ret_cf(false);
            }
            0x41 => {
                let key = self.path_at(self.ds_dx());
                if self.vfs.remove(&key) {
                    self.ret_cf(false);
                } else {
                    self.dos_err(2);
                }
            }
            0x42 => {
                let h = self.cpu.bx() as usize;
                let off = ((self.cpu.cx() as u32) << 16 | self.cpu.dx() as u32) as i32 as i64;
                let whence = self.cpu.al();
                let Some(Some(f)) = self.files.get_mut(h) else { return self.dos_err(6) };
                let len = self.vfs.get(&f.key).map(|d| d.len()).unwrap_or(0) as i64;
                let base = match whence {
                    0 => 0,
                    1 => f.pos as i64,
                    _ => len,
                };
                let np = (base + off).max(0) as usize;
                f.pos = np;
                self.cpu.set_ax(np as u16);
                self.cpu.set_dx((np >> 16) as u16);
                self.ret_cf(false);
            }
            0x43 => {
                let key = self.path_at(self.ds_dx());
                if self.vfs.get(&key).is_some() || self.vfs.is_dir(&key) {
                    self.cpu.set_cx(if self.vfs.is_dir(&key) { 0x10 } else { 0x20 });
                    self.ret_cf(false);
                } else {
                    self.dos_err(2);
                }
            }
            0x44 => {
                let h = self.cpu.bx() as usize;
                let dev = matches!(self.files.get(h), Some(Some(f)) if f.device);
                match self.cpu.al() {
                    0x00 => {
                        self.cpu.set_dx(if dev { 0x80 } else { 0x02 });
                        self.ret_cf(false);
                    }
                    0x06 | 0x07 => {
                        self.cpu.set_al(0xFF);
                        self.ret_cf(false);
                    }
                    0x08 => {
                        self.cpu.set_ax(1);
                        self.ret_cf(false);
                    }
                    _ => {
                        self.warn(format!("int 21h AX={:04x}", self.cpu.ax()));
                        self.dos_err(1);
                    }
                }
            }
            0x47 => {
                let a = lin(self.cpu.s[DS], self.cpu.si() as u32);
                let mut c = self.cwd.as_bytes().to_vec();
                c.push(0);
                self.cpu.mem.write_bytes(a, &c);
                self.ret_cf(false);
            }
            0x48 => {
                let want = self.cpu.bx();
                match self.alloc(want, self.psp) {
                    Ok(seg) => {
                        self.cpu.set_ax(seg);
                        self.ret_cf(false);
                    }
                    Err(max) => {
                        self.cpu.set_bx(max);
                        self.dos_err(8);
                    }
                }
            }
            0x49 => {
                let seg = self.cpu.s[ES];
                if self.free(seg) {
                    self.ret_cf(false);
                } else {
                    self.dos_err(9);
                }
            }
            0x4A => {
                let (seg, paras) = (self.cpu.s[ES], self.cpu.bx());
                match self.resize(seg, paras) {
                    Ok(()) => self.ret_cf(false),
                    Err(max) => {
                        self.cpu.set_bx(max);
                        self.dos_err(8);
                    }
                }
            }
            0x4B => self.exec(),
            0x4C => self.terminate(false),
            0x4D => self.cpu.set_ax(0),
            0x4E | 0x4F => self.find_file(ah == 0x4E),
            0x50 => self.psp = self.cpu.bx(),
            0x51 | 0x62 => {
                let p = self.psp;
                self.cpu.set_bx(p);
            }
            0x56 => {
                let from = self.path_at(self.ds_dx());
                let to = self.path_at(lin(self.cpu.s[ES], self.cpu.di() as u32));
                if self.vfs.rename(&from, &to) {
                    self.ret_cf(false);
                } else {
                    self.dos_err(2);
                }
            }
            0x57 => {
                self.cpu.set_cx(0);
                self.cpu.set_dx(0x1A21);
                self.ret_cf(false);
            }
            0x58 => {
                self.cpu.set_ax(0);
                self.ret_cf(false);
            }
            0x59 => self.cpu.set_ax(0),
            _ => {
                self.warn(format!("int 21h AH={ah:02x}"));
                self.dos_err(1);
            }
        }
    }

    fn find_file(&mut self, first: bool) {
        if first {
            let spec = self.path_at(self.ds_dx());
            let (dir, pat) = spec.rsplit_once('\\').map(|(d, p)| (d.to_string(), p.to_string())).unwrap_or((String::new(), spec.clone()));
            let attrs = self.cpu.cx();
            let matches = self
                .vfs
                .list(&dir)
                .into_iter()
                .filter(|(n, d, _)| wildcard(&pat, n) && (!d || attrs & 0x10 != 0))
                .collect();
            self.find = Some(FindState { matches, next: 0 });
        }
        let Some(st) = self.find.as_mut() else { return self.dos_err(18) };
        let Some((name, dir, size)) = st.matches.get(st.next).cloned() else { return self.dos_err(18) };
        st.next += 1;
        let a = lin(self.dta.0, self.dta.1 as u32);
        let mem = &mut self.cpu.mem;
        mem.wb(a + 0x15, if dir { 0x10 } else { 0x20 });
        mem.ww(a + 0x16, 0);
        mem.ww(a + 0x18, 0x1A21);
        mem.wd(a + 0x1A, size);
        let mut nm = name.into_bytes();
        nm.truncate(12);
        nm.push(0);
        mem.write_bytes(a + 0x1E, &nm);
        self.ret_cf(false);
    }

    /// `4B00h`: load and execute a child program (the overlay loader's path, docs/OVERLAYS.md §1).
    fn exec(&mut self) {
        if self.cpu.al() != 0 {
            self.warn(format!("int 21h AX={:04x}", self.cpu.ax()));
            return self.dos_err(1);
        }
        let key = self.path_at(self.ds_dx());
        let pb = lin(self.cpu.s[ES], self.cpu.bx() as u32);
        let tail_off = self.cpu.mem.rw(pb + 2);
        let tail_seg = self.cpu.mem.rw(pb + 4);
        let ta = lin(tail_seg, tail_off as u32);
        let n = self.cpu.mem.rb(ta) as usize;
        let mut tail = vec![0u8; n.min(126)];
        self.cpu.mem.read_bytes(ta + 1, &mut tail);
        let parent = self.psp;
        match self.load_mz(&key, &tail, parent) {
            Err(e) => {
                self.log.push(format!("exec {key}: DOS error {e}"));
                self.dos_err(e)
            }
            Ok((psp, cs, ip, ss, sp)) => {
                self.log.push(format!("exec {key} at PSP {psp:04x}"));
                let name = key.rsplit('\\').next().unwrap_or(&key).to_string();
                self.images.retain(|im| im.psp != psp);
                self.images.push(Image { name, psp, seg: psp + 0x10 });
                self.arm_hooks();
                // Return frame: pop the INT's IP/CS/FLAGS so the parent resumes after `int 21h`.
                let ret_ip = self.cpu.pop16();
                let ret_cs = self.cpu.pop16();
                let ret_fl = self.cpu.pop16();
                let mut segs = self.cpu.s;
                segs[CS] = ret_cs;
                self.exec_stack.push(ExecFrame { parent_psp: parent, regs: self.cpu.r, segs, ip: ret_ip as u32, flags: ret_fl as u32 });
                if key.ends_with("EL2SETUP.OVL") && self.skip_setup {
                    // Deviation: replace the set-up checks and sound prompt (EL2SETUP:0496)
                    // with a host stub; see DEVIATIONS.md.
                    let a = lin(cs, 0x0496);
                    self.cpu.mem.write_bytes(a, &[0x0F, 0xFF, HLE_SETUP_SKIP, 0xCB]);
                }
                self.psp = psp;
                self.dta = (psp, 0x80);
                let c = &mut self.cpu;
                c.s = [psp, cs, ss, psp, c.s[4], c.s[5]];
                c.ip = ip as u32;
                c.r[ESP] = sp as u32;
                c.r[EAX] = 0;
                c.r[EBX] = 0;
                c.flags = 0x0202;
            }
        }
    }

    fn terminate(&mut self, resident: bool) {
        let code = self.cpu.al();
        let psp = self.psp;
        if resident {
            let paras = self.cpu.dx().max(6);
            let _ = self.resize(psp, paras);
        } else {
            let owned: Vec<u16> = self.blocks.iter().filter(|b| b.owner == psp).map(|b| b.seg).collect();
            for s in owned {
                self.free(s);
            }
            for f in self.files.iter_mut().skip(5) {
                *f = None;
            }
        }
        match self.exec_stack.pop() {
            None => {
                self.log.push(format!("program exited with code {code}"));
                self.exit_code = Some(code);
            }
            Some(fr) => {
                self.psp = fr.parent_psp;
                self.dta = (fr.parent_psp, 0x80);
                let c = &mut self.cpu;
                c.r = fr.regs;
                c.s = fr.segs;
                c.ip = fr.ip;
                c.flags = (fr.flags & !CF) | 0x0002;
                let _ = (EBP, ECX, EDI, ESI);
            }
        }
    }

    // ---- int 33h mouse ------------------------------------------------------------------------

    fn int33(&mut self) {
        let ax = self.cpu.ax();
        match ax {
            0x00 | 0x21 => {
                self.cpu.set_ax(0xFFFF);
                self.cpu.set_bx(2);
                self.mouse.visible = -1;
                self.mouse.handler = None;
            }
            0x01 => self.mouse.visible += 1,
            0x02 => self.mouse.visible -= 1,
            0x03 => {
                let (b, x, y) = (self.mouse.buttons | self.mouse.latched, self.mouse.x, self.mouse.y);
                self.mouse.latched = 0;
                self.cpu.set_bx(b);
                self.cpu.set_cx(x as u16);
                self.cpu.set_dx(y as u16);
            }
            0x04 => {
                self.mouse.x = self.cpu.cx() as i16 as i32;
                self.mouse.y = self.cpu.dx() as i16 as i32;
            }
            0x07 => {
                self.mouse.min_x = self.cpu.cx() as i16 as i32;
                self.mouse.max_x = self.cpu.dx() as i16 as i32;
            }
            0x08 => {
                self.mouse.min_y = self.cpu.cx() as i16 as i32;
                self.mouse.max_y = self.cpu.dx() as i16 as i32;
            }
            0x0B => {
                let (mx, my) = (self.mouse.mickey_x, self.mouse.mickey_y);
                self.cpu.set_cx(mx as u16);
                self.cpu.set_dx(my as u16);
                self.mouse.mickey_x = 0;
                self.mouse.mickey_y = 0;
            }
            0x0C => {
                let h = (self.cpu.cx(), self.cpu.s[ES], self.cpu.dx());
                self.mouse.handler = Some(h);
                self.warn(format!("mouse event handler {:04x}:{:04x} mask {:04x}", h.1, h.2, h.0));
            }
            0x05 | 0x06 => {
                let b = (self.cpu.bx() as usize).min(2);
                let n = if ax == 5 { std::mem::take(&mut self.mouse.press_count[b]) } else { 0 };
                let (st, x, y) = (self.mouse.buttons | self.mouse.latched, self.mouse.x, self.mouse.y);
                self.cpu.set_ax(st);
                self.cpu.set_bx(n);
                self.cpu.set_cx(x as u16);
                self.cpu.set_dx(y as u16);
            }
            0x0F | 0x13 | 0x1A | 0x1D | 0x0A | 0x10 => {}
            0x24 => {
                self.cpu.set_bx(0x0626);
                self.cpu.set_cx(0x0400);
            }
            _ => self.warn(format!("int 33h AX={ax:04x}")),
        }
    }

    /// Host mouse button change (`bit` 1 left, 2 right, 4 middle).
    pub fn mouse_button(&mut self, bit: u16, pressed: bool) {
        let m = &mut self.mouse;
        if pressed {
            m.buttons |= bit;
            m.latched |= bit;
            let i = bit.trailing_zeros() as usize;
            if i < 3 {
                m.press_count[i] = m.press_count[i].saturating_add(1);
            }
        } else {
            m.buttons &= !bit;
        }
    }

    /// Host mouse input, in the game's coordinate range.
    pub fn mouse_move(&mut self, dx: i32, dy: i32) {
        let m = &mut self.mouse;
        m.mickey_x += dx;
        m.mickey_y += dy;
        m.x = (m.x + dx).clamp(m.min_x, m.max_x);
        m.y = (m.y + dy).clamp(m.min_y, m.max_y);
    }

    // ---- int 67h EMS -------------------------------------------------------------------------

    fn ems_map(&mut self, handle: u16, logical: u16, slot: usize) -> u8 {
        if slot > 3 {
            return 0x8B;
        }
        if logical == 0xFFFF {
            self.ems.mapped[slot] = None;
            self.cpu.mem.map_ems(slot, None);
            return 0;
        }
        let Some(Some(pages)) = self.ems.handles.get(handle as usize) else { return 0x83 };
        let Some(&p) = pages.get(logical as usize) else { return 0x8A };
        self.ems.mapped[slot] = Some((handle, logical));
        self.cpu.mem.map_ems(slot, Some(p));
        0
    }

    fn ems_alloc_pages(&mut self, n: usize) -> Option<Vec<usize>> {
        let free: Vec<usize> = (0..self.ems.used.len()).filter(|&i| !self.ems.used[i]).take(n).collect();
        if free.len() < n {
            return None;
        }
        for &p in &free {
            self.ems.used[p] = true;
        }
        Some(free)
    }

    fn set_map(&mut self, map: [Option<(u16, u16)>; 4]) {
        for (slot, m) in map.iter().enumerate() {
            match m {
                Some((h, l)) => {
                    self.ems_map(*h, *l, slot);
                }
                None => {
                    self.ems_map(0, 0xFFFF, slot);
                }
            }
        }
    }

    fn int67(&mut self) {
        let ah = self.cpu.ah();
        let free = self.ems.used.iter().filter(|u| !**u).count() as u16;
        let status: u8 = match ah {
            0x40 => 0,
            0x41 => {
                self.cpu.set_bx(FRAME_SEGMENT);
                0
            }
            0x42 => {
                self.cpu.set_bx(free);
                self.cpu.set_dx(self.ems.used.len() as u16);
                0
            }
            0x43 => {
                let n = self.cpu.bx() as usize;
                match self.ems_alloc_pages(n) {
                    None => 0x88,
                    Some(pages) => {
                        let h = match self.ems.handles.iter().position(|h| h.is_none()) {
                            Some(i) => {
                                self.ems.handles[i] = Some(pages);
                                i
                            }
                            None => {
                                self.ems.handles.push(Some(pages));
                                self.ems.handles.len() - 1
                            }
                        };
                        self.log.push(format!("EMS: allocated {n} pages as handle {h}"));
                        self.cpu.set_dx(h as u16);
                        0
                    }
                }
            }
            0x44 => {
                let (slot, logical, handle) = (self.cpu.al() as usize, self.cpu.bx(), self.cpu.dx());
                self.ems_map(handle, logical, slot)
            }
            0x45 => {
                let h = self.cpu.dx() as usize;
                match self.ems.handles.get_mut(h) {
                    Some(slot @ Some(_)) => {
                        for p in slot.take().unwrap() {
                            self.ems.used[p] = false;
                        }
                        if h == 0 {
                            self.ems.handles[0] = Some(Vec::new());
                        }
                        0
                    }
                    _ => 0x83,
                }
            }
            0x46 => {
                self.cpu.set_al(0x40);
                0
            }
            0x47 => {
                let h = self.cpu.dx();
                let m = self.ems.mapped;
                self.ems.saved.retain(|(x, _)| *x != h);
                self.ems.saved.push((h, m));
                0
            }
            0x48 => {
                let h = self.cpu.dx();
                match self.ems.saved.iter().position(|(x, _)| *x == h) {
                    Some(i) => {
                        let (_, m) = self.ems.saved.remove(i);
                        self.set_map(m);
                        0
                    }
                    None => 0x8E,
                }
            }
            0x4B => {
                let n = self.ems.handles.iter().filter(|h| h.is_some()).count() as u16;
                self.cpu.set_bx(n);
                0
            }
            0x4C => match self.ems.handles.get(self.cpu.dx() as usize) {
                Some(Some(p)) => {
                    let n = p.len() as u16;
                    self.cpu.set_bx(n);
                    0
                }
                _ => 0x83,
            },
            0x4E => {
                // Page map as 4 × (handle, logical) words = 16 bytes.
                let al = self.cpu.al();
                if al == 3 {
                    self.cpu.set_al(16);
                } else {
                    if al == 0 || al == 2 {
                        let a = lin(self.cpu.s[ES], self.cpu.di() as u32);
                        for (i, m) in self.ems.mapped.iter().enumerate() {
                            let (h, l) = m.unwrap_or((0xFFFF, 0xFFFF));
                            self.cpu.mem.ww(a + i as u32 * 4, h);
                            self.cpu.mem.ww(a + i as u32 * 4 + 2, l);
                        }
                    }
                    if al == 1 || al == 2 {
                        let a = lin(self.cpu.s[DS], self.cpu.si() as u32);
                        let mut map = [None; 4];
                        for (i, m) in map.iter_mut().enumerate() {
                            let h = self.cpu.mem.rw(a + i as u32 * 4);
                            let l = self.cpu.mem.rw(a + i as u32 * 4 + 2);
                            *m = if h == 0xFFFF { None } else { Some((h, l)) };
                        }
                        self.set_map(map);
                    }
                }
                0
            }
            0x50 => {
                let handle = self.cpu.dx();
                let a = lin(self.cpu.s[DS], self.cpu.si() as u32);
                let by_seg = self.cpu.al() == 1;
                let mut st = 0;
                for i in 0..self.cpu.cx() as u32 {
                    let logical = self.cpu.mem.rw(a + i * 4);
                    let phys = self.cpu.mem.rw(a + i * 4 + 2);
                    let slot = if by_seg { (phys.wrapping_sub(FRAME_SEGMENT) / 0x400) as usize } else { phys as usize };
                    st = self.ems_map(handle, logical, slot);
                    if st != 0 {
                        break;
                    }
                }
                st
            }
            0x51 => {
                let h = self.cpu.dx() as usize;
                let n = self.cpu.bx() as usize;
                let cur = self.ems.handles.get(h).cloned().flatten();
                match cur {
                    None => 0x83,
                    Some(mut pages) => {
                        if n < pages.len() {
                            for p in pages.drain(n..) {
                                self.ems.used[p] = false;
                            }
                            self.ems.handles[h] = Some(pages);
                            0
                        } else {
                            match self.ems_alloc_pages(n - pages.len()) {
                                None => 0x88,
                                Some(more) => {
                                    pages.extend(more);
                                    self.ems.handles[h] = Some(pages);
                                    0
                                }
                            }
                        }
                    }
                }
            }
            0x58 => {
                if self.cpu.al() == 0 {
                    let a = lin(self.cpu.s[ES], self.cpu.di() as u32);
                    for i in 0..4u32 {
                        self.cpu.mem.ww(a + i * 4, FRAME_SEGMENT + i as u16 * 0x400);
                        self.cpu.mem.ww(a + i * 4 + 2, i as u16);
                    }
                }
                self.cpu.set_cx(4);
                0
            }
            _ => {
                self.warn(format!("int 67h AX={:04x}", self.cpu.ax()));
                0x84
            }
        };
        self.cpu.set_ah(status);
        let _ = (CS, EAX);
    }
}

impl Devices {
    fn outb_eoi(&mut self) {
        use crate::cpu::Io;
        self.outb(0x20, 0x20);
    }
}
