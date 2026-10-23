/* ============================================================================
   PEQUOD FC — master firmware for the ESP32-S3 foot controller
   ----------------------------------------------------------------------------
   * Talks to the existing slave boards with exactly the same I2C commands as
     the previous master (0x00 id, 0x01 type, 0x02 read, 0x04 encoder,
     0x20/0x21/0x22 LEDs), so the slave firmware is unchanged.
   * Native link to PEQUOD Host over USB (MIDI SysEx) and Wi-Fi (UDP 7400),
     see docs/controller-protocol.md. Every setting is edited from PEQUOD.
   * Controls can also send plain MIDI (USB + 5-pin DIN) to other gear.

   Arduino IDE settings
     Board ............ ESP32S3 Dev Module
     USB Mode ......... USB-OTG (TinyUSB)
     USB CDC On Boot .. Disabled   (the USB port is a pure MIDI device named after
                                    the controller; debug output goes to the UART pins)
     Partition Scheme . any scheme with OTA + SPIFFS, e.g. "Default 4MB with spiffs"
                        (keep the same scheme as before to keep your old settings)
   Libraries: ArduinoJson 7, Adafruit SSD1306, Adafruit GFX
   ============================================================================ */

#include <Arduino.h>
#include <Wire.h>
#include <WiFi.h>
#include <WiFiUdp.h>
#include <ESPmDNS.h>
#include <SPIFFS.h>
#include <Preferences.h>
#include <Update.h>
#include <ArduinoJson.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>
#include "USB.h"
#include "USBMIDI.h"
#include "mbedtls/base64.h"
#include "esp_ota_ops.h"

#define FW_VERSION    "2.2.0"
#define PROTO_VERSION 4     // 3: a control can do several things at once (PEQUOD + MIDI + layer)
                            // 4: screen "layout" mode (PEQUOD lays out the screen, see drawLayout)

// ── Hardware (same as the previous master) ──────────────────────────────────
static const int SDA_PIN = 8, SCL_PIN = 18;          // slave bus (Wire)
static const int OLED_SDA = 4, OLED_SCL = 5;         // display bus (Wire1)
static const uint8_t OLED_ADDR = 0x3C;
static const int MIDI_TX = 17, MIDI_RX = 16;         // 5-pin DIN (Serial1)
static const int RGB_PIN = 48;                       // onboard RGB LED
static const uint16_t LINK_PORT = 7400;
static const unsigned long HOST_TIMEOUT_MS = 4500;

static const int MAX_BOARDS = 16;
static const int MAX_SLOTS = 8;
static const int MAX_CTRLS = 4;
static const int MAX_LAYERS = 4;

// Control numbers on the link (and in the settings):
//   0-3 = switches (a pedal-with-switch board's switch is 0), 100 = expression pedal,
//   200 = encoder rotation, 201 = encoder push
static const int C_PEDAL = 100, C_ENC_TURN = 200, C_ENC_PUSH = 201;

// ── Types ───────────────────────────────────────────────────────────────────
enum Kind : uint8_t { KIND_NONE, KIND_FOOTSWITCH, KIND_BUTTON, KIND_PEDAL, KIND_PEDAL_SWITCH, KIND_ENCODER, KIND_KNOB };

enum Mode : uint8_t {
  M_NONE, M_PEQUOD, M_CC, M_NOTE, M_PC,
  M_LAYER_NEXT, M_LAYER_PREV, M_LAYER_SET, M_LAYER_HOLD,
  M_CC_REL, M_CC_ABS, M_LAYER_DIR
};

enum LedMode : uint8_t { LED_AUTO, LED_ON, LED_OFF, LED_MIDI };

// One control (on one layer) can do several things at once: always tells PEQUOD (when connected),
// plus up to 4 MIDI messages to other gear, plus one layer change.
struct MidiMsg { uint8_t type = M_CC; uint8_t num = 0; uint8_t ch = 1; uint8_t absValue = 0; };

struct Action {
  MidiMsg midi[4];
  uint8_t nMidi = 0;
  uint8_t layerMode = M_NONE;    // M_LAYER_NEXT / PREV / SET / HOLD, or M_LAYER_DIR for an encoder
  uint8_t layerNum = 1;
  bool toggle = true;            // latching (shared by its MIDI messages)
};

struct Control {
  int code = 0;                 // link number (see above)
  Action act[MAX_LAYERS];
  uint32_t color = 0xE0A82E;
  bool colorIsLayer = false;
  uint8_t led = LED_AUTO;

  // runtime
  bool down = false;
  uint8_t pressLayer = 0;
  bool toggled[MAX_LAYERS] = { false, false, false, false };
  bool pqOn = false;
  bool pqHasColor = false;
  uint32_t pqColor = 0;
  bool midiOn = false;
  uint8_t absValue = 0;
};

struct Board {
  uint8_t addr = 0;
  uint64_t id = 0;
  uint8_t type = 0;
  char key[9] = { 0 };          // 8 hex digits of the slave's unique ID (same key as the old firmware)
  Kind kind = KIND_NONE;
  int slot = -1;
  int numCtrls = 0;
  Control ctl[MAX_CTRLS];

  int pedalMin = 0, pedalMax = 4095;
  bool pedalInvert = false;
  bool encInvert = false;

  // runtime
  uint8_t lastButtons = 0;
  int lastPedal10 = -100;
  int lastPedal7 = -1;
  int rawPedal = 0;
  int16_t lastEncPos = 0;
  bool encPosValid = false;
  uint8_t lastPedalBtn = 0;
  unsigned long failUntil = 0;
  unsigned long identifyUntil = 0;
  int identifyCode = -1;        // -1 = whole board
  bool calWatch = false;
};

struct Reassembler {
  String text;
  int id = -1, next = 0, count = 0;
};

enum Transport : uint8_t { T_USB = 0, T_UDP = 1, T_NONE = 2 };

// ── Globals ─────────────────────────────────────────────────────────────────
USBMIDI usbMidi;
WiFiUDP udp;
bool udpRunning = false;
Adafruit_SSD1306 display(128, 64, &Wire1, -1);
bool displayOk = false;

Board boards[MAX_BOARDS];
int numBoards = 0;

JsonDocument cfg;            // master copy of the settings (what PEQUOD edits)
bool cfgDirty = false;
unsigned long cfgDirtySince = 0;

int numLayers = 1;
int layer = 0;
int holdReturnLayer = -1;
uint32_t layerColors[MAX_LAYERS] = { 0xF44336, 0x9C27B0, 0x2196F3, 0x4CAF50 };
uint8_t brightness = 200;
uint8_t offGlow = 10;
bool midiUsb = true, midiDin = true;
uint8_t lastPcCh = 0, lastPcNum = 255;

// link
Transport active = T_NONE;
unsigned long lastRx[2] = { 0, 0 };
IPAddress udpPeer;
uint16_t udpPeerPort = 0;
uint8_t txId = 0;
Reassembler rxUsb, rxUdp;
bool wasLinked = false;

// screen content sent by PEQUOD
struct Screen {
  bool valid = false;
  String mode = "preset", top, main, sub, note, flash;
  float cents = 0;
  bool inTune = false;
  unsigned long flashUntil = 0;
} screen;

// "layout" screens: PEQUOD sends ready-made items, the controller just draws them
struct ScreenItem {
  char kind = 't';              // 't' text, 'b' filled box, 'r' outline
  int16_t x = 0, y = 0, w = 0, h = 0;
  uint8_t size = 1, align = 0;  // text: size 1-4, align 0 left / 1 centre / 2 right (x is that edge / the centre)
  bool inv = false;             // text: black on a white box
  String text;
};
const int MAX_ITEMS = 24;
ScreenItem items[MAX_ITEMS];
int numItems = 0;
bool displayDirty = true;

// firmware update
struct FwUpdate { bool running = false; int lastN = -1; size_t written = 0, size = 0; } fw;

// wifi
bool wifiWasConnected = false;
bool wifiScanPending = false;
unsigned long wifiStartedAt = 0;
bool usingFallbackWifi = false;

// ── Small helpers ───────────────────────────────────────────────────────────
uint32_t parseColor(const char* s, uint32_t fallback) {
  if (!s || !*s) return fallback;
  if (*s == '#') s++;
  char* end = nullptr;
  unsigned long v = strtoul(s, &end, 16);
  return (end && end != s) ? (uint32_t) (v & 0xFFFFFF) : fallback;
}

String colorToHex(uint32_t c) {
  char b[8];
  snprintf(b, sizeof(b), "#%06lX", (unsigned long) (c & 0xFFFFFF));
  return String(b);
}

uint8_t scaleChan(uint8_t v, uint8_t scale) {
  if (v == 0) return 0;
  uint16_t s = ((uint16_t) v * scale) / 255;
  return s == 0 ? 1 : (uint8_t) s;    // like FastLED nscale8_video: dim colours stay visible
}

uint32_t scaleColor(uint32_t c, uint8_t scale) {
  return ((uint32_t) scaleChan((c >> 16) & 0xFF, scale) << 16)
       | ((uint32_t) scaleChan((c >> 8) & 0xFF, scale) << 8)
       |  (uint32_t) scaleChan(c & 0xFF, scale);
}

// LEDs are linear but eyes aren't: without this a colour's smaller channels look far too
// strong at full brightness (red goes pink, yellow goes white). Gamma 2.4 keeps colours saturated.
uint8_t gammaLut[256];
void buildGamma() {
  for (int i = 0; i < 256; i++) {
    float v = powf(i / 255.0f, 2.4f) * 255.0f + 0.5f;
    gammaLut[i] = (i > 0 && v < 1.0f) ? 1 : (uint8_t) v;
  }
}

uint32_t trueColor(uint32_t c) {
  return ((uint32_t) gammaLut[(c >> 16) & 0xFF] << 16) | ((uint32_t) gammaLut[(c >> 8) & 0xFF] << 8) | gammaLut[c & 0xFF];
}

Kind kindForType(uint8_t t) {
  if (t == 41) return KIND_PEDAL_SWITCH;
  if (t >= 20 && t <= 39) return KIND_FOOTSWITCH;
  if (t >= 60 && t <= 79) return KIND_BUTTON;
  if (t >= 40 && t <= 59) return KIND_PEDAL;
  if (t >= 100 && t <= 119) return KIND_ENCODER;
  if (t >= 80 && t <= 99) return KIND_KNOB;
  return KIND_NONE;
}

const char* kindName(Kind k) {
  switch (k) {
    case KIND_FOOTSWITCH:   return "footswitch";
    case KIND_BUTTON:       return "button";
    case KIND_PEDAL:        return "pedal";
    case KIND_PEDAL_SWITCH: return "pedal_switch";
    case KIND_ENCODER:      return "encoder";
    case KIND_KNOB:         return "knob";
    default:                return "other";
  }
}

bool hasSwitchRow(const Board& b) { return b.kind == KIND_FOOTSWITCH || b.kind == KIND_BUTTON; }

// The link numbers of a board's controls, in order.
int controlCodes(const Board& b, int out[MAX_CTRLS]) {
  switch (b.kind) {
    case KIND_FOOTSWITCH:
    case KIND_BUTTON:       for (int i = 0; i < 4; i++) out[i] = i; return 4;
    case KIND_PEDAL:        out[0] = C_PEDAL; return 1;
    case KIND_PEDAL_SWITCH: out[0] = C_PEDAL; out[1] = 0; return 2;
    case KIND_ENCODER:      out[0] = C_ENC_TURN; out[1] = C_ENC_PUSH; return 2;
    default:                return 0;
  }
}

bool isSwitchCode(int code) { return (code >= 0 && code < 4) || code == C_ENC_PUSH; }

Control* findControl(Board& b, int code) {
  for (int i = 0; i < b.numCtrls; i++)
    if (b.ctl[i].code == code) return &b.ctl[i];
  return nullptr;
}

// Physical LED number on the slave for a control, or 0 if it has no LED.
int ledIndex(const Board& b, int code) {
  if (hasSwitchRow(b) && code >= 0 && code < 4) return code + 1;
  if (b.kind == KIND_PEDAL_SWITCH && code == 0) return 1;
  return 0;
}

Board* boardByKey(const char* key) {
  for (int i = 0; i < numBoards; i++)
    if (strcmp(boards[i].key, key) == 0) return &boards[i];
  return nullptr;
}

const char* modeName(uint8_t m) {
  switch (m) {
    case M_PEQUOD: return "pequod";      case M_CC: return "cc";            case M_NOTE: return "note";
    case M_PC: return "pc";              case M_LAYER_NEXT: return "layer_next";
    case M_LAYER_PREV: return "layer_prev"; case M_LAYER_SET: return "layer_set";
    case M_LAYER_HOLD: return "layer_hold"; case M_CC_REL: return "cc_rel";
    case M_CC_ABS: return "cc_abs";      case M_LAYER_DIR: return "layer";
    default: return "none";
  }
}

uint8_t modeFromName(const char* s) {
  if (!s) return M_NONE;
  for (uint8_t m = M_NONE; m <= M_LAYER_DIR; m++)
    if (strcmp(s, modeName(m)) == 0) return m;
  return M_NONE;
}

uint8_t ledFromName(const char* s) {
  if (!s) return LED_AUTO;
  if (!strcmp(s, "on")) return LED_ON;
  if (!strcmp(s, "off")) return LED_OFF;
  if (!strcmp(s, "midi")) return LED_MIDI;
  return LED_AUTO;
}

bool linked() {
  return active != T_NONE && millis() - lastRx[active] < HOST_TIMEOUT_MS;
}

String macId() {
  uint64_t mac = ESP.getEfuseMac();
  char buf[13];
  snprintf(buf, sizeof(buf), "%012llX", (unsigned long long) (mac & 0xFFFFFFFFFFFFULL));
  return String(buf);
}

// ── MIDI out ────────────────────────────────────────────────────────────────
void dinWrite3(uint8_t a, uint8_t b, uint8_t c) { Serial1.write(a); Serial1.write(b); Serial1.write(c); }

void midiCC(uint8_t num, uint8_t val, uint8_t ch) {
  ch = constrain(ch, 1, 16);
  if (midiUsb) usbMidi.controlChange(num & 0x7F, val & 0x7F, ch);
  if (midiDin) dinWrite3(0xB0 | (ch - 1), num & 0x7F, val & 0x7F);
}

void midiNote(uint8_t num, bool on, uint8_t ch) {
  ch = constrain(ch, 1, 16);
  if (midiUsb) { if (on) usbMidi.noteOn(num & 0x7F, 127, ch); else usbMidi.noteOff(num & 0x7F, 0, ch); }
  if (midiDin) dinWrite3((on ? 0x90 : 0x80) | (ch - 1), num & 0x7F, on ? 127 : 0);
}

void midiPC(uint8_t num, uint8_t ch) {
  ch = constrain(ch, 1, 16);
  if (midiUsb) usbMidi.programChange(num & 0x7F, ch);
  if (midiDin) { Serial1.write(0xC0 | (ch - 1)); Serial1.write(num & 0x7F); }
  lastPcCh = ch;
  lastPcNum = num;
}

// ── Link: framing ───────────────────────────────────────────────────────────
//   USB  : F0 7D 'P' 'Q' 'J' id index count <≤100 bytes of JSON> F7
//   Wi-Fi: "PQL1" 'J' id index count <≤1000 bytes of JSON>          (UDP 7400)
void usbSendSysEx(const uint8_t* data, size_t n) {
  // USB-MIDI 1.0 SysEx: CIN 0x4 = start/continue (3 bytes), 0x5/0x6/0x7 = end with 1/2/3 bytes.
  size_t i = 0;
  while (i < n) {
    size_t left = n - i;
    midiEventPacket_t p;
    uint8_t b[3] = { 0, 0, 0 };
    uint8_t cin;
    if (left > 3) { cin = 0x4; memcpy(b, data + i, 3); i += 3; }
    else          { cin = (uint8_t) (0x4 + left); memcpy(b, data + i, left); i += left; }
    p.header = cin;
    p.byte1 = b[0]; p.byte2 = b[1]; p.byte3 = b[2];
    unsigned long t0 = millis();
    while (!usbMidi.writePacket(&p)) {           // FIFO full: wait a little
      if (millis() - t0 > 20) return;            // host isn't reading; give up quietly
      delay(1);
    }
  }
}

void sendTextOn(Transport tr, const String& json) {
  if (tr == T_NONE) return;
  if (tr == T_UDP && (!udpRunning || udpPeerPort == 0)) return;
  const int chunk = (tr == T_USB) ? 100 : 1000;
  const int len = json.length();
  const int count = max(1, (len + chunk - 1) / chunk);
  if (count > 127) return;
  const uint8_t id = (txId++) & 0x7F;
  static uint8_t frame[4 + 5 + 1000 + 1];
  for (int i = 0; i < count; i++) {
    int n = 0;
    if (tr == T_USB) { frame[n++] = 0xF0; frame[n++] = 0x7D; frame[n++] = 'P'; frame[n++] = 'Q'; }
    else             { frame[n++] = 'P'; frame[n++] = 'Q'; frame[n++] = 'L'; frame[n++] = '1'; }
    frame[n++] = 'J'; frame[n++] = id; frame[n++] = (uint8_t) i; frame[n++] = (uint8_t) count;
    const int off = i * chunk, m = min(chunk, len - off);
    for (int k = 0; k < m; k++) {
      uint8_t ch = (uint8_t) json[off + k];
      frame[n++] = ch < 0x80 ? ch : '?';         // keep every byte 7-bit (non-ASCII is escaped below)
    }
    if (tr == T_USB) { frame[n++] = 0xF7; usbSendSysEx(frame, n); }
    else { udp.beginPacket(udpPeer, udpPeerPort); udp.write(frame, n); udp.endPacket(); }
  }
}

// JSON for SysEx must be 7-bit: escape UTF-8 as \uXXXX.
String asciiJson(const String& in) {
  String out;
  out.reserve(in.length() + 8);
  for (unsigned i = 0; i < in.length();) {
    uint8_t c = (uint8_t) in[i];
    if (c < 0x80) { out += (char) c; i++; continue; }
    uint32_t cp = '?'; int n = 1;
    if ((c & 0xE0) == 0xC0 && i + 1 < in.length()) { cp = ((c & 0x1F) << 6) | (in[i + 1] & 0x3F); n = 2; }
    else if ((c & 0xF0) == 0xE0 && i + 2 < in.length()) { cp = ((c & 0x0F) << 12) | ((in[i + 1] & 0x3F) << 6) | (in[i + 2] & 0x3F); n = 3; }
    else if ((c & 0xF8) == 0xF0) { n = 4; }
    char buf[8];
    snprintf(buf, sizeof(buf), "\\u%04X", (unsigned) (cp > 0xFFFF ? '?' : cp));
    out += buf;
    i += n;
  }
  return out;
}

void sendDoc(Transport tr, JsonDocument& doc) {
  String s;
  serializeJson(doc, s);
  sendTextOn(tr, asciiJson(s));
}

void sendToHost(JsonDocument& doc) {
  if (linked()) sendDoc(active, doc);
}

void sendError(Transport tr, const String& what) {
  JsonDocument d; d["t"] = "err"; d["msg"] = what;
  sendDoc(tr, d);
}

// ── Config → runtime ────────────────────────────────────────────────────────
JsonObject objectAt(JsonObject parent, const char* key) {
  return parent[key].is<JsonObject>() ? parent[key].as<JsonObject>() : parent[key].to<JsonObject>();
}

void defaultControl(JsonObject c, const Board& b, int code, int position) {
  static const char* palette[4] = { "#E0A82E", "#4CAF50", "#2196F3", "#E91E63" };
  c["color"] = palette[position & 3];
  c["led"] = "auto";
  JsonArray layers = c["layers"].to<JsonArray>();
  for (int l = 0; l < MAX_LAYERS; l++) {
    JsonObject a = layers.add<JsonObject>();
    a["mode"] = "pequod";
    a["num"] = code == C_PEDAL ? 11 : 20 + l * 8 + position;
    a["ch"] = 1;
    a["toggle"] = true;
  }
}

String defaultBoardName(const Board& b, int n) {
  switch (b.kind) {
    case KIND_PEDAL: case KIND_PEDAL_SWITCH: return "Pedal " + String(n);
    case KIND_ENCODER: return "Knob " + String(n);
    default: return "Switches " + String(n);
  }
}

// Every connected board gets a settings entry with all of its controls.
void ensureBoardEntries() {
  JsonObject units = objectAt(cfg.as<JsonObject>(), "units");
  int counter[8] = { 0 };
  for (int i = 0; i < numBoards; i++) {
    Board& b = boards[i];
    JsonObject u = objectAt(units, b.key);
    counter[b.kind]++;
    if (!u["name"].is<const char*>()) u["name"] = defaultBoardName(b, counter[b.kind]);
    JsonObject ctrls = objectAt(u, "controls");
    int codes[MAX_CTRLS];
    int n = controlCodes(b, codes);
    for (int k = 0; k < n; k++) {
      String key = String(codes[k]);
      if (!ctrls[key].is<JsonObject>()) defaultControl(ctrls[key].to<JsonObject>(), b, codes[k], k);
      JsonObject c = ctrls[key];
      JsonArray layers = c["layers"].is<JsonArray>() ? c["layers"].as<JsonArray>() : c["layers"].to<JsonArray>();
      while (layers.size() < MAX_LAYERS) {
        JsonObject a = layers.add<JsonObject>();
        a["mode"] = "pequod"; a["num"] = 20; a["ch"] = 1; a["toggle"] = true;
      }
    }
    if ((b.kind == KIND_PEDAL || b.kind == KIND_PEDAL_SWITCH) && !u["pedal"].is<JsonObject>()) {
      JsonObject p = u["pedal"].to<JsonObject>();
      p["min"] = 0; p["max"] = 4095; p["invert"] = false;
    }
    if (b.kind == KIND_ENCODER && !u["encoder"].is<JsonObject>()) u["encoder"].to<JsonObject>()["invert"] = false;
  }
}

void assignSlots() {
  JsonObject units = cfg["units"].as<JsonObject>();
  bool used[MAX_SLOTS] = { false };
  for (int i = 0; i < numBoards; i++) boards[i].slot = -1;

  // Keep each board's remembered position when it's free...
  for (int i = 0; i < numBoards; i++) {
    int s = units[boards[i].key]["slot"] | -1;
    if (s >= 0 && s < MAX_SLOTS && !used[s]) { boards[i].slot = s; used[s] = true; }
  }
  // ...and give new boards the first free one.
  for (int i = 0; i < numBoards; i++) {
    if (boards[i].slot < 0)
      for (int s = 0; s < MAX_SLOTS; s++)
        if (!used[s]) { boards[i].slot = s; used[s] = true; break; }
    units[boards[i].key]["slot"] = boards[i].slot;
  }
  // Poll and report boards in slot order.
  for (int i = 1; i < numBoards; i++)
    for (int j = i; j > 0 && boards[j - 1].slot > boards[j].slot; j--) {
      Board t = boards[j]; boards[j] = boards[j - 1]; boards[j - 1] = t;
    }
}

void applyConfig() {
  ensureBoardEntries();
  assignSlots();

  brightness = constrain((int) (cfg["brightness"] | 200), 5, 255);
  offGlow = constrain((int) (cfg["offGlow"] | 10), 0, 100);
  numLayers = constrain((int) (cfg["layers"] | 1), 1, MAX_LAYERS);
  if (layer >= numLayers) layer = 0;
  JsonArray lc = cfg["layerColors"].as<JsonArray>();
  for (int l = 0; l < MAX_LAYERS; l++)
    if (!lc.isNull() && l < (int) lc.size()) layerColors[l] = parseColor(lc[l] | "", layerColors[l]);
  midiUsb = cfg["midi"]["usb"] | true;
  midiDin = cfg["midi"]["din"] | true;

  JsonObject units = cfg["units"].as<JsonObject>();
  for (int i = 0; i < numBoards; i++) {
    Board& b = boards[i];
    JsonObject u = units[b.key];
    int codes[MAX_CTRLS];
    b.numCtrls = controlCodes(b, codes);
    for (int k = 0; k < b.numCtrls; k++) {
      Control& ctl = b.ctl[k];
      ctl.code = codes[k];
      JsonObject jc = u["controls"][String(codes[k])];
      const char* col = jc["color"] | "#E0A82E";
      ctl.colorIsLayer = !strcmp(col, "layer");
      ctl.color = parseColor(col, 0xE0A82E);
      ctl.led = ledFromName(jc["led"] | "auto");
      JsonArray layers = jc["layers"];
      for (int l = 0; l < MAX_LAYERS; l++) {
        JsonObject a = layers[l];
        Action& act = ctl.act[l];
        act = Action();
        act.toggle = a["toggle"] | true;
        if (a["midi"].is<JsonArray>()) {                   // PEQUOD 2.1+: everything this control does
          for (JsonVariant mv : a["midi"].as<JsonArray>()) {
            if (act.nMidi >= 4) break;
            MidiMsg& mm = act.midi[act.nMidi];
            mm.type = modeFromName(mv["type"] | "cc");
            if (mm.type != M_CC && mm.type != M_NOTE && mm.type != M_PC && mm.type != M_CC_REL && mm.type != M_CC_ABS) continue;
            mm.num = constrain((int) (mv["num"] | 20), 0, 127);
            mm.ch = constrain((int) (mv["ch"] | 1), 1, 16);
            act.nMidi++;
          }
          const char* lt = a["layer"]["type"] | "none";
          act.layerMode = !strcmp(lt, "next") ? M_LAYER_NEXT : !strcmp(lt, "prev") ? M_LAYER_PREV
                        : !strcmp(lt, "set") ? M_LAYER_SET : !strcmp(lt, "hold") ? M_LAYER_HOLD
                        : !strcmp(lt, "turn") ? M_LAYER_DIR : M_NONE;
          act.layerNum = constrain((int) (a["layer"]["num"] | 1), 1, MAX_LAYERS);
        } else {                                           // older settings: one thing per control
          uint8_t mode = modeFromName(a["mode"] | "pequod");
          int num = constrain((int) (a["num"] | 20), 0, 127), ch = constrain((int) (a["ch"] | 1), 1, 16);
          if (mode == M_CC || mode == M_NOTE || mode == M_PC || mode == M_CC_REL || mode == M_CC_ABS) {
            act.midi[0].type = mode; act.midi[0].num = num; act.midi[0].ch = ch; act.nMidi = 1;
          } else if (mode >= M_LAYER_NEXT && mode <= M_LAYER_HOLD) {
            act.layerMode = mode; act.layerNum = constrain(num, 1, MAX_LAYERS);
          } else if (mode == M_LAYER_DIR) {
            act.layerMode = M_LAYER_DIR;
          }
        }
      }
    }
    if (b.kind == KIND_PEDAL || b.kind == KIND_PEDAL_SWITCH) {
      b.pedalMin = u["pedal"]["min"] | 0;
      b.pedalMax = u["pedal"]["max"] | 4095;
      if (b.pedalMax <= b.pedalMin + 10) b.pedalMax = b.pedalMin + 10;
      b.pedalInvert = u["pedal"]["invert"] | false;
    }
    if (b.kind == KIND_ENCODER) b.encInvert = u["encoder"]["invert"] | false;
  }
  displayDirty = true;
}

void markConfigDirty() {
  cfgDirty = true;
  cfgDirtySince = millis();
}

void saveConfig() {
  File f = SPIFFS.open("/pqfc.json", "w");
  if (!f) { Serial.println("[CFG] save failed"); return; }
  serializeJson(cfg, f);
  f.close();
  cfgDirty = false;
  Serial.println("[CFG] saved");
}

// ── Defaults, and import from the previous master's files (first boot only) ─
bool readJsonFile(const char* path, JsonDocument& doc) {
  if (!SPIFFS.exists(path)) return false;
  File f = SPIFFS.open(path, "r");
  if (!f) return false;
  bool ok = !deserializeJson(doc, f);
  f.close();
  return ok;
}

void defaultSettings() {
  cfg.clear();
  cfg["name"] = "Longsleeves";
  cfg["brightness"] = 200;
  cfg["offGlow"] = 10;
  cfg["layers"] = 1;
  JsonArray colors = cfg["layerColors"].to<JsonArray>();
  for (int l = 0; l < MAX_LAYERS; l++) colors.add(colorToHex(layerColors[l]));
  cfg["midi"]["usb"] = true;
  cfg["midi"]["din"] = true;
  cfg["wifi"]["enabled"] = true;
  cfg["wifi"]["ssid"] = "";
  cfg["wifi"]["pass"] = "";
  cfg["wifi"]["ssid2"] = "";
  cfg["wifi"]["pass2"] = "";
  cfg["wifi"]["hostname"] = "pequod-fc";
  cfg["display"]["contrast"] = 200;
  cfg["units"].to<JsonObject>();
}

void importOldSettings() {
  Serial.println("[CFG] first boot: importing the previous firmware's settings");

  JsonDocument w;
  if (readJsonFile("/wifi.json", w)) {
    cfg["wifi"]["ssid"] = w["primarySSID"] | "";
    cfg["wifi"]["pass"] = w["primaryPass"] | "";
    cfg["wifi"]["ssid2"] = w["fallbackSSID"] | "";
    cfg["wifi"]["pass2"] = w["fallbackPass"] | "";
    String host = w["hostname"] | "";
    if (host.length()) cfg["wifi"]["hostname"] = host;
  }

  JsonDocument ui;
  if (readJsonFile("/ui.json", ui)) {
    int m = ui["midiOutputMode"] | 2;           // 0 DIN only, 1 USB only, 2 both
    cfg["midi"]["usb"] = (m != 0);
    cfg["midi"]["din"] = (m != 1);
  }

  JsonDocument lc;
  if (readJsonFile("/layer_colors.json", lc))
    for (int l = 0; l < MAX_LAYERS; l++) {
      String k = "layer" + String(l);
      if (lc[k].is<const char*>()) cfg["layerColors"][l] = lc[k].as<const char*>();
    }

  Preferences prefs;
  if (prefs.begin("ui", true)) {
    cfg["layers"] = constrain(prefs.getInt("numLayers", 1), 1, MAX_LAYERS);
    cfg["brightness"] = constrain(prefs.getInt("ledBright", 200), 5, 255);
    prefs.end();
  }

  // Per board: position, colours, pedal calibration and MIDI numbers.
  // Controls start in PEQUOD mode; their old MIDI numbers are kept for when you
  // switch a control to "MIDI to other gear".
  JsonObject units = cfg["units"].as<JsonObject>();
  for (int i = 0; i < numBoards; i++) {
    Board& b = boards[i];
    JsonDocument od;
    String path = String("/device_") + b.key + ".json";
    if (!readJsonFile(path.c_str(), od)) continue;

    JsonObject u = objectAt(units, b.key);
    u["slot"] = od["preferredSlot"] | -1;
    JsonObject ctrls = objectAt(u, "controls");
    int codes[MAX_CTRLS];
    int n = controlCodes(b, codes);
    for (int k = 0; k < n; k++) {
      JsonObject jc = ctrls[String(codes[k])].to<JsonObject>();
      defaultControl(jc, b, codes[k], k);
      // old firmware: switch buttons at index 0-3; pedal at 0; a pedal's switch at 1
      int oldBtn = (codes[k] == C_PEDAL) ? 0 : (b.kind == KIND_PEDAL_SWITCH ? 1 : codes[k]);
      if (codes[k] >= 0 && codes[k] < 4 && od["leds"][oldBtn]["color"].is<const char*>())
        jc["color"] = od["leds"][oldBtn]["color"].as<const char*>();
      for (int l = 0; l < MAX_LAYERS; l++) {
        JsonObject ob = od["layers"][l]["buttons"][oldBtn];
        if (ob.isNull()) continue;
        JsonObject a = jc["layers"][l];
        a["num"] = constrain((int) (ob["mapping"] | a["num"].as<int>()), 0, 127);
        a["ch"] = constrain((int) (ob["channel"] | 1), 1, 16);
        a["toggle"] = strcmp(ob["behavior"] | "momentary", "toggle") == 0;
      }
    }
    if (b.kind == KIND_PEDAL || b.kind == KIND_PEDAL_SWITCH) {
      u["pedal"]["min"] = od["pedal"]["calMin"] | 0;
      u["pedal"]["max"] = od["pedal"]["calMax"] | 4095;
      u["pedal"]["invert"] = od["pedal"]["invert"] | false;
    }
    if (b.kind == KIND_ENCODER) u["encoder"]["invert"] = od["encoder"]["invertDirection"] | false;
  }
}

void loadConfig() {
  JsonDocument d;
  if (readJsonFile("/pqfc.json", d)) {
    cfg = d;
    return;
  }
  defaultSettings();
  if (SPIFFS.exists("/wifi.json") || SPIFFS.exists("/ui.json") || SPIFFS.exists("/layer_colors.json"))
    importOldSettings();
  markConfigDirty();
}

// ── I2C slave protocol (unchanged from the previous master) ─────────────────
bool slaveId(uint8_t addr, uint64_t& id) {
  Wire.beginTransmission(addr);
  Wire.write(0x00);
  if (Wire.endTransmission() != 0) return false;
  Wire.requestFrom(addr, (uint8_t) 8);
  if (Wire.available() < 8) return false;
  id = 0;
  for (int i = 0; i < 8; i++) { id <<= 8; id |= Wire.read(); }
  return true;
}

bool slaveType(uint8_t addr, uint8_t& type) {
  Wire.beginTransmission(addr);
  Wire.write(0x01);
  if (Wire.endTransmission() != 0) return false;
  Wire.requestFrom(addr, (uint8_t) 1);
  if (Wire.available() < 1) return false;
  type = Wire.read();
  return true;
}

void writeRgb(uint32_t c) {
  Wire.write((uint8_t) ((c >> 16) & 0xFF));
  Wire.write((uint8_t) ((c >> 8) & 0xFF));
  Wire.write((uint8_t) (c & 0xFF));
}

void slaveSetAllLeds(uint8_t addr, const uint32_t leds[5]) {
  Wire.beginTransmission(addr);
  Wire.write(0x21);
  for (int i = 0; i < 5; i++) writeRgb(scaleColor(leds[i], brightness));
  Wire.endTransmission();
}

void slaveSetLayerColor(uint8_t addr, uint32_t color) {
  Wire.beginTransmission(addr);
  Wire.write(0x22);
  writeRgb(scaleColor(trueColor(color), brightness));
  Wire.endTransmission();
}

void scanBoards() {
  numBoards = 0;
  for (uint8_t addr = 1; addr < 0x78 && numBoards < MAX_BOARDS; addr++) {
    uint64_t id; uint8_t type;
    if (slaveId(addr, id) && slaveType(addr, type)) {
      Kind k = kindForType(type);
      if (k == KIND_NONE) continue;          // another master or an unknown device
      Board& b = boards[numBoards];
      b = Board();
      b.addr = addr; b.id = id; b.type = type; b.kind = k;
      snprintf(b.key, sizeof(b.key), "%08lX", (unsigned long) (id & 0xFFFFFFFFULL));
      int codes[MAX_CTRLS];
      b.numCtrls = controlCodes(b, codes);
      for (int c = 0; c < b.numCtrls; c++) b.ctl[c].code = codes[c];
      numBoards++;
      Serial.printf("[I2C] 0x%02X  id %s  type %d (%s)\n", addr, b.key, type, kindName(k));
    }
    delay(5);
  }
  Serial.printf("[I2C] %d board(s)\n", numBoards);
}

// ── Layers ──────────────────────────────────────────────────────────────────
void sendLayer() {
  JsonDocument d;
  d["t"] = "layer"; d["l"] = layer;
  sendToHost(d);
}

void setLayer(int l) {
  l = constrain(l, 0, numLayers - 1);
  if (l == layer) return;
  layer = l;
  for (int i = 0; i < numBoards; i++)
    if (boards[i].kind == KIND_FOOTSWITCH) slaveSetLayerColor(boards[i].addr, layerColors[layer]);
  sendLayer();
  displayDirty = true;
}

// ── Control handling ────────────────────────────────────────────────────────
void sendEvent(Board& b, int code, int lay, float v, float f, bool withF) {
  JsonDocument d;
  d["t"] = "ev"; d["u"] = b.key; d["c"] = code; d["l"] = lay; d["v"] = v;
  if (withF) d["f"] = f;
  sendToHost(d);
}

void onSwitch(Board& b, Control& ctl, bool pressed) {
  ctl.down = pressed;
  if (pressed) ctl.pressLayer = layer;
  const int lay = ctl.pressLayer;          // a release belongs to the layer it was pressed on
  const Action& a = ctl.act[lay];

  { // every press, whatever its mode: lets PEQUOD's window follow your feet
    JsonDocument d;
    d["t"] = "press"; d["u"] = b.key; d["c"] = ctl.code; d["v"] = pressed ? 1 : 0;
    sendToHost(d);
  }

  if (linked()) sendEvent(b, ctl.code, lay, pressed ? 1 : 0, 0, false);   // PEQUOD does whatever it's set to

  // MIDI to other gear: latching flips once per press, for all of this control's messages
  bool on = pressed;
  if (a.toggle && a.nMidi > 0) {
    if (pressed) ctl.toggled[lay] = !ctl.toggled[lay];
    on = ctl.toggled[lay];
  }
  for (int i = 0; i < a.nMidi; i++) {
    const MidiMsg& mm = a.midi[i];
    if (mm.type == M_PC) { if (pressed) midiPC(mm.num, mm.ch); continue; }
    if (a.toggle && !pressed) continue;                     // latching: only presses change anything
    if (mm.type == M_CC)   midiCC(mm.num, on ? 127 : 0, mm.ch);
    if (mm.type == M_NOTE) midiNote(mm.num, on, mm.ch);
  }

  switch (a.layerMode) {
    case M_LAYER_NEXT: if (pressed) setLayer((layer + 1) % numLayers); break;
    case M_LAYER_PREV: if (pressed) setLayer((layer + numLayers - 1) % numLayers); break;
    case M_LAYER_SET:  if (pressed) setLayer((int) a.layerNum - 1); break;
    case M_LAYER_HOLD:
      if (pressed) { holdReturnLayer = layer; setLayer((int) a.layerNum - 1); }
      else if (holdReturnLayer >= 0) { setLayer(holdReturnLayer); holdReturnLayer = -1; }
      break;
    default: break;
  }
}

void onPedal(Board& b, int raw) {
  b.rawPedal = raw;
  Control* ctl = findControl(b, C_PEDAL);
  if (!ctl) return;
  int lo = b.pedalMin, hi = b.pedalMax;
  int v10 = map(constrain(raw, lo, hi), lo, hi, 0, 1023);
  if (b.pedalInvert) v10 = 1023 - v10;
  if (v10 == b.lastPedal10) return;
  if (abs(v10 - b.lastPedal10) < 4 && v10 != 0 && v10 != 1023) return;   // ignore jitter, always reach the ends
  b.lastPedal10 = v10;
  int v7 = (v10 * 127 + 511) / 1023;
  const Action& a = ctl->act[layer];
  if (linked()) sendEvent(b, C_PEDAL, layer, v7, v10 / 1023.0f, true);
  if (v7 != b.lastPedal7)
    for (int i = 0; i < a.nMidi; i++)
      if (a.midi[i].type == M_CC) midiCC(a.midi[i].num, v7, a.midi[i].ch);
  b.lastPedal7 = v7;
}

void onEncoder(Board& b, int steps) {
  if (b.encInvert) steps = -steps;
  Control* ctl = findControl(b, C_ENC_TURN);
  if (!ctl) return;
  Action& a = ctl->act[layer];
  if (linked()) sendEvent(b, C_ENC_TURN, layer, steps, 0, false);
  for (int i = 0; i < a.nMidi; i++) {
    MidiMsg& mm = a.midi[i];
    switch (mm.type) {
      case M_CC_REL: case M_CC: midiCC(mm.num, constrain(64 + steps, 1, 127), mm.ch); break;
      case M_CC_ABS:
        mm.absValue = constrain((int) mm.absValue + steps, 0, 127);
        midiCC(mm.num, mm.absValue, mm.ch);
        break;
      case M_PC: {
        int p = ((lastPcNum == 255 ? 0 : lastPcNum) + steps) % 128;
        if (p < 0) p += 128;
        midiPC(p, mm.ch);
        break;
      }
      default: break;
    }
  }
  if (a.layerMode == M_LAYER_DIR) setLayer(((layer + (steps > 0 ? 1 : -1)) % numLayers + numLayers) % numLayers);
}

// ── LEDs ────────────────────────────────────────────────────────────────────
uint32_t controlColor(Board& b, Control& ctl) {
  const Action& a = ctl.act[layer];
  uint32_t col = ctl.colorIsLayer ? layerColors[layer] : ctl.color;
  bool on = false;

  switch (ctl.led) {
    case LED_ON:   on = true; break;
    case LED_OFF:  on = false; break;
    case LED_MIDI: on = ctl.midiOn; break;
    default:
      // lit when PEQUOD says so, while held, when its latched MIDI is on, or when its layer is active
      on = ctl.down || (linked() && ctl.pqOn);
      if (linked() && ctl.pqHasColor) col = ctl.pqColor;
      if (a.nMidi > 0 && a.toggle && ctl.toggled[layer]) on = true;
      for (int i = 0; i < a.nMidi; i++)
        if (a.midi[i].type == M_PC && lastPcCh == a.midi[i].ch && lastPcNum == a.midi[i].num) on = true;
      if ((a.layerMode == M_LAYER_SET || a.layerMode == M_LAYER_HOLD) && layer == (int) a.layerNum - 1) on = true;
  }

  if (millis() < b.identifyUntil && (b.identifyCode < 0 || b.identifyCode == ctl.code))
    return ((millis() / 150) % 2) == 0 ? 0xFFFFFF : 0;    // "which one is this?" flash

  if (ctl.led == LED_OFF) return 0;
  col = trueColor(col);
  if (!on) return offGlow == 0 ? 0 : scaleColor(col, (uint8_t) (offGlow * 255 / 100));
  return col;
}

void boardLeds(Board& b, uint32_t leds[5]) {
  for (int i = 0; i < 5; i++) leds[i] = 0;
  for (int k = 0; k < b.numCtrls; k++) {
    int idx = ledIndex(b, b.ctl[k].code);
    if (idx > 0) leds[idx] = controlColor(b, b.ctl[k]);
  }
}

void setOnboardLed(uint32_t c) {
  c = scaleColor(c, 40);
#if defined(ESP_ARDUINO_VERSION) && ESP_ARDUINO_VERSION >= ESP_ARDUINO_VERSION_VAL(3, 1, 0)
  rgbLedWrite(RGB_PIN, (c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
#else
  neopixelWrite(RGB_PIN, (c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF);
#endif
}

// ── Polling (same command order as the previous master) ─────────────────────
void pollBoards() {
  for (int i = 0; i < numBoards; i++) {
    Board& b = boards[i];
    if (millis() < b.failUntil) continue;

    if (hasSwitchRow(b)) {
      uint32_t leds[5];
      boardLeds(b, leds);
      slaveSetAllLeds(b.addr, leds);                   // LEDs first, then read (as before)

      Wire.beginTransmission(b.addr);
      Wire.write(0x02);
      if (Wire.endTransmission() != 0) { b.failUntil = millis() + 50; continue; }
      Wire.requestFrom(b.addr, (uint8_t) 1);
      if (Wire.available() < 1) continue;
      uint8_t states = Wire.read();
      for (int c = 0; c < 4; c++) {
        bool now = states & (1 << c), before = b.lastButtons & (1 << c);
        if (now != before) if (Control* ctl = findControl(b, c)) onSwitch(b, *ctl, now);
      }
      b.lastButtons = states;
    }
    else if (b.kind == KIND_PEDAL || b.kind == KIND_PEDAL_SWITCH) {
      bool withSwitch = (b.kind == KIND_PEDAL_SWITCH);
      if (withSwitch) {
        uint32_t leds[5];
        boardLeds(b, leds);
        slaveSetAllLeds(b.addr, leds);
      }
      Wire.beginTransmission(b.addr);
      Wire.write(0x02);
      if (Wire.endTransmission() != 0) { b.failUntil = millis() + 50; continue; }
      Wire.requestFrom(b.addr, (uint8_t) (withSwitch ? 3 : 2));
      if (Wire.available() < 2) continue;
      int raw = Wire.read() << 8;
      raw |= Wire.read();
      onPedal(b, raw);
      if (withSwitch && Wire.available() >= 1) {
        uint8_t btn = Wire.read();
        bool now = btn & 0x01, before = b.lastPedalBtn & 0x01;
        if (now != before) if (Control* ctl = findControl(b, 0)) onSwitch(b, *ctl, now);
        b.lastPedalBtn = btn;
      }
    }
    else if (b.kind == KIND_ENCODER) {
      Wire.beginTransmission(b.addr);
      Wire.write(0x04);
      if (Wire.endTransmission() != 0) { b.failUntil = millis() + 50; continue; }
      Wire.requestFrom(b.addr, (uint8_t) 3);
      if (Wire.available() < 3) continue;
      uint8_t lo = Wire.read(), hi = Wire.read(), btn = Wire.read();
      int16_t pos = (int16_t) ((hi << 8) | lo);
      if (!b.encPosValid) { b.lastEncPos = pos; b.encPosValid = true; }
      int delta = (int) pos - (int) b.lastEncPos;
      if (delta > 32767) delta -= 65536;
      if (delta < -32768) delta += 65536;
      if (delta != 0) onEncoder(b, delta);
      b.lastEncPos = pos;
      if (btn & 0x02) if (Control* ctl = findControl(b, C_ENC_PUSH)) onSwitch(b, *ctl, btn & 0x01);
    }
  }
}

// ── Messages to PEQUOD ──────────────────────────────────────────────────────
void fillInfo(JsonDocument& d, Transport tr) {
  d["t"] = "info";
  d["proto"] = PROTO_VERSION;
  d["fw"] = FW_VERSION;
  d["id"] = macId();
  d["name"] = cfg["name"] | "Longsleeves";
  d["via"] = tr == T_USB ? "usb" : "wifi";
  d["layers"] = numLayers;
  d["layer"] = layer;
  d["globalLayers"] = true;
  d["brightness"] = brightness;
  JsonObject w = d["wifi"].to<JsonObject>();
  w["connected"] = WiFi.isConnected();
  w["ap"] = false;
  w["ssid"] = cfg["wifi"]["ssid"] | "";
  w["ssid2"] = cfg["wifi"]["ssid2"] | "";
  w["ip"] = WiFi.isConnected() ? WiFi.localIP().toString() : String();
  w["rssi"] = WiFi.isConnected() ? WiFi.RSSI() : 0;
  const esp_partition_t* next = esp_ota_get_next_update_partition(NULL);
  JsonObject o = d["ota"].to<JsonObject>();
  o["ok"] = next != nullptr;
  o["max"] = next ? (uint32_t) next->size : 0;
  JsonArray units = d["units"].to<JsonArray>();
  for (int i = 0; i < numBoards; i++) {
    JsonObject u = units.add<JsonObject>();
    u["slot"] = boards[i].slot;
    u["type"] = boards[i].type;
    u["kind"] = kindName(boards[i].kind);
    u["name"] = cfg["units"][boards[i].key]["name"] | kindName(boards[i].kind);
    u["uid"] = boards[i].key;
    u["layer"] = layer;
  }
}

void sendInfo(Transport tr) {
  JsonDocument d;
  fillInfo(d, tr);
  sendDoc(tr, d);
}

void sendConfig(Transport tr) {
  JsonDocument d;
  d["t"] = "cfg";
  d["cfg"].set(cfg);
  for (const char* k : { "pass", "pass2" }) {           // passwords never leave the controller
    String has = String("has") + (k[4] ? "Pass2" : "Pass");
    d["cfg"]["wifi"][has] = String(cfg["wifi"][k] | "").length() > 0;
    d["cfg"]["wifi"][k] = nullptr;
  }
  sendDoc(tr, d);
}

// ── Firmware update over the link ───────────────────────────────────────────
void fwAck(Transport tr, int n, bool ok, const String& err) {
  JsonDocument d;
  d["t"] = "fw_ack"; d["n"] = n; d["ok"] = ok;
  if (err.length()) d["err"] = err;
  sendDoc(tr, d);
}

void fwBegin(Transport tr, JsonDocument& in) {
  size_t size = in["size"] | 0;
  const esp_partition_t* next = esp_ota_get_next_update_partition(NULL);
  if (!next) { fwAck(tr, -1, false, "This controller was flashed without an update partition"); return; }
  if (size == 0 || size > next->size) { fwAck(tr, -1, false, "Firmware is too big for this controller"); return; }
  if (fw.running) Update.abort();
  if (!Update.begin(size, U_FLASH)) { fwAck(tr, -1, false, String("Update.begin failed: ") + Update.errorString()); return; }
  if (in["md5"].is<const char*>()) Update.setMD5(in["md5"].as<const char*>());
  fw.running = true; fw.lastN = -1; fw.written = 0; fw.size = size;
  screen.valid = true; screen.mode = "text"; screen.top = "UPDATING"; screen.main = "0%"; screen.sub = "keep plugged in";
  displayDirty = true;
  fwAck(tr, -1, true, "");
}

void fwData(Transport tr, JsonDocument& in) {
  int n = in["n"] | -1;
  if (!fw.running) { fwAck(tr, n, false, "No update in progress"); return; }
  if (n == fw.lastN) { fwAck(tr, n, true, ""); return; }          // a resend: already written
  if (n != fw.lastN + 1) { fwAck(tr, n, false, "Out of order"); return; }
  const char* b64 = in["d"] | "";
  static uint8_t bin[1024];
  size_t outLen = 0;
  if (mbedtls_base64_decode(bin, sizeof(bin), &outLen, (const unsigned char*) b64, strlen(b64)) != 0) { fwAck(tr, n, false, "Bad data"); return; }
  if (Update.write(bin, outLen) != outLen) { fwAck(tr, n, false, String("Write failed: ") + Update.errorString()); return; }
  fw.lastN = n;
  fw.written += outLen;
  String pct = String((int) (fw.written * 100 / max((size_t) 1, fw.size))) + "%";
  if (pct != screen.main) { screen.main = pct; displayDirty = true; }
  fwAck(tr, n, true, "");
}

void fwEnd(Transport tr) {
  if (!fw.running) { fwAck(tr, -2, false, "No update in progress"); return; }
  fw.running = false;
  bool ok = Update.end(true);
  JsonDocument d;
  d["t"] = "fw_done"; d["ok"] = ok;
  if (!ok) d["err"] = Update.errorString();
  sendDoc(tr, d);
  if (ok) {
    if (cfgDirty) saveConfig();
    if (displayOk) { display.clearDisplay(); display.setTextSize(1); display.setCursor(0, 28); display.print(" UPDATED - RESTARTING"); display.display(); }
    delay(400);
    ESP.restart();
  }
}

// ── Messages from PEQUOD ────────────────────────────────────────────────────
void startWifi();

void applyContrast() {
  if (!displayOk) return;
  display.ssd1306_command(SSD1306_SETCONTRAST);
  display.ssd1306_command((uint8_t) (cfg["display"]["contrast"] | 200));
}

void handleMessage(Transport tr, JsonDocument& m) {
  const String t = m["t"] | "";
  lastRx[tr] = millis();
  if (t != "hello") {
    if (active != tr && active != T_NONE && linked() && tr == T_UDP) return;   // USB already in use: ignore Wi-Fi
    active = tr;                                                            // PEQUOD chose this route
  }

  if (t == "hello")         { sendInfo(tr); return; }
  if (t == "ping")          { JsonDocument d; d["t"] = "pong"; sendDoc(tr, d); return; }
  if (t == "bye")           { if (active == tr) { lastRx[tr] = 0; } return; }
  if (t == "get_cfg")       { sendConfig(tr); return; }
  if (t == "rescan")        { scanBoards(); applyConfig(); markConfigDirty(); sendInfo(tr); sendConfig(tr); return; }

  if (t == "set_cfg") {
    String oldWifi;
    serializeJson(cfg["wifi"], oldWifi);
    String oldName = cfg["name"] | "";
    JsonDocument incoming;
    incoming.set(m["cfg"]);
    for (const char* k : { "pass", "pass2" })            // null = keep the stored password
      if (incoming["wifi"][k].isNull()) incoming["wifi"][k] = cfg["wifi"][k] | "";
    incoming["wifi"].remove("hasPass");
    incoming["wifi"].remove("hasPass2");
    cfg = incoming;
    applyConfig();
    // "temp": PEQUOD changed preset and sent that preset's switch settings - use them, don't wear the flash
    if (!(m["temp"] | false)) markConfigDirty();
    applyContrast();
    String newWifi;
    serializeJson(cfg["wifi"], newWifi);
    if (newWifi != oldWifi) startWifi();
    JsonDocument ok; ok["t"] = "cfg_ok"; ok["restart"] = (oldName != String(cfg["name"] | "")); sendDoc(tr, ok);
    if (oldName != String(cfg["name"] | "")) sendInfo(tr);
    return;
  }

  if (t == "leds") {
    for (int i = 0; i < numBoards; i++)
      for (int c = 0; c < MAX_CTRLS; c++) { boards[i].ctl[c].pqOn = false; boards[i].ctl[c].pqHasColor = false; }
    for (JsonVariant ev : m["l"].as<JsonArray>()) {
      JsonArray e = ev.as<JsonArray>();
      Board* b = boardByKey(e[0] | "");
      if (!b) continue;
      Control* ctl = findControl(*b, e[1] | -1);
      if (!ctl) continue;
      ctl->pqOn = (e[2] | 0) != 0;
      if (e[3].is<const char*>()) { ctl->pqHasColor = true; ctl->pqColor = parseColor(e[3].as<const char*>(), ctl->color); }
    }
    return;
  }

  if (t == "screen") {
    screen.valid = true;
    screen.mode = m["mode"] | "preset";
    screen.top = m["top"] | "";
    screen.main = m["main"] | "";
    screen.sub = m["sub"] | "";
    screen.note = m["note"] | "";
    screen.cents = m["cents"] | 0.0f;
    screen.inTune = m["inTune"] | false;
    if (m["flash"].is<const char*>()) { screen.flash = m["flash"].as<const char*>(); screen.flashUntil = millis() + 1200; }
    if (screen.mode == "layout") {
      // items: [["t",x,y,size,align,inv,"text"], ["b",x,y,w,h], ["r",x,y,w,h], ...]
      numItems = 0;
      for (JsonVariant ev : m["items"].as<JsonArray>()) {
        if (numItems >= MAX_ITEMS) break;
        JsonArray e = ev.as<JsonArray>();
        ScreenItem& it = items[numItems];
        const char* k = e[0] | "t";
        it.kind = k[0];
        it.x = e[1] | 0;
        it.y = e[2] | 0;
        if (it.kind == 't') {
          it.size = constrain((int) (e[3] | 1), 1, 4);
          it.align = constrain((int) (e[4] | 0), 0, 2);
          it.inv = (e[5] | 0) != 0;
          it.text = e[6] | "";
        } else {
          it.w = e[3] | 0;
          it.h = e[4] | 0;
        }
        numItems++;
      }
    }
    displayDirty = true;
    return;
  }

  if (t == "layer")         { setLayer(m["l"] | 0); return; }
  if (t == "identify")      {
    if (Board* b = boardByKey(m["u"] | "")) { b->identifyUntil = millis() + 2500; b->identifyCode = m["c"] | -1; }
    return;
  }
  if (t == "cal")           { if (Board* b = boardByKey(m["u"] | "")) b->calWatch = m["on"] | false; return; }
  if (t == "wifiscan")      { WiFi.scanNetworks(true); wifiScanPending = true; return; }
  if (t == "fw_begin")      { fwBegin(tr, m); return; }
  if (t == "fw_data")       { fwData(tr, m); return; }
  if (t == "fw_end")        { fwEnd(tr); return; }
  if (t == "fw_abort")      { if (fw.running) { Update.abort(); fw.running = false; screen.valid = false; displayDirty = true; } return; }
  if (t == "reboot")        { if (cfgDirty) saveConfig(); delay(100); ESP.restart(); }
  if (t == "factory") {
    String name = cfg["name"] | "Longsleeves";
    defaultSettings();
    cfg["name"] = name;
    saveConfig();
    delay(100);
    ESP.restart();
  }
}

void feedFrame(Transport tr, Reassembler& r, const uint8_t* f, size_t n) {
  if (n < 4 || f[0] != 'J') return;
  const int id = f[1], index = f[2], count = f[3];
  if (index == 0) { r.id = id; r.next = 0; r.count = count; r.text = ""; }
  if (id != r.id || index != r.next || count != r.count) { r.id = -1; return; }
  r.text.concat((const char*) (f + 4), n - 4);
  if (++r.next < r.count) return;
  r.id = -1;
  JsonDocument m;
  DeserializationError err = deserializeJson(m, r.text);
  r.text = "";
  if (err) { Serial.println("[LINK] bad JSON"); return; }
  handleMessage(tr, m);
}

// USB-MIDI input: SysEx carries the link; notes/CCs drive LEDs set to follow MIDI.
static uint8_t usbSx[1200];
static size_t usbSxLen = 0;
static bool usbSxActive = false;

void midiInMessage(uint8_t status, uint8_t d1, uint8_t d2) {
  uint8_t type = status & 0xF0, ch = (status & 0x0F) + 1;
  bool isNote = (type == 0x90 || type == 0x80), isCC = (type == 0xB0);
  if (!isNote && !isCC) return;
  bool on = (type == 0x90 && d2 > 0) || (isCC && d2 >= 64);
  for (int i = 0; i < numBoards; i++)
    for (int c = 0; c < boards[i].numCtrls; c++) {
      Control& ctl = boards[i].ctl[c];
      if (ctl.led != LED_MIDI) continue;
      const Action& a = ctl.act[layer];
      for (int k = 0; k < a.nMidi; k++)
        if (a.midi[k].num == d1 && a.midi[k].ch == ch && ((isNote && a.midi[k].type == M_NOTE) || (isCC && a.midi[k].type != M_NOTE && a.midi[k].type != M_PC)))
          ctl.midiOn = on;
    }
}

void usbSysExByte(uint8_t b) {
  if (b == 0xF0) { usbSxActive = true; usbSxLen = 0; return; }
  if (!usbSxActive) return;
  if (b == 0xF7) {
    usbSxActive = false;
    // 7D 'P' 'Q' <frame>
    if (usbSxLen >= 7 && usbSx[0] == 0x7D && usbSx[1] == 'P' && usbSx[2] == 'Q')
      feedFrame(T_USB, rxUsb, usbSx + 3, usbSxLen - 3);
    return;
  }
  if (usbSxLen < sizeof(usbSx)) usbSx[usbSxLen++] = b;
  else usbSxActive = false;
}

void pollUsbMidi() {
  midiEventPacket_t p;
  int budget = 96;
  while (budget-- > 0 && usbMidi.readPacket(&p)) {
    switch (p.header & 0x0F) {
      case 0x4: usbSysExByte(p.byte1); usbSysExByte(p.byte2); usbSysExByte(p.byte3); break;
      case 0x5: usbSysExByte(p.byte1); break;
      case 0x6: usbSysExByte(p.byte1); usbSysExByte(p.byte2); break;
      case 0x7: usbSysExByte(p.byte1); usbSysExByte(p.byte2); usbSysExByte(p.byte3); break;
      case 0x8: case 0x9: case 0xB: midiInMessage(p.byte1, p.byte2, p.byte3); break;
      default: break;
    }
  }
}

void pollDinMidi() {
  static uint8_t status = 0, d1 = 0, count = 0;
  while (Serial1.available()) {
    uint8_t b = Serial1.read();
    if (b >= 0xF8) continue;                      // real-time bytes
    if (b & 0x80) { status = (b < 0xF0) ? b : 0; count = 0; continue; }
    if (!status) continue;
    if (count == 0) { d1 = b; count = 1; }
    else { midiInMessage(status, d1, b); count = 0; }
  }
}

void pollUdp() {
  if (!udpRunning) return;
  static uint8_t buf[1100];
  for (int k = 0; k < 8; k++) {
    if (udp.parsePacket() <= 0) break;
    int n = udp.read(buf, sizeof(buf));
    if (n < 8 || buf[0] != 'P' || buf[1] != 'Q' || buf[2] != 'L' || buf[3] != '1') continue;
    udpPeer = udp.remoteIP();
    udpPeerPort = udp.remotePort();
    feedFrame(T_UDP, rxUdp, buf + 4, n - 4);
  }
}

// ── Wi-Fi ───────────────────────────────────────────────────────────────────
void beginNetwork(const char* ssidKey, const char* passKey) {
  String ssid = cfg["wifi"][ssidKey] | "";
  String host = cfg["wifi"]["hostname"] | "pequod-fc";
  WiFi.mode(WIFI_STA);
  WiFi.setHostname(host.c_str());
  WiFi.setSleep(false);                           // lowest latency
  WiFi.begin(ssid.c_str(), (const char*) (cfg["wifi"][passKey] | ""));
  wifiStartedAt = millis();
}

void startWifi() {
  if (udpRunning) { udp.stop(); udpRunning = false; }
  WiFi.disconnect(true);
  bool enabled = cfg["wifi"]["enabled"] | true;
  String ssid = cfg["wifi"]["ssid"] | "";
  wifiWasConnected = false;
  usingFallbackWifi = false;
  if (!enabled || ssid.length() == 0) { WiFi.mode(WIFI_OFF); return; }
  beginNetwork("ssid", "pass");
}

void loopWifi() {
  bool now = WiFi.isConnected();
  if (now != wifiWasConnected) {
    wifiWasConnected = now;
    if (now) {
      udpRunning = udp.begin(LINK_PORT);
      String host = cfg["wifi"]["hostname"] | "pequod-fc";
      MDNS.end();
      if (MDNS.begin(host.c_str())) MDNS.addService("pequod", "udp", LINK_PORT);
      Serial.printf("[WiFi] %s  %s\n", WiFi.SSID().c_str(), WiFi.localIP().toString().c_str());
    } else if (udpRunning) {
      udp.stop();
      udpRunning = false;
    }
    if (linked()) sendInfo(active);
    displayDirty = true;
  }

  // Can't reach the main network for 20 s: try the backup one (and back again).
  bool enabled = cfg["wifi"]["enabled"] | true;
  if (enabled && !now && wifiStartedAt != 0 && millis() - wifiStartedAt > 20000) {
    String ssid2 = cfg["wifi"]["ssid2"] | "";
    if (ssid2.length()) {
      usingFallbackWifi = !usingFallbackWifi;
      WiFi.disconnect();
      if (usingFallbackWifi) beginNetwork("ssid2", "pass2"); else beginNetwork("ssid", "pass");
    } else {
      wifiStartedAt = millis();
    }
  }

  if (wifiScanPending) {
    int n = WiFi.scanComplete();
    if (n >= 0 || n == WIFI_SCAN_FAILED) {
      wifiScanPending = false;
      JsonDocument d;
      d["t"] = "wifilist";
      JsonArray nets = d["nets"].to<JsonArray>();
      for (int i = 0; i < n && i < 24; i++) {
        JsonObject o = nets.add<JsonObject>();
        o["ssid"] = WiFi.SSID(i);
        o["rssi"] = WiFi.RSSI(i);
        o["lock"] = WiFi.encryptionType(i) != WIFI_AUTH_OPEN;
      }
      WiFi.scanDelete();
      sendToHost(d);
    }
  }
}

// ── Display ─────────────────────────────────────────────────────────────────
void drawCentered(const String& s, int y, int size) {
  display.setTextSize(size);
  int w = s.length() * 6 * size;
  display.setCursor(max(0, (128 - w) / 2), y);
  display.print(s);
}

void drawLayout() {
  display.setTextWrap(false);
  for (int i = 0; i < numItems; i++) {
    const ScreenItem& it = items[i];
    if (it.kind == 'b') { display.fillRect(it.x, it.y, it.w, it.h, SSD1306_WHITE); continue; }
    if (it.kind == 'r') { display.drawRect(it.x, it.y, it.w, it.h, SSD1306_WHITE); continue; }
    const int w = it.text.length() * 6 * it.size, h = 8 * it.size;
    int x = it.align == 1 ? it.x - w / 2 : it.align == 2 ? it.x - w : it.x;
    if (it.inv) {
      display.fillRect(x - 2, it.y - 1, w + 3, h + 1, SSD1306_WHITE);
      display.setTextColor(SSD1306_BLACK);
    }
    display.setTextSize(it.size);
    display.setCursor(x, it.y);
    display.print(it.text);
    display.setTextColor(SSD1306_WHITE);
  }
  display.setTextWrap(true);
}

void drawHostScreen() {
  if (screen.mode == "layout") {
    drawLayout();
  } else if (screen.mode == "tuner") {
    drawCentered(screen.note.length() ? screen.note : String("-"), 6, 4);
    const int y = 50, cx = 64;
    display.drawFastHLine(8, y, 112, SSD1306_WHITE);
    for (int c = -50; c <= 50; c += 25)
      display.drawFastVLine(cx + c * 112 / 100, y - (c == 0 ? 6 : 3), c == 0 ? 12 : 6, SSD1306_WHITE);
    if (screen.note.length()) {
      int x = cx + (int) (constrain(screen.cents, -50.0f, 50.0f) * 112 / 100.0f);
      display.fillRect(x - 2, y - 8, 5, 16, SSD1306_WHITE);
    }
    display.setTextSize(1);
    display.setCursor(0, 0);
    display.print(screen.inTune ? "IN TUNE" : screen.top);
  } else {
    drawCentered(screen.top, 0, 1);
    display.drawFastHLine(0, 10, 128, SSD1306_WHITE);
    String m = screen.main;
    if ((int) m.length() <= 10) drawCentered(m, 22, 2);
    else {
      int cut = m.lastIndexOf(' ', 10);
      if (cut <= 0) cut = 10;
      drawCentered(m.substring(0, cut), 14, 2);
      String rest = m.substring(cut);
      rest.trim();
      if (rest.length() > 10) rest = rest.substring(0, 10);
      drawCentered(rest, 32, 2);
    }
    drawCentered(screen.sub, 56, 1);
  }
  if (screen.flash.length() && millis() < screen.flashUntil) {
    display.fillRect(0, 52, 128, 12, SSD1306_WHITE);
    display.setTextColor(SSD1306_BLACK);
    drawCentered(screen.flash, 54, 1);
    display.setTextColor(SSD1306_WHITE);
  }
}

void drawStandalone() {
  drawCentered(String(cfg["name"] | "PEQUOD FC"), 2, 1);
  display.drawFastHLine(0, 12, 128, SSD1306_WHITE);
  drawCentered("PEQUOD", 20, 2);
  display.setTextSize(1);
  drawCentered("not connected", 38, 1);
  display.setCursor(0, 56);
  if (WiFi.isConnected()) display.print(WiFi.localIP().toString());
  else display.print(String(cfg["wifi"]["ssid"] | "").length() ? "Wi-Fi..." : "USB");
  if (numLayers > 1) { display.setCursor(104, 56); display.print("L"); display.print(layer + 1); }
}

void drawDisplay() {
  if (!displayOk) return;
  display.clearDisplay();
  display.setTextColor(SSD1306_WHITE);
  if ((linked() || fw.running) && screen.valid) drawHostScreen();
  else drawStandalone();
  display.display();
}

// ── Setup / loop ────────────────────────────────────────────────────────────
void setup() {
  Serial.begin(115200);
  buildGamma();
  Serial1.begin(31250, SERIAL_8N1, MIDI_RX, MIDI_TX);
  esp_ota_mark_app_valid_cancel_rollback();      // we booted fine: keep this firmware

  if (!SPIFFS.begin(true)) Serial.println("[FS] mount failed");

  Wire.begin(SDA_PIN, SCL_PIN);
  Wire.setClock(100000);                 // the previous master ended up at 100 kHz too
  Wire.setTimeout(100);

  Wire1.begin(OLED_SDA, OLED_SCL, 400000);
  displayOk = display.begin(SSD1306_SWITCHCAPVCC, OLED_ADDR);
  if (displayOk) {
    display.clearDisplay();
    display.setTextColor(SSD1306_WHITE);
    drawCentered("PEQUOD FC", 20, 2);
    drawCentered("v" FW_VERSION, 44, 1);
    display.display();
  }

  scanBoards();
  loadConfig();
  applyConfig();

  // The controller's name is also its USB-MIDI port name on the computer.
  static String usbName;
  usbName = cfg["name"] | "Longsleeves";
  USB.productName(usbName.c_str());
  USB.manufacturerName("PEQUOD");
  usbMidi.begin();
  USB.begin();

  applyContrast();
  for (int i = 0; i < numBoards; i++)
    if (boards[i].kind == KIND_FOOTSWITCH) slaveSetLayerColor(boards[i].addr, layerColors[layer]);

  startWifi();
  if (cfgDirty) saveConfig();
  Serial.println("[PEQUOD FC] ready");
}

void loop() {
  pollUsbMidi();
  pollDinMidi();
  pollUdp();
  loopWifi();

  static unsigned long lastPoll = 0;
  if (millis() - lastPoll >= 10) { lastPoll = millis(); pollBoards(); }

  // Notice PEQUOD arriving and leaving.
  bool link = linked();
  if (link != wasLinked) {
    wasLinked = link;
    if (!link) {
      active = T_NONE;
      screen.valid = false;
      for (int i = 0; i < numBoards; i++) boards[i].calWatch = false;
      if (fw.running) { Update.abort(); fw.running = false; }
      Serial.println("[LINK] PEQUOD disconnected");
    } else {
      sendLayer();
      Serial.println("[LINK] PEQUOD connected");
    }
    displayDirty = true;
  }

  // Raw pedal readings while PEQUOD's calibration screen is open.
  static unsigned long lastRaw = 0;
  if (link && millis() - lastRaw >= 50) {
    lastRaw = millis();
    for (int i = 0; i < numBoards; i++)
      if (boards[i].calWatch) {
        JsonDocument d;
        d["t"] = "raw"; d["u"] = boards[i].key; d["raw"] = boards[i].rawPedal; d["v"] = boards[i].lastPedal7;
        sendToHost(d);
      }
  }

  // Save settings shortly after the last change (spares the flash while sliders move).
  if (cfgDirty && millis() - cfgDirtySince > 1500) saveConfig();

  static unsigned long lastDraw = 0;
  bool flashExpired = screen.flashUntil != 0 && millis() >= screen.flashUntil;
  if ((displayDirty || flashExpired) && millis() - lastDraw >= 40) {
    if (flashExpired) screen.flashUntil = 0;
    lastDraw = millis();
    displayDirty = false;
    drawDisplay();
  }

  static unsigned long lastRgb = 0;
  if (millis() - lastRgb >= 200) {
    lastRgb = millis();
    setOnboardLed(link ? layerColors[layer] : ((millis() / 1000) % 2 ? 0x202020 : 0));
  }
}
