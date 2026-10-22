# Working with the real hardware

Emulators are the usual place to observe a game. They are programs too, and programs can be wrong. The original machine is the final reference. Real hardware helps in three ways. It gives
you a copy of your own media and firmware. It gives measurements when two emulators disagree.
It also shows signals no emulator logs. This page covers the hardware side of the work, for
people who own the machines. Andrew Huang's *Hacking the Xbox* (2003, free from No Starch
Press) is a long example of the method. It covers taking a console apart, reading its buses
and firmware, and documenting what was found.

## Dumping your own media and firmware

| Medium | Tools | Notes |
|---|---|---|
| Cartridges | Open-source cartridge readers for many consoles, and readers for single systems such as the Game Boy | Record the board's chips (mapper, save memory, real-time clock): the emulator needs them |
| CDs and DVDs | Drives and software that read raw sectors and subchannels, as [Redump](http://redump.org/) describes for each system | Compare your hashes with Redump's ([CDs and DVDs](../media/discs.md)) |
| Floppy disks | Flux readers: Greaseweazle, KryoFlux, SuperCard Pro | Keep the flux image as the record ([Floppy disks](../media/floppies.md)) |
| BIOS and firmware | The dumping programs that emulator projects document for each machine | Many emulators need the machine's own BIOS for exact behaviour |
| Save data | Cartridge readers, memory card adapters, the console's own copy features | A real save is a test case for the port's save reader |

Dump each item more than once and compare the hashes. A dirty contact or a weak disc gives a
different result each time. Keep the dumps outside git, as with all game material.

## Measuring

- **Video.** A capture card records what the console really draws: the frame rate, the
  border, the timing of mid-frame effects. Compare it with the emulator and with the port.
- **Sound.** Record the line output and compare it with the emulator's sound. Chip emulation,
  filters and mixing levels differ. A recording of the real machine settles which is right.
- **Timing.** Test programs written by emulator developers measure instruction timing,
  interrupts and video timing on real hardware. Their results, and the emulator's accuracy
  tests built from them, are the reference for the [emulator choice](../observe/running-the-original.md).
- **Signals.** A logic analyser records the signals on a bus or a chip's pins: writes to a
  sound chip, reads from a cartridge, the start-up sequence. Inexpensive USB logic analysers
  with the open-source sigrok and PulseView software decode many buses. A log of every write to
  the sound chip, taken while the original plays, is exact ground truth for a sound driver
  ([Sound](sound.md)).

## Working safely

- Work on hardware you can afford to lose, never the only copy of a machine or a game.
- Use an antistatic mat and wrist strap, and check voltages before you connect anything. Many
  older machines use 5 volt logic, and many modern analysers expect 3.3 volts.
- Prefer clips and sockets to soldering. Photograph every board before and after.
- Record each measurement with the hardware revision, the tools and the settings, as you would
  any evidence ([Handling the game as evidence](../working/evidence.md)).

Reading a console's protected firmware or security keys can fall under laws on circumvention,
even for a machine you own ([Legal](../LEGAL.md)). Dump only what you need for the work. Use
the methods emulator projects document, and decide with the law of your country in mind.
