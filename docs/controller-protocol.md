# PEQUOD Link — controller protocol (v1)

The link between PEQUOD Host and the PEQUOD FC foot controller (ESP32-S3 master + I2C slave boards).
The same messages travel over USB (inside MIDI SysEx) or Wi-Fi (UDP), so everything works the same either way.

## Framing

Every message is one JSON object, written as ASCII (non-ASCII characters escaped as `\uXXXX`), split into frames:

```
0x50 'P'  0x51 'Q'  0x01 version  seq  part  parts  payload...
```

* `seq` 0–127 — message number (wraps); all frames of one message share it
* `part` 0-based frame index, `parts` total frames (1–127)
* payload ≤ 200 bytes per frame; a message may be up to 127 × 200 bytes
* all bytes are < 0x80, so a frame fits in SysEx unchanged

**USB:** each frame is sent as SysEx `F0 7D <frame> F7` (0x7D = non-commercial manufacturer ID) on the controller's USB-MIDI port.
**Wi-Fi:** each frame is one UDP datagram. The controller listens on **UDP 9124**; PEQUOD broadcasts discovery to `255.255.255.255:9124` and the controller answers to the sender's address and port.

A receiver collects frames with the same `seq` until it has `parts` of them, then parses the JSON. Incomplete messages are dropped after 1 s.

## Session

1. PEQUOD sends `hello` every 2 s (to USB ports whose name matches a known controller name, and as a Wi-Fi broadcast) until a controller answers.
2. The controller answers `hello`. PEQUOD then asks for `getdev` and `getcfg`.
3. Both sides send `ping` once a second. No message for 3 s = link lost; PEQUOD goes back to step 1.
4. If both USB and Wi-Fi answer, USB is used.

## Messages — PEQUOD → controller

| `t` | fields | meaning |
|---|---|---|
| `hello` | `app`, `ver`, `proto` | discovery / start a session |
| `ping` | | keep-alive |
| `getcfg` | | reply `cfg` |
| `setcfg` | `cfg` | replace the whole config (applied at once, saved to flash) → `cfgok` |
| `getdev` | | reply `devices` |
| `rescan` | | rescan the I2C bus → `devices` |
| `leds` | `v`: `[[dev, ctrl, on, "#rrggbb" or null], …]` | LED state for controls in PEQUOD mode (current layer) |
| `disp` | `bank`, `preset`, `idx`, `muted`, `msg` | text for the OLED (`msg` shows briefly) |
| `tun` | `n` note, `c` cents, `hz` | tuner reading (≈10/s while muted); `n:"-"` = no note |
| `raw` | `on` 0/1 | stream raw pedal readings (for calibration) |
| `identify` | `dev` | flash that board's LEDs for 3 s |
| `wifiscan` | | → `wifilist` |
| `reboot` | | restart the controller |
| `factory` | | erase settings and restart |

## Messages — controller → PEQUOD

| `t` | fields | meaning |
|---|---|---|
| `hello` | `name`, `fw`, `proto`, `layer`, `layers`, `wifi` | answer to `hello` |
| `ping` | | keep-alive |
| `cfg` | `cfg` | full config |
| `cfgok` | | config saved |
| `devices` | `list`: `[{dev, addr, type, kind, slot}]` | connected boards, by slot |
| `ev` | `dev`, `c`, `l`, `v`, `f` | a control in PEQUOD mode moved: switch `v` 1/0; pedal `v` 0–127, `f` 0–1; encoder `v` = steps (±) |
| `press` | `dev`, `c`, `v` | any switch pressed/released, whatever its mode (used for "press a switch to select it") |
| `layer` | `v` | active layer changed (0-based) |
| `raw` | `dev`, `v` | raw pedal reading (while `raw` is on) |
| `wifi` | `connected`, `ssid`, `ip`, `rssi` | Wi-Fi status changed |
| `wifilist` | `nets`: `[{ssid, rssi, lock}]` | scan result |

`dev` is the board's unique ID (8 hex digits), so assignments follow the board, not the socket it's plugged into.
Control numbers `c`: footswitch/button boards 0–3; pedal 0 = expression, 1 = switch (pedal-with-switch boards); encoder 0 = rotation, 1 = push.

## Config

```json
{
  "name": "Longsleeves",
  "brightness": 200,                // LED brightness 5–255
  "offGlow": 10,                    // % brightness of an LED that is "off" (0 = dark)
  "layers": 1,                      // 1–4
  "layerColors": ["#F44336", "#9C27B0", "#2196F3", "#4CAF50"],
  "midi": { "usb": true, "din": true },
  "wifi": { "enabled": true, "ssid": "", "pass": null, "hasPass": false, "hostname": "pequod-fc" },
  "display": { "contrast": 200, "tunerWhenMuted": true },
  "devices": {
    "A1B2C3D4": {
      "slot": 0, "name": "Board 1",
      "controls": [
        { "layers": [ { "mode": "pequod", "num": 20, "ch": 1, "toggle": true } ],
          "color": "#E0A82E", "led": "auto" }
      ],
      "pedal": { "min": 0, "max": 4095, "invert": false },
      "encoder": { "invert": false }
    }
  }
}
```

* `wifi.pass` is never sent back to PEQUOD (`null` + `hasPass`); sending `null` keeps the stored password.
* Control `mode`
  * switches: `pequod`, `cc`, `note`, `pc`, `layer_next`, `layer_prev`, `layer_set` (`num` = layer 1–4), `layer_hold` (`num` = layer while held), `none`
  * pedal: `pequod`, `cc`, `none`
  * encoder: `pequod`, `cc_rel` (64±steps), `cc_abs` (0–127, kept in the controller), `pc` (steps through programs), `layer` (next/previous by direction), `none`
* `toggle` (cc/note): true = each press flips 127/0 (note on/off); false = 127 while held, 0 on release.
* `led`: `auto` (PEQUOD state in PEQUOD mode; toggle/held/active state otherwise), `on`, `off`, `midi` (follows incoming MIDI on the same number and channel, value ≥ 64 = on).
* Unknown fields are kept, so newer PEQUOD versions can add settings without breaking older firmware.
