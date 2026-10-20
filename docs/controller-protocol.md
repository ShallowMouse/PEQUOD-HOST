# PEQUOD Link — controller protocol (v2)

How PEQUOD Host talks to the PEQUOD FC foot controller (ESP32-S3 master, `Controller/PEQUOD_FC`).
The same JSON messages travel over USB (inside MIDI SysEx) or Wi-Fi (UDP), so everything works the same either way.
The slave boards are untouched: the master still talks to them with the original I2C commands.

## Framing

Every message is one JSON object, written as 7-bit ASCII (anything else escaped as `\uXXXX`), split into frames:

| route | frame |
|---|---|
| USB | `F0 7D 'P' 'Q' 'J' id index count <up to 100 bytes of JSON> F7` on the controller's USB-MIDI port |
| Wi-Fi | `'P' 'Q' 'L' '1' 'J' id index count <up to 1000 bytes of JSON>` as one UDP datagram |

* `id` 0–127 — message number (wraps); all frames of one message share it
* `index` 0-based frame number, `count` total frames (1–127)
* 0x7D is the MIDI "non-commercial" manufacturer ID, so other gear ignores the SysEx

The controller listens on **UDP 7400** and answers to whatever address and port a message came from.
It also announces itself with mDNS as `_pequod._udp`.

## Finding each other

1. PEQUOD sends `hello` to every MIDI port whose input and output share a name (it opens the port itself; it does not need to be switched on under AUDIO), and broadcasts `hello` to `255.255.255.255:7400` every 2 s (plus a fixed IP if one is set).
2. The controller answers with `info`. The first answer wins; USB is preferred over Wi-Fi for the same controller.
3. PEQUOD sends `ping` every second; the controller answers `pong`. Nothing heard for 4.5 s = link lost on both sides.
4. On connect PEQUOD asks for `get_cfg`.

## PEQUOD → controller

| `t` | fields | meaning |
|---|---|---|
| `hello` | `app`, `proto` | discovery → `info` |
| `ping` | | → `pong` |
| `bye` | | PEQUOD is closing |
| `get_cfg` | | → `cfg` |
| `set_cfg` | `cfg` | replace the whole config, applied at once, saved to flash 1.5 s later → `cfg_ok` |
| `rescan` | | look for boards again → `info` + `cfg` |
| `leds` | `l`: `[[unit, control, on, "#rrggbb"?], …]` | which PEQUOD-mode controls are lit (everything not listed is off); colour optional |
| `screen` | `mode` `preset`/`tuner`/`text`, `top`, `main`, `sub`, `note`, `cents`, `inTune`, `flash` | OLED content; `flash` shows for 1.2 s at the bottom |
| `layer` | `l` | switch to layer `l` (0-based) |
| `identify` | `u`, `c` (-1 = whole board) | flash that control's LED for 2.5 s |
| `cal` | `u`, `on` | stream raw pedal readings (`raw`) for calibration |
| `wifiscan` | | → `wifilist` |
| `fw_begin` | `size`, `md5`? | start a firmware update → `fw_ack n:-1` |
| `fw_data` | `n`, `d` (base64, ≤ 768 bytes) | chunk `n` (resending the last one is fine) → `fw_ack n` |
| `fw_end` | | check + install → `fw_done`, then the controller restarts |
| `fw_abort` | | cancel the update |
| `reboot` | | restart |
| `factory` | | erase all settings except the name, restart |

## Controller → PEQUOD

| `t` | fields | meaning |
|---|---|---|
| `info` | `proto`, `fw`, `id`, `name`, `via`, `layers`, `layer`, `brightness`, `wifi{connected, ssid, ssid2, ip, rssi}`, `ota{ok, max}`, `units[{slot, type, kind, name, uid}]` | who it is and which boards are connected |
| `pong` | | |
| `cfg` | `cfg` | the full config (passwords left out) |
| `cfg_ok` | | config applied |
| `ev` | `u`, `c`, `l`, `v`, `f`? | a control set to **Control PEQUOD** moved: switch `v` 1/0; pedal `v` 0–127 and `f` 0–1; encoder turn `v` = steps (±) |
| `press` | `u`, `c`, `v` | any switch pressed (1) or released (0), whatever it is set to do — PEQUOD uses it to select the switch in its window |
| `layer` | `l` | the active layer changed |
| `raw` | `u`, `raw`, `v` | raw pedal reading 0–4095 (while `cal` is on) |
| `wifilist` | `nets[{ssid, rssi, lock}]` | scan result |
| `fw_ack` | `n`, `ok`, `err`? | firmware chunk received |
| `fw_done` | `ok`, `err`? | firmware installed (or not) |
| `err` | `msg` | something went wrong |

`u` / `uid` is the board's unique ID (last 8 hex digits of the slave's 64-bit ID, the same key the old firmware used),
so settings follow the board, not the socket it is plugged into.

Control numbers `c`: 0–3 switches (a pedal-with-switch board's toe switch is 0), 100 expression pedal, 200 encoder turn, 201 encoder push.
Unit `kind`: `footswitch`, `button`, `pedal`, `pedal_switch`, `encoder`, `knob`.

## Config

```json
{
  "name": "Longsleeves",              // also the USB-MIDI port name (the controller restarts to apply it)
  "brightness": 200,                  // LED brightness 5-255
  "offGlow": 10,                      // % brightness of an LED that is "off" (0 = dark)
  "layers": 1,                        // 1-4
  "layerColors": ["#F44336", "#9C27B0", "#2196F3", "#4CAF50"],
  "midi": { "usb": true, "din": true },   // where "Send MIDI to other gear" goes
  "wifi": { "enabled": true, "ssid": "", "pass": null, "hasPass": false,
            "ssid2": "", "pass2": null, "hasPass2": false, "hostname": "pequod-fc" },
  "display": { "contrast": 200, "tunerWhenMuted": true },
  "units": {
    "A1B2C3D4": {
      "slot": 0, "name": "Switches 1",
      "controls": {
        "0": { "color": "#E0A82E", "led": "auto",
               "layers": [ { "mode": "pequod", "num": 20, "ch": 1, "toggle": true }, … 4 entries ] }
      },
      "pedal":   { "min": 0, "max": 4095, "invert": false },
      "encoder": { "invert": false }
    }
  }
}
```

* Passwords are never sent to PEQUOD (`null` + `hasPass`); sending `null` keeps the stored one.
* `mode`
  * switches: `pequod` (PEQUOD decides), `cc`, `note`, `pc`, `layer_next`, `layer_prev`, `layer_set` (`num` = layer 1–4), `layer_hold` (`num` = layer while held), `none`
  * pedal: `pequod`, `cc`, `none`
  * encoder turn: `pequod`, `cc_rel` (64 ± steps), `cc_abs` (0–127, kept by the controller), `pc` (steps through programs), `layer`, `none`
* `toggle` (cc/note): true = each press flips 127/0 (note on/off); false = 127 while held, 0 on release.
* `color`: `#rrggbb` or `"layer"` (the colour of the current layer).
* `led`: `auto` (PEQUOD's state for PEQUOD controls; latched/held/active state otherwise), `on`, `off`, `midi` (follows incoming MIDI with the same number and channel, ≥ 64 = on).
* What a PEQUOD-mode control does (effect on/off, next preset, tuner, a plugin knob …) is stored in PEQUOD, with the other MIDI/OSC mappings, not on the controller.
* Fields the controller doesn't know are kept, so newer PEQUOD versions can add settings.
