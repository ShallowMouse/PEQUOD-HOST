# PEQUOD FC — foot controller firmware

New firmware for the **master** board (ESP32-S3) of the modular foot controller.
The slave boards keep their firmware: the master talks to them with exactly the same I2C commands as before.

With it, PEQUOD Host finds the controller by itself (USB or Wi-Fi) and everything is set up from
PEQUOD's **CONTROLLER** window: what every switch, pedal and knob does, LED colours, layers,
pedal calibration, MIDI out, Wi-Fi, brightness and the controller's name.

## First install (once, with USB)

The Windows download has `Controller firmware/PEQUOD_FC.bin` next to `PEQUOD Host.exe`.

**Arduino IDE** (from `source/PEQUOD_FC.ino`):

| setting | value |
|---|---|
| Board | ESP32S3 Dev Module |
| USB Mode | USB-OTG (TinyUSB) |
| USB CDC On Boot | Disabled |
| Partition Scheme | Default 4MB with spiffs (or whatever you used before — keep it the same to keep your old settings) |

Libraries: **ArduinoJson** (7.x), **Adafruit SSD1306**, **Adafruit GFX Library**.

If the board doesn't show up for uploading, hold BOOT, tap RESET, release BOOT, then upload.

**Or without the IDE:** write `PEQUOD_FC-full.bin` to address `0x0` with
[ESP Web Tools / esptool](https://espressif.github.io/esptool-js/).

On first start the new firmware takes over the old settings it finds — Wi-Fi network, layer count and colours,
LED brightness, MIDI out choice, each board's position, colours, pedal calibration and MIDI numbers.
Every control starts as *Control PEQUOD*; its old MIDI number is kept for when you switch it to *Send MIDI to other gear*.

## Later updates

PEQUOD Host > CONTROLLER > SETTINGS > **UPDATE FIRMWARE**, pick `PEQUOD_FC.bin`. Works over USB or Wi-Fi.

## Using it

* Plug it in with USB, or put it on your Wi-Fi (CONTROLLER > SETTINGS > Wi-Fi). The dot on the CONTROLLER button turns green.
* Press a switch with your foot: the window jumps to it. Choose what it does, and whether it applies to **every preset** or **this preset** only.
* **QUICK SETUP** gives every unassigned control a useful job (the preset's first effects on/off, next preset, presets 1–4 on a second board, tuner on the toe switch / knob push, knob turns through presets).
* The screen shows the bank and preset, flashes what you just switched, and turns into a tuner while PEQUOD is muted.
* Without PEQUOD, controls set to *Send MIDI to other gear* or *Change layer* keep working on their own.

## Testing without the hardware

`tools/fake_controller.py` pretends to be a controller on Wi-Fi so PEQUOD's window can be tried out:

```
python3 Controller/tools/fake_controller.py --script demo
```

The protocol is described in [docs/controller-protocol.md](../docs/controller-protocol.md).
