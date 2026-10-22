"""A small DOS, BIOS and PC for tools/emu/dos_emu/, so a whole 16-bit program can run headless.

    from dos_emu import DosEmu, ProgramStopped
    from dos_env import DosEnv
    emu = DosEmu('target/game/GAME.EXE')
    env = DosEnv(emu, root='target/game', overlay='derived/run')   # files, KERNEL, ports, BIOS
    try:
        emu.run_range(emu.entry, '1000:ca59')      # from the program's entry to a chosen point
        emu.run_to('1000:ca59')                    # then carry on (run_range resets the stack)
    except ProgramStopped as stop:
        print('exit code', stop.args[0])
    print(env.log[-20:])                           # what the program asked of DOS, in order

What it provides:
  - DOS (INT 21h): files under `root` (names matched without case). A file the program writes
    is saved to `overlay` when closed (and kept in `env.written`); root is never written, and
    a file only opened read/write (Pascal's default) is not saved at all. Later opens read the
    overlay's copy first. handles, seek, find first/next with the
    DTA, current drive and directory, date and time (fixed: `env.clock`, so runs repeat
    exactly), version 5.0, interrupt vectors (kept in a table; nothing is ever called through
    them), device information, and exit (ends the run with ProgramStopped(exit code)).
  - DPMI16 KERNEL imports (Borland Pascal 7, Windows 3.x): GlobalAlloc / GlobalLock (64 KB
    selectors in a row, 8 apart, for blocks over 64 KB), GlobalFree, GetFreeSpace, InitTask.
  - Ports: VGA status (0x3DA) alternates so retrace waits end; the DAC (0x3C8/0x3C9) is kept
    in `env.dac` (256 x RGB, 6-bit); PIT, PIC, keyboard, joystick, Sound Blaster and AdLib
    ports answer with safe constant values. Every port write is counted in `env.ports`.
  - BIOS: INT 10h (video modes, recorded in `env.video_mode`), INT 16h (no key waiting),
    INT 33h (no mouse), INT 1Ah and the tick count at 0040:006C (`env.ticks`, one tick per
    128 reads of it, so wait-for-the-next-tick loops end).
  - DPMI (INT 31h): descriptors, DOS memory, interrupt and exception vectors, version, memory
    information; calls into real mode are skipped. INT 2Fh answers "nothing installed".
  - The timer: IRQ 0 goes to INT 8's handler (the program's, or the BIOS's, which counts the
    tick) at the rate the program sets on the PIT, counted in instructions: `ips` per second
    (default 25 million, a fast 486). `env.irq0_count` counts deliveries.
Anything else stops the run with its name (the emulator's own message), so you know what to
add. It does not emulate sound output, a real keyboard (use env.keys), or DPMI services
beyond what is listed.
"""
from unicorn import UC_HOOK_CODE, UC_HOOK_INSN, UC_HOOK_MEM_READ
from unicorn.x86_const import UC_X86_INS_IN, UC_X86_INS_OUT

from dos_emu import FarPtr

from .bios import BiosServices
from .dos import DosServices
from .dpmi import DpmiServices
from .pc import PcHardware

class DosEnv(DosServices, DpmiServices, BiosServices, PcHardware):
    def __init__(self, emu, root, overlay=None, clock=(1999, 8, 31, 12, 0, 0, 0), args='', ips=25_000_000,
                 program='C:\\GAME.EXE', environment=('PATH=C:\\', 'COMSPEC=C:\\COMMAND.COM')):
        self.emu, self.root = emu, root
        self.overlay = overlay
        self.clock = clock              # year, month, day, hour, minute, second, hundredths
        self.ticks = 0
        self.files = {}                 # handle -> [path, data(bytearray), pos, writable]
        self.next_handle = 5
        self.vectors = {}
        self.dta = None
        self.find = None
        self.log = []
        self.stdout = bytearray()
        self.dac = bytearray(768)
        self.dac_index = self.dac_read = 0
        self.video_mode = None
        self.ports = {}
        self.retrace = 0
        self.pit, self.pit_high = 0xFFFF, False
        emu.interrupts.update({0x21: self.int21, 0x10: self.int10, 0x16: self.int16,
                               0x33: self.int33, 0x1A: self.int1a, 0x31: self.int31,
                               0x2F: self.int2f})
        self.dos_mem = 0x2000                 # next free real-mode paragraph for DOS memory
        self.pm_vectors, self.rm_vectors, self.exc_vectors = {}, {}, {}
        self.callbacks = []
        self.keys = []                        # BIOS keyboard queue: scan << 8 | ascii
        self.written = {}                     # files the program wrote: NAME -> bytes
        emu.stubs.update({'KERNEL.15': self.global_alloc, 'KERNEL.17': self.global_free,
                          'KERNEL.18': self.global_lock, 'KERNEL.169': self.get_free_space,
                          'KERNEL.91': self.init_task,
                          'RTM.5': self.rtm_alloc, 'RTM.6': self.rtm_free, 'RTM.21': self.rtm_free_mem})
        self.init_bios_data()
        self.psp = self.make_psp(args, program, environment)
        # The BIOS's own interrupt handlers: an IRET for every vector (INT 8 also counts the
        # BIOS tick), so a program that chains to the old handler returns cleanly.
        self.bios = emu.new_selector(access=0x9A)
        emu.write(FarPtr(self.bios, 0), b'\xCF' * 0x400)
        at = emu.linear(self.bios, 8 * 4)
        emu.uc.hook_add(UC_HOOK_CODE, self.bios_int8, begin=at, end=at)
        # The timer: IRQ 0 every (instructions per second / tick rate) instructions.
        self.ips = ips
        self.divisor = 0x10000
        self.pit_write_high = False
        self.irq0_count = 0
        emu.on_slice = self.on_slice
        self.set_timer_slice()
        emu.uc.hook_add(UC_HOOK_MEM_READ, self.bios_tick_read, begin=0x46C, end=0x46F)
        self.tick_reads = 0
        emu.uc.hook_add(UC_HOOK_INSN, self.port_in, None, 1, 0, UC_X86_INS_IN)
        emu.uc.hook_add(UC_HOOK_INSN, self.port_out, None, 1, 0, UC_X86_INS_OUT)
