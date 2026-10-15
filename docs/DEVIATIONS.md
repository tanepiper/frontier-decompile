# Deviations from the original

This lists the places where the reimplementation deliberately differs from the DOS original. Everything
not listed here should match the original 1:1. In code, each deviation is marked with a
`// Deviation:` comment that names the original address it replaces.

## Policy: no legacy hardware setup

The reimplementation reproduces the **game**, not the 1993 PC configuration around it. It skips any
step that only exists to set up DOS hardware or memory and goes straight into the game.

| Original behaviour | Reimplementation | Status |
|---|---|---|
| Text-mode prompt "Please select sound support required" (1 Roland LAPC1, 2 AdLib / SoundBlaster, 3 PC speaker, 4 No sound) before the game starts | Not shown. Boot goes straight to the game's own start-up (intro/menu) | Decided; prompt routine not yet traced (M1) |
| Choosing a sound driver overlay (`EL2ROL`, `EL2ADLIB`, `EL2SPK`) | No driver choice. Audio reproduces one reference output (AdLib / SoundBlaster, option 2, which the DOSBox reference config also uses) through the platform's audio API | Decided; reference driver may be revisited when audio is traced |
| Video driver overlay (`EL2MCGA3`) and VGA mode 13h set-up | The 320×200 palettised frame is produced in software and presented with wgpu | Decided |
| DOS memory checks ("Not enough free memory") and overlay loading | Not reproduced. Overlay code is ported like the rest of the game | Decided |

Anything the player sees as part of the game itself stays 1:1, including the intro, menus, the
in-game options screens, gameplay and the save format. To compare boot captures with DOSBox, answer
the sound prompt with **2** so the reference uses the same driver.
