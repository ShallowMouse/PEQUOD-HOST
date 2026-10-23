# PEQUOD Host

A standalone VST3 host for Windows with a Guitar Rig-style layout, MIDI and OSC control, and the PEQUOD look.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ PEQUOD   < PRESET NAME >  INIT SAVE SAVE-AS  LEARN      AUDIO PLUGINS  IN  OUT│
├────────────┬───────────────────────────────────────────────┬─────────────────┤
│ PLUGINS |  │ ● 01 MODULE NAME            EDIT  -  X        │ OSC             │
│ PRESETS    │ ┌───────── plugin panel ──────────┐          │ MIDI INPUTS     │
│ [search]   │ └─────────────────────────────────┘          │ GUITAR INPUT    │
│ MAKER      │ SPLIT 1      A ──────■────── B            X   │ MAPPINGS        │
│  Plugin    │ │ A: ● 02 ...       │ B: ● 03 ...      │      │  CC 20 -> ...   │
│  Plugin    │ + DROP PLUGIN                    + SPLIT      │  /fader -> ...  │
└────────────┴───────────────────────────────────────────────┴─────────────────┘
```

## Using it

- **Add plugins**: drag from the browser into the rack (or into a split lane), or double-click to add at the end. First launch scans your VST3 folder; use **PLUGINS** to rescan.
- **Rack modules**: power LED = on/off, **EDIT** = floating window, **+/-** = show/hide the plugin panel in the rack, drag the module header to reorder or to move it into a split lane.
- **Rack or pedalboard**: the **RACK / PEDALBOARD** buttons at the bottom right switch between the vertical rack and a horizontal pedalboard (modules left to right, scrolling sideways). Everything else works the same; your choice is remembered.
- **Split**: **+ SPLIT** adds an A/B parallel module. The fader crossfades: centre plays both lanes at full level, the ends fade the other lane out.
- **Presets and banks**: presets live in banks, which are folders under `Documents\PEQUOD Host\Presets` (presets from before banks existed are moved into **Default**). The PRESETS tab lists every bank; click a bank to make it current, **+ BANK** makes a new one, right-click a bank to rename or delete it, right-click a preset to move it to another bank. **SAVE AS** asks which bank to save into. `<` / `>` step through the presets in the current bank; click the name for every bank and preset. Ctrl+S saves, Ctrl+Shift+S saves as. Your last session is restored on launch.
- **Guitar input**: defaults to *Left (input 1)* copied to both channels, so a guitar in input 1 comes out in stereo. Change it under **AUDIO**.
- **AUDIO**: the **Audio & MIDI** tab has the guitar input, the audio device, sample rate and buffer size (ASIO, WASAPI, DirectSound) and which MIDI inputs are enabled; the **OSC** tab has the ports, send-sync and a list of OSC addresses.
- **PLUGINS**: the VST3 folders to search are listed at the top (add, remove or reset them); **SCAN FOLDERS** looks for new or updated plugins there.
- **Tuner**: always listening to the guitar input (top bar). Right-click it to change the reference pitch (A = 432–444 Hz). **MUTE** silences the output, e.g. while tuning; right-click it to learn a footswitch.
- **Multi-core**: plugins in the two lanes of a split always run on separate CPU cores, with no added latency. To spread a long serial chain over more cores, click the **CPU** readout in the top bar and pick 2–4 cores; each extra core adds one buffer of latency (5.3 ms at 256 samples / 48 kHz), shown in the menu. The chain is split between cores automatically by measured CPU use, and every module shows its own CPU share so you can find the heavy ones.
- **Effects map** (right panel): the whole signal chain at a glance. Click a block to jump to it in the rack, click its LED to switch it on or off.
- **Preset switching**: plugins the next preset shares with the current one stay loaded and just take the new settings, and plugins you used recently are kept loaded in the background (up to 12). Anything new is loaded while the current preset keeps playing; then the output fades out and back in over a few milliseconds while the chain is swapped. No gap, no burst of dry guitar.
- **Undo / redo**: **UNDO** and **REDO** at the bottom right, or Ctrl+Z / Ctrl+Y (Ctrl+Shift+Z). Undoes adding, removing and moving modules and splits, and **INIT** (up to 40 steps). Loading another preset starts a fresh history. Knob changes inside a plugin use that plugin's own undo.
- **Crash protection**: if a plugin crashes while processing audio, PEQUOD takes it out of the signal and keeps playing; the module shows **CRASHED - SKIPPED** (restart PEQUOD to use it again). If PEQUOD itself didn't close properly, the next start asks how to start: without the plugin it crashed in (when known), with an empty rack, or normally. Your saved presets are never changed.
- **Output safety**: a limiter just under 0 dB on the output protects ears and speakers from a runaway plugin (right-click **OUT** to turn it off). The light on top of the OUT meter turns red if the output hit 0 dB (click it to clear), and mustard while the limiter is working.
- **Level per preset**: the **OUT** level is saved with each preset. To even out presets, right-click **OUT** > *Use this preset's level as the target* and play for 6 seconds; then on each other preset, right-click **OUT** > *Match this preset to the target* and play the same way. OUT is set for you; save the preset to keep it.
- **Instruments (VSTi)**: add a synth, piano etc. from the browser like any plugin (they're marked INSTRUMENT) and play it from any MIDI keyboard (enable it under **AUDIO**), or click **KEYS** at the bottom for an on-screen keyboard (also playable from the computer keyboard: A W S E D F T G Y H U J K). The **INPUT: GUITAR / INSTRUMENT** button at the bottom switches between them: on INSTRUMENT the guitar is muted so only the instrument is heard (the tuner still listens) and the IN label shows **IN OFF**. It's saved with each preset, and can go on a footswitch (right-click it to learn, or pick *Input: guitar / instrument* in the controller window; OSC `/pequod/instrument`). Effects after the instrument in the rack process it.

## MIDI

- All MIDI inputs are enabled on first run (toggle them under **AUDIO**). MIDI is also passed to any instrument plugins.
- **Program Change** loads the preset at that position in the current bank (PC 0 = first preset). **Bank Select (CC 0)** chooses the bank (0 = first bank); the next Program Change loads from it. If you map CC 0 to something with LEARN, that mapping wins.
- **Learn**: click **LEARN**, move a control on any plugin panel, then move a MIDI knob/fader/footswitch (CC or note). Right-click an on/off LED, a split fader or the IN/OUT knobs to learn those directly. **+ HOST** learns preset next/prev, bank next/prev, master levels and MUTE. Bank next/prev keeps the same preset number in the new bank.
- Footswitch mappings default to **Toggle** (each press flips the state). Click a mapping and pick **Direct** for latching pedals that send 127 = on, 0 = off.
- Mappings to plugins, on/off and split faders are saved with the preset; host mappings (preset/bank next/prev, master, mute) are global.
- Every module and split has a permanent ID saved in the preset, and mappings point at that ID, so reordering modules or a plugin failing to load never moves a mapping onto the wrong module. (Mappings to a module that didn't load are skipped.) Presets from earlier versions get IDs the next time they're saved.

## OSC

Receives on port **9000**, sends feedback to **127.0.0.1:9001** by default (change both in the right panel). Slots are numbered **1, 2, 3...** in rack order, lane A before lane B; the number is shown on each module. Parameters are numbered from 1.

| Address | Args | Does |
|---|---|---|
| `/pequod/slot/{n}/param/{p}` | float 0–1 | Set parameter *p* of slot *n* |
| `/pequod/slot/{n}/on` | 0/1 | Slot on/off |
| `/pequod/slot/{n}/bypass` | 0/1 | Same, inverted |
| `/pequod/split/{k}/mix` | float 0–1 | Split *k* A/B fader |
| `/pequod/master/in`, `/pequod/master/out` | float 0–1 | Level, −36 dB to +12 dB (0.75 = 0 dB) |
| `/pequod/preset/next`, `/pequod/preset/prev` | — | Step presets in the current bank |
| `/pequod/preset/load` | int (1-based, in the bank) or string name | Load preset |
| `/pequod/bank/next`, `/pequod/bank/prev` | — | Step banks (loads the same preset number) |
| `/pequod/bank/load` | int (1-based) or string name | Switch bank and load a preset |
| `/pequod/bank/select` | int (1-based) | Switch bank without loading |
| `/pequod/mute` | 0/1 (no arg = toggle) | Output mute |
| `/pequod/sync` | — | Send the full state back (all names and values) |

Any other address can be mapped with **LEARN**: touch a plugin control, then send the OSC message.

**Feedback sent out** (so TouchOSC, an ESP32 controller etc. stay in sync): every parameter change as `/pequod/slot/{n}/param/{p} f`, plus `/pequod/rack/count`, `/pequod/slot/{n}/name`, `/pequod/slot/{n}/on`, `/pequod/slot/{n}/params` (count), `/pequod/split/{k}/mix`, `/pequod/master/in|out`, `/pequod/mute`, `/pequod/tuner/note` (s, `-` when silent), `/pequod/tuner/cents` (f), `/pequod/tuner/hz` (f), `/pequod/bank/name`, `/pequod/bank/index`, `/pequod/preset/name`, `/pequod/preset/index`. `/pequod/sync` also sends `/pequod/slot/{n}/param/{p}/name`. Learned OSC addresses get feedback on their own address too.

## Foot controller (PEQUOD FC)

PEQUOD has native support for the ESP32-S3 modular foot controller (new master firmware in `Controller/PEQUOD_FC`,
the slave boards are unchanged). Plug it in with USB or put it on Wi-Fi and the **CONTROLLER** button's dot turns green.

* Press a switch with your foot and the CONTROLLER window jumps to it; pick what it does in plain words (one or several functions per switch) —
  effect on/off, a plugin knob, next/previous/specific preset,
  banks, tuner, mute, split A/B — or let it send its own MIDI to other gear, or change layers.
* Each footswitch is either **GLOBAL** (the same functions in every preset) or **THIS PRESET** (each preset gives it its own), never both.
  Global plugin functions follow the plugin by name. Global switches have a dashed ring. MIDI and OSC mappings can be global or per preset too.
* LEDs can glow in the colour of the effect they switch (the PEQUOD category colours: oxblood amps, tangerine drive, ocean delays…;
  other plugins go by their type). LEDs show what's on, the screen shows the bank and preset, flashes what you switched, and becomes a tuner while muted.
* Everything the controller stores is edited there too: colours, layers, pedal calibration, MIDI out (USB / 5-pin), Wi-Fi,
  brightness, name, firmware updates.

Setup and flashing: [Controller/README.md](Controller/README.md). Protocol: [docs/controller-protocol.md](docs/controller-protocol.md).

**Screen layout**: the **SCREEN** tab in the controller window lays out the controller's screen. The preview is the real screen, pixel for pixel. Add text, lines and boxes; drag them around (or use the arrow keys); pick the size (1-4), left / centre / right and black-on-white. Text can include things PEQUOD fills in (**INSERT...**): `{preset}`, `{bank}`, `{num}`, `{next}`, `{bpm}`, `{beat}`, `{click}`, `{layer}`, `{msg}` (what a switch just did), `{fx 1}` / `{on 1}` (name / ON-OFF of effect 1, any number), `{mute}`, `{name}`, `{time}`. Turn off the white message bar to show messages only where you put `{msg}`. The layout is stored on the controller. Needs controller firmware 2.2 or newer (older firmware keeps the standard screen).

## Building

**GitHub (no local tools needed)**: push this folder to a GitHub repo. The *Build Windows* workflow builds on every push; download `PEQUOD-Host-Windows` from the run's *Artifacts*. Pushing a tag like `v1.0.0` also attaches the .exe to a release.

**Locally**: Visual Studio 2022 (Desktop C++ workload) and CMake 3.22+:

```
cmake -S . -B build -G "Visual Studio 17 2022" -A x64
cmake --build build --config Release
```

The exe is at `build\PequodHost_artefacts\Release\PEQUOD Host.exe`. JUCE 8.0.15 is downloaded automatically (or pass `-DJUCE_DIR=path\to\JUCE`). The exe is self-contained (static runtime, fonts embedded).

## Notes

- Built on JUCE 8, which is dual-licensed: AGPLv3 or a commercial JUCE licence. Personal use is fine; selling PEQUOD Host closed-source would need a JUCE licence.
- Fonts: Courier Prime (SIL OFL 1.1) and Roboto (Apache 2.0), licences in `Resources/Fonts`.
- Plugins load in-process; a plugin that crashes during scanning is remembered and skipped next time.
- Settings live in `%APPDATA%\PEQUOD\PEQUOD Host.settings`.
