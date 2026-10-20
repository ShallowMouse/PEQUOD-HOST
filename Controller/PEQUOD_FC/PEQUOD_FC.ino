/* ============================================================================
   PEQUOD FC — master firmware for the ESP32-S3 foot controller
   ----------------------------------------------------------------------------
   * Talks to the existing slave boards with exactly the same I2C commands as
     the previous master (0x00 id, 0x01 type, 0x02 read, 0x04 encoder,
     0x20/0x21/0x22 LEDs), so slave firmware is unchanged.
   * Native link to PEQUOD Host over USB (MIDI SysEx) and Wi-Fi (UDP 9124),
     see docs/controller-protocol.md. All settings are edited from PEQUOD.
   * Controls can also send plain MIDI (USB + 5-pin DIN) to other gear.

   Board: ESP32S3 Dev Module, USB Mode: "USB-OTG (TinyUSB)", USB CDC On Boot: Disabled
   (the USB port is then a pure MIDI device named after the controller; debug output
   goes to the UART pins).
   Libraries: ArduinoJson 7, Adafruit SSD1306, Adafruit GFX.
   ============================================================================ */

#include <Arduino.h>
#include <Wire.h>
#include <WiFi.h>
#include <WiFiUdp.h>
#include <ESPmDNS.h>
#include <SPIFFS.h>
#include <Preferences.h>
#include <ArduinoJson.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>
#include "USB.h"
#include "USBMIDI.h"

#define FW_VERSION    "2.0.0"
#define PROTO_VERSION 1

// ── Hardware (same as the previous master) ──────────────────────────────────
static const int SDA_PIN = 8, SCL_PIN = 18;          // slave bus (Wire)
static const int OLED_SDA = 4, OLED_SCL = 5;         // display bus (Wire1)
static const uint8_t OLED_ADDR = 0x3C;
static const int MIDI_TX = 17, MIDI_RX = 16;         // 5-pin DIN (Serial1)
static const int RGB_PIN = 48;                       // onboard RGB LED
static const uint16_t LINK_PORT = 9124;

static const int MAX_BOARDS = 16;
static const int MAX_SLOTS = 8;
static const int MAX_CTRLS = 4;
static const int MAX_LAYERS = 4;

// ── Types ───────────────────────────────────────────────────────────────────
enum Kind : uint8_t { KIND_NONE, KIND_SWITCHES, KIND_PEDAL, KIND_ENCODER, KIND_KNOB };

enum Mode : uint8_t {
  M_NONE, M_PEQUOD, M_CC, M_NOTE, M_PC,
  M_LAYER_NEXT, M_LAYER_PREV, M_LAYER_SET, M_LAYER_HOLD,
  M_CC_REL, M_CC_ABS, M_LAYER_DIR
};

enum LedMode : uint8_t { LED_AUTO, LED_ON, LED_OFF, LED_MIDI };

struct Action {
  uint8_t mode = M_PEQUOD;
  uint8_t num = 0;
  uint8_t ch = 1;
  bool toggle = true;
};

struct Control {
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
  char key[9] = { 0 };
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
};

struct Reassembler {
  String buf;
  int seq = -1, nextPart = 0, parts = 0;
  unsigned long started = 0;
};

enum Transport : uint8_t { T_NONE, T_USB, T_UDP };

// ── Globals ─────────────────────────────────────────────────────────────────
USBMIDI usbMidi;
WiFiUDP udp;
Adafruit_SSD1306 display(128, 64, &Wire1, -1);
bool displayOk = false;

Board boards[MAX_BOARDS];
int numBoards = 0;

JsonDocument cfg;            // master copy of the settings (also what PEQUOD edits)
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
unsigned long lastRx[3] = { 0, 0, 0 };
IPAddress udpPeer;
uint16_t udpPeerPort = 0;
uint8_t txSeq = 0;
Reassembler rxUsb, rxUdp;
bool rawStream = false;
bool wasLinked = false;

// display data from PEQUOD
String dispBank, dispPreset, dispMsg;
int dispIdx = 0;
bool dispMuted = false;
unsigned long dispMsgUntil = 0;
String tunNote = "-";
float tunCents = 0, tunHz = 0;
unsigned long tunAt = 0;
bool displayDirty = true;

// wifi
bool wifiWasConnected = false;
bool wifiScanPending = false;

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

Kind kindForType(uint8_t t) {
  if ((t >= 20 && t <= 39) || (t >= 60 && t <= 79)) return KIND_SWITCHES;
  if (t >= 40 && t <= 59) return KIND_PEDAL;
  if (t >= 100 && t <= 119) return KIND_ENCODER;
  if (t >= 80 && t <= 99) return KIND_KNOB;
  return KIND_NONE;
}

const char* kindName(Kind k) {
  switch (k) {
    case KIND_SWITCHES: return "switches";
    case KIND_PEDAL:    return "pedal";
    case KIND_ENCODER:  return "encoder";
    case KIND_KNOB:     return "knob";
    default:            return "other";
  }
}

int controlCount(const Board& b) {
  switch (b.kind) {
    case KIND_SWITCHES: return 4;
    case KIND_PEDAL:    return b.type == 41 ? 2 : 1;
    case KIND_ENCODER:  return 2;
    default:            return 0;
  }
}

// Is control c of board b a push switch (as opposed to a pedal or encoder rotation)?
bool isSwitch(const Board& b, int c) {
  if (b.kind == KIND_SWITCHES) return true;
  if (b.kind == KIND_PEDAL) return c == 1;
  if (b.kind == KIND_ENCODER) return c == 1;
  return false;
}

// Physical LED number on the slave for control c, or 0 if the control has no LED.
int ledIndex(const Board& b, int c) {
  if (b.kind == KIND_SWITCHES) return c + 1;
  if (b.kind == KIND_PEDAL && b.type == 41 && c == 1) return 1;
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

const char* ledName(uint8_t l) {
  switch (l) { case LED_ON: return "on"; case LED_OFF: return "off"; case LED_MIDI: return "midi"; default: return "auto"; }
}

uint8_t ledFromName(const char* s) {
  if (!s) return LED_AUTO;
  if (!strcmp(s, "on")) return LED_ON;
  if (!strcmp(s, "off")) return LED_OFF;
  if (!strcmp(s, "midi")) return LED_MIDI;
  return LED_AUTO;
}

bool linked() {
  return active != T_NONE && millis() - lastRx[active] < 3000;
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

// ── Link: sending ───────────────────────────────────────────────────────────
void usbSendSysEx(const uint8_t* data, size_t n) {
  // USB-MIDI 1.0 SysEx: CIN 0x4 = start/continue (3 bytes), 0x5/0x6/0x7 = end with 1/2/3 bytes.
  size_t i = 0;
  while (i < n) {
    size_t left = n - i;
    midiEventPacket_t p;
    uint8_t cin;
    uint8_t b[3] = { 0, 0, 0 };
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

void sendFrame(Transport tr, const uint8_t* frame, size_t n) {
  if (tr == T_USB) {
    uint8_t sx[220];
    sx[0] = 0xF0; sx[1] = 0x7D;
    memcpy(sx + 2, frame, n);
    sx[n + 2] = 0xF7;
    usbSendSysEx(sx, n + 3);
  } else if (tr == T_UDP && udpPeerPort != 0 && WiFi.isConnected()) {
    udp.beginPacket(udpPeer, udpPeerPort);
    udp.write(frame, n);
    udp.endPacket();
  }
}

void sendTextOn(Transport tr, const String& json) {
  if (tr == T_NONE) return;
  const int chunk = 200;
  int len = json.length();
  int parts = max(1, (len + chunk - 1) / chunk);
  if (parts > 127) return;   // too big for one message
  uint8_t seq = txSeq++ & 0x7F;
  uint8_t frame[6 + chunk];
  for (int p = 0; p < parts; p++) {
    frame[0] = 0x50; frame[1] = 0x51; frame[2] = 0x01;
    frame[3] = seq; frame[4] = (uint8_t) p; frame[5] = (uint8_t) parts;
    int off = p * chunk, n = min(chunk, len - off);
    for (int k = 0; k < n; k++) {
      uint8_t ch = (uint8_t) json[off + k];
      frame[6 + k] = ch < 0x80 ? ch : '?';      // keep every byte 7-bit for SysEx
    }
    sendFrame(tr, frame, 6 + n);
  }
}

void sendDoc(Transport tr, JsonDocument& doc) {
  String s;
  serializeJson(doc, s);
  sendTextOn(tr, s);
}

void sendToPequod(JsonDocument& doc) {
  if (linked()) sendDoc(active, doc);
}

// ── Config → runtime ────────────────────────────────────────────────────────
void defaultControl(JsonObject c, const Board& b, int idx) {
  static const char* palette[4] = { "#E0A82E", "#4CAF50", "#2196F3", "#E91E63" };
  c["color"] = palette[idx & 3];
  c["led"] = "auto";
  JsonArray layers = c["layers"].to<JsonArray>();
  for (int l = 0; l < MAX_LAYERS; l++) {
    JsonObject a = layers.add<JsonObject>();
    a["mode"] = "pequod";
    a["num"] = (b.kind == KIND_PEDAL && idx == 0) ? 11 : 20 + l * 8 + idx;
    a["ch"] = 1;
    a["toggle"] = true;
  }
}

// Makes sure cfg has an entry for every connected board, with the right number of controls.
void ensureBoardEntries() {
  JsonObject devs = cfg["devices"].is<JsonObject>() ? cfg["devices"].as<JsonObject>() : cfg["devices"].to<JsonObject>();
  for (int i = 0; i < numBoards; i++) {
    Board& b = boards[i];
    JsonObject d = devs[b.key].is<JsonObject>() ? devs[b.key].as<JsonObject>() : devs[b.key].to<JsonObject>();
    if (!d["name"].is<const char*>()) d["name"] = String(b.kind == KIND_PEDAL ? "Pedal " : b.kind == KIND_ENCODER ? "Knob " : "Board ") + String(i + 1);
    JsonArray ctrls = d["controls"].is<JsonArray>() ? d["controls"].as<JsonArray>() : d["controls"].to<JsonArray>();
    while ((int) ctrls.size() < controlCount(b)) {
      int idx = (int) ctrls.size();
      defaultControl(ctrls.add<JsonObject>(), b, idx);
    }
    for (JsonObject c : ctrls) {
      JsonArray layers = c["layers"].is<JsonArray>() ? c["layers"].as<JsonArray>() : c["layers"].to<JsonArray>();
      while (layers.size() < MAX_LAYERS) {
        JsonObject a = layers.add<JsonObject>();
        a["mode"] = "pequod"; a["num"] = 20; a["ch"] = 1; a["toggle"] = true;
      }
    }
    if (b.kind == KIND_PEDAL && !d["pedal"].is<JsonObject>()) {
      JsonObject p = d["pedal"].to<JsonObject>();
      p["min"] = 0; p["max"] = 4095; p["invert"] = false;
    }
    if (b.kind == KIND_ENCODER && !d["encoder"].is<JsonObject>()) d["encoder"].to<JsonObject>()["invert"] = false;
  }
}

void assignSlots() {
  JsonObject devs = cfg["devices"].as<JsonObject>();
  bool used[MAX_SLOTS] = { false };
  for (int i = 0; i < numBoards; i++) boards[i].slot = -1;

  // Keep each board's remembered slot when it's free...
  for (int i = 0; i < numBoards; i++) {
    int s = devs[boards[i].key]["slot"] | -1;
    if (s >= 0 && s < MAX_SLOTS && !used[s]) { boards[i].slot = s; used[s] = true; }
  }
  // ...and give new boards the first free one.
  for (int i = 0; i < numBoards; i++) {
    if (boards[i].slot >= 0) continue;
    for (int s = 0; s < MAX_SLOTS; s++)
      if (!used[s]) { boards[i].slot = s; used[s] = true; break; }
    devs[boards[i].key]["slot"] = boards[i].slot;
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

  JsonObject devs = cfg["devices"].as<JsonObject>();
  for (int i = 0; i < numBoards; i++) {
    Board& b = boards[i];
    JsonObject d = devs[b.key];
    b.numCtrls = controlCount(b);
    JsonArray ctrls = d["controls"];
    for (int c = 0; c < b.numCtrls; c++) {
      JsonObject jc = ctrls[c];
      Control& ctl = b.ctl[c];
      const char* col = jc["color"] | "#E0A82E";
      ctl.colorIsLayer = !strcmp(col, "layer");
      ctl.color = parseColor(col, 0xE0A82E);
      ctl.led = ledFromName(jc["led"] | "auto");
      JsonArray layers = jc["layers"];
      for (int l = 0; l < MAX_LAYERS; l++) {
        JsonObject a = layers[l];
        ctl.act[l].mode = modeFromName(a["mode"] | "pequod");
        ctl.act[l].num = constrain((int) (a["num"] | 20), 0, 127);
        ctl.act[l].ch = constrain((int) (a["ch"] | 1), 1, 16);
        ctl.act[l].toggle = a["toggle"] | true;
      }
    }
    if (b.kind == KIND_PEDAL) {
      b.pedalMin = d["pedal"]["min"] | 0;
      b.pedalMax = d["pedal"]["max"] | 4095;
      if (b.pedalMax <= b.pedalMin + 10) b.pedalMax = b.pedalMin + 10;
      b.pedalInvert = d["pedal"]["invert"] | false;
    }
    if (b.kind == KIND_ENCODER) b.encInvert = d["encoder"]["invert"] | false;
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

// ── Import from the previous master's files (first boot only) ───────────────
bool readJsonFile(const char* path, JsonDocument& doc) {
  if (!SPIFFS.exists(path)) return false;
  File f = SPIFFS.open(path, "r");
  if (!f) return false;
  bool ok = !deserializeJson(doc, f);
  f.close();
  return ok;
}

void importOldSettings() {
  Serial.println("[CFG] first boot: importing previous settings");
  cfg.clear();
  cfg["name"] = "Longsleeves";

  JsonDocument w;
  if (readJsonFile("/wifi.json", w)) {
    cfg["wifi"]["ssid"] = w["primarySSID"] | "";
    cfg["wifi"]["pass"] = w["primaryPass"] | "";
    String host = w["hostname"] | "pequod-fc";
    cfg["wifi"]["hostname"] = host.length() ? host : String("pequod-fc");
  }
  cfg["wifi"]["enabled"] = true;

  JsonDocument ui;
  if (readJsonFile("/ui.json", ui)) {
    int m = ui["midiOutputMode"] | 2;           // 0 DIN only, 1 USB only, 2 both
    cfg["midi"]["usb"] = (m != 0);
    cfg["midi"]["din"] = (m != 1);
  }

  JsonDocument lc;
  JsonArray colors = cfg["layerColors"].to<JsonArray>();
  bool haveLc = readJsonFile("/layer_colors.json", lc);
  for (int l = 0; l < MAX_LAYERS; l++)
    colors.add(haveLc && lc["layer" + String(l)].is<const char*>() ? String(lc["layer" + String(l)].as<const char*>())
                                                                    : colorToHex(layerColors[l]));

  Preferences prefs;
  prefs.begin("ui", true);
  cfg["layers"] = constrain(prefs.getInt("numLayers", 1), 1, MAX_LAYERS);
  cfg["brightness"] = constrain(prefs.getInt("ledBright", 200), 5, 255);
  prefs.end();

  // Per-board settings: slot order, colours, pedal calibration, MIDI numbers.
  // Modes start as "pequod"; the old MIDI numbers are kept for when you switch a control back to MIDI.
  JsonObject devs = cfg["devices"].to<JsonObject>();
  for (int i = 0; i < numBoards; i++) {
    Board& b = boards[i];
    JsonDocument od;
    String path = String("/device_") + b.key + ".json";
    if (!readJsonFile(path.c_str(), od)) continue;

    JsonObject d = devs[b.key].to<JsonObject>();
    d["slot"] = od["preferredSlot"] | -1;
    JsonArray ctrls = d["controls"].to<JsonArray>();
    int n = controlCount(b);
    for (int c = 0; c < n; c++) {
      JsonObject jc = ctrls.add<JsonObject>();
      defaultControl(jc, b, c);
      int oldBtn = (b.kind == KIND_PEDAL && c == 1) ? 1 : c;   // old firmware: pedal switch used index 1
      if (od["leds"][oldBtn]["color"].is<const char*>()) jc["color"] = od["leds"][oldBtn]["color"].as<const char*>();
      for (int l = 0; l < MAX_LAYERS; l++) {
        JsonObject ob = od["layers"][l]["buttons"][oldBtn];
        if (ob.isNull()) continue;
        JsonObject a = jc["layers"][l];
        a["num"] = constrain((int) (ob["mapping"] | a["num"].as<int>()), 0, 127);
        a["ch"] = constrain((int) (ob["channel"] | 1), 1, 16);
        a["toggle"] = strcmp(ob["behavior"] | "momentary", "toggle") == 0;
      }
    }
    if (b.kind == KIND_PEDAL) {
      d["pedal"]["min"] = od["pedal"]["calMin"] | 0;
      d["pedal"]["max"] = od["pedal"]["calMax"] | 4095;
      d["pedal"]["invert"] = od["pedal"]["invert"] | false;
    }
    if (b.kind == KIND_ENCODER) d["encoder"]["invert"] = od["encoder"]["invertDirection"] | false;
  }
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
  cfg["wifi"]["hostname"] = "pequod-fc";
  cfg["display"]["contrast"] = 200;
  cfg["display"]["tunerWhenMuted"] = true;
  cfg["devices"].to<JsonObject>();
}

void loadConfig() {
  JsonDocument d;
  if (readJsonFile("/pqfc.json", d)) {
    cfg = d;
  } else {
    defaultSettings();
    if (SPIFFS.exists("/wifi.json") || SPIFFS.exists("/ui.json") || SPIFFS.exists("/layer_colors.json")) {
      importOldSettings();
      JsonDocument defs;
      defs.set(cfg);
      defaultSettings();              // fill anything the import didn't set
      for (JsonPair kv : defs.as<JsonObject>()) cfg[kv.key()] = kv.value();
    }
    markConfigDirty();
  }
  if (!cfg["display"].is<JsonObject>()) { cfg["display"]["contrast"] = 200; cfg["display"]["tunerWhenMuted"] = true; }
  if (!cfg["offGlow"].is<int>()) cfg["offGlow"] = 10;
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

void slaveSetAllLeds(uint8_t addr, const uint32_t leds[5]) {
  Wire.beginTransmission(addr);
  Wire.write(0x21);
  for (int i = 0; i < 5; i++) {
    uint32_t c = scaleColor(leds[i], brightness);
    Wire.write((uint8_t) ((c >> 16) & 0xFF)); Wire.write((uint8_t) ((c >> 8) & 0xFF)); Wire.write((uint8_t) (c & 0xFF));
  }
  Wire.endTransmission();
}

void slaveSetLayerColor(uint8_t addr, uint32_t color) {
  uint32_t c = scaleColor(color, brightness);
  Wire.beginTransmission(addr);
  Wire.write(0x22);
  Wire.write((uint8_t) ((c >> 16) & 0xFF)); Wire.write((uint8_t) ((c >> 8) & 0xFF)); Wire.write((uint8_t) (c & 0xFF));
  Wire.endTransmission();
}

void scanBoards() {
  numBoards = 0;
  for (uint8_t addr = 1; addr < 0x78 && numBoards < MAX_BOARDS; addr++) {
    uint64_t id; uint8_t type;
    if (slaveId(addr, id) && slaveType(addr, type)) {
      Kind k = kindForType(type);
      if (k == KIND_NONE) continue;          // another master or unknown device
      Board& b = boards[numBoards];
      b = Board();
      b.addr = addr; b.id = id; b.type = type; b.kind = k;
      snprintf(b.key, sizeof(b.key), "%08lX", (unsigned long) (id & 0xFFFFFFFFULL));   // same key as the old firmware
      b.numCtrls = controlCount(b);
      numBoards++;
      Serial.printf("[I2C] 0x%02X id %s type %d (%s)\n", addr, b.key, type, kindName(k));
    }
    delay(5);
  }
  Serial.printf("[I2C] %d board(s)\n", numBoards);
}

// ── Layers ──────────────────────────────────────────────────────────────────
void sendLayer() {
  JsonDocument d;
  d["t"] = "layer"; d["v"] = layer;
  sendToPequod(d);
}

void setLayer(int l) {
  l = constrain(l, 0, numLayers - 1);
  if (l == layer) return;
  layer = l;
  for (int i = 0; i < numBoards; i++)
    if (boards[i].type >= 20 && boards[i].type <= 39) slaveSetLayerColor(boards[i].addr, layerColors[layer]);
  sendLayer();
  displayDirty = true;
}

// ── Control handling ────────────────────────────────────────────────────────
void sendEvent(Board& b, int c, int lay, int v, float f, bool withF) {
  JsonDocument d;
  d["t"] = "ev"; d["dev"] = b.key; d["c"] = c; d["l"] = lay; d["v"] = v;
  if (withF) d["f"] = f;
  sendToPequod(d);
}

void sendPress(Board& b, int c, bool pressed) {
  JsonDocument d;
  d["t"] = "press"; d["dev"] = b.key; d["c"] = c; d["v"] = pressed ? 1 : 0;
  sendToPequod(d);
}

void onSwitch(Board& b, int c, bool pressed) {
  Control& ctl = b.ctl[c];
  ctl.down = pressed;
  if (pressed) ctl.pressLayer = layer;
  const int lay = ctl.pressLayer;          // a release uses the layer it was pressed on
  const Action& a = ctl.act[lay];
  sendPress(b, c, pressed);

  switch (a.mode) {
    case M_PEQUOD:
      if (linked()) sendEvent(b, c, lay, pressed ? 1 : 0, 0, false);
      break;
    case M_CC:
      if (a.toggle) { if (pressed) { ctl.toggled[lay] = !ctl.toggled[lay]; midiCC(a.num, ctl.toggled[lay] ? 127 : 0, a.ch); } }
      else midiCC(a.num, pressed ? 127 : 0, a.ch);
      break;
    case M_NOTE:
      if (a.toggle) { if (pressed) { ctl.toggled[lay] = !ctl.toggled[lay]; midiNote(a.num, ctl.toggled[lay], a.ch); } }
      else midiNote(a.num, pressed, a.ch);
      break;
    case M_PC:
      if (pressed) midiPC(a.num, a.ch);
      break;
    case M_LAYER_NEXT: if (pressed) setLayer((layer + 1) % numLayers); break;
    case M_LAYER_PREV: if (pressed) setLayer((layer + numLayers - 1) % numLayers); break;
    case M_LAYER_SET:  if (pressed) setLayer(max(0, (int) a.num - 1)); break;
    case M_LAYER_HOLD:
      if (pressed) { holdReturnLayer = layer; setLayer(max(0, (int) a.num - 1)); }
      else if (holdReturnLayer >= 0) { setLayer(holdReturnLayer); holdReturnLayer = -1; }
      break;
    default: break;
  }
}

void onPedal(Board& b, int raw) {
  b.rawPedal = raw;
  int lo = b.pedalMin, hi = b.pedalMax;
  int v10 = map(constrain(raw, lo, hi), lo, hi, 0, 1023);
  if (b.pedalInvert) v10 = 1023 - v10;
  if (abs(v10 - b.lastPedal10) < 4 && v10 != 0 && v10 != 1023) return;   // ignore jitter, always reach the ends
  if (v10 == b.lastPedal10) return;
  b.lastPedal10 = v10;
  int v7 = (v10 * 127 + 511) / 1023;
  const Action& a = b.ctl[0].act[layer];
  if (a.mode == M_PEQUOD) {
    if (linked()) sendEvent(b, 0, layer, v7, v10 / 1023.0f, true);
  } else if (a.mode == M_CC && v7 != b.lastPedal7) {
    midiCC(a.num, v7, a.ch);
  }
  b.lastPedal7 = v7;
}

void onEncoder(Board& b, int steps) {
  if (b.encInvert) steps = -steps;
  Control& ctl = b.ctl[0];
  const Action& a = ctl.act[layer];
  switch (a.mode) {
    case M_PEQUOD: if (linked()) sendEvent(b, 0, layer, steps, 0, false); break;
    case M_CC_REL: midiCC(a.num, constrain(64 + steps, 1, 127), a.ch); break;
    case M_CC_ABS:
      ctl.absValue = constrain((int) ctl.absValue + steps, 0, 127);
      midiCC(a.num, ctl.absValue, a.ch);
      break;
    case M_PC: {
      int p = ((lastPcNum == 255 ? 0 : lastPcNum) + steps) % 128;
      if (p < 0) p += 128;
      midiPC(p, a.ch);
      break;
    }
    case M_LAYER_DIR: setLayer(((layer + (steps > 0 ? 1 : -1)) % numLayers + numLayers) % numLayers); break;
    default: break;
  }
}

// ── LEDs ────────────────────────────────────────────────────────────────────
uint32_t controlColor(Board& b, int c, bool& on) {
  Control& ctl = b.ctl[c];
  const Action& a = ctl.act[layer];
  uint32_t col = ctl.colorIsLayer ? layerColors[layer] : ctl.color;

  switch (ctl.led) {
    case LED_ON:   on = true; break;
    case LED_OFF:  on = false; break;
    case LED_MIDI: on = ctl.midiOn; break;
    default:
      switch (a.mode) {
        case M_PEQUOD:
          on = ctl.down || (linked() && ctl.pqOn);
          if (linked() && ctl.pqHasColor) col = ctl.pqColor;
          break;
        case M_CC: case M_NOTE: on = a.toggle ? ctl.toggled[layer] : ctl.down; break;
        case M_PC:          on = (lastPcCh == a.ch && lastPcNum == a.num); break;
        case M_LAYER_SET:
        case M_LAYER_HOLD:  on = (layer == (int) a.num - 1); break;
        case M_LAYER_NEXT:
        case M_LAYER_PREV:  on = ctl.down; break;
        default:            on = false; break;
      }
  }

  if (millis() < b.identifyUntil) {                       // "which board is this?" flash
    on = ((millis() / 150) % 2) == 0;
    col = 0xFFFFFF;
    return on ? col : 0;
  }
  if (ctl.led == LED_OFF) return 0;
  if (!on) return offGlow == 0 ? 0 : scaleColor(col, (uint8_t) (offGlow * 255 / 100));
  return col;
}

void boardLeds(Board& b, uint32_t leds[5]) {
  for (int i = 0; i < 5; i++) leds[i] = 0;
  for (int c = 0; c < b.numCtrls; c++) {
    int idx = ledIndex(b, c);
    if (idx <= 0) continue;
    bool on = false;
    leds[idx] = controlColor(b, c, on);
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

    if (b.kind == KIND_SWITCHES) {
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
        if (now != before) onSwitch(b, c, now);
      }
      b.lastButtons = states;
    }
    else if (b.kind == KIND_PEDAL) {
      bool withSwitch = (b.type == 41);
      if (withSwitch) {
        uint32_t leds[5];
        boardLeds(b, leds);
        slaveSetAllLeds(b.addr, leds);
      }
      Wire.beginTransmission(b.addr);
      Wire.write(0x02);
      if (Wire.endTransmission() != 0) { b.failUntil = millis() + 50; continue; }
      uint8_t want = withSwitch ? 3 : 2;
      Wire.requestFrom(b.addr, want);
      if (Wire.available() < 2) continue;
      int raw = (Wire.read() << 8);
      raw |= Wire.read();
      onPedal(b, raw);
      if (withSwitch && Wire.available() >= 1) {
        uint8_t btn = Wire.read();
        bool now = btn & 0x01, before = b.lastPedalBtn & 0x01;
        if (now != before) onSwitch(b, 1, now);
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
      if (btn & 0x02) onSwitch(b, 1, btn & 0x01);
    }
  }
}

// ── Link: receiving ─────────────────────────────────────────────────────────
void sendHello(Transport tr) {
  JsonDocument d;
  d["t"] = "hello";
  d["name"] = cfg["name"] | "Longsleeves";
  d["fw"] = FW_VERSION;
  d["proto"] = PROTO_VERSION;
  d["layer"] = layer;
  d["layers"] = numLayers;
  d["transport"] = tr == T_USB ? "usb" : "wifi";
  d["wifi"]["connected"] = WiFi.isConnected();
  d["wifi"]["ssid"] = WiFi.isConnected() ? WiFi.SSID() : String();
  d["wifi"]["ip"] = WiFi.isConnected() ? WiFi.localIP().toString() : String();
  sendDoc(tr, d);
}

void sendDevices(Transport tr) {
  JsonDocument d;
  d["t"] = "devices";
  JsonArray list = d["list"].to<JsonArray>();
  for (int i = 0; i < numBoards; i++) {
    JsonObject o = list.add<JsonObject>();
    o["dev"] = boards[i].key; o["addr"] = boards[i].addr; o["type"] = boards[i].type;
    o["kind"] = kindName(boards[i].kind); o["slot"] = boards[i].slot;
  }
  sendDoc(tr, d);
}

void sendConfig(Transport tr) {
  JsonDocument d;
  d["t"] = "cfg";
  d["cfg"].set(cfg);
  const char* pass = cfg["wifi"]["pass"] | "";
  d["cfg"]["wifi"]["hasPass"] = strlen(pass) > 0;
  d["cfg"]["wifi"]["pass"] = nullptr;                   // never send the password back
  sendDoc(tr, d);
}

void sendWifiStatus() {
  JsonDocument d;
  d["t"] = "wifi";
  d["connected"] = WiFi.isConnected();
  d["ssid"] = WiFi.isConnected() ? WiFi.SSID() : String();
  d["ip"] = WiFi.isConnected() ? WiFi.localIP().toString() : String();
  d["rssi"] = WiFi.isConnected() ? WiFi.RSSI() : 0;
  sendToPequod(d);
}

void startWifi();

void handleMessage(Transport tr, JsonDocument& m) {
  const char* t = m["t"] | "";
  lastRx[tr] = millis();
  if (strcmp(t, "hello") != 0) active = tr;           // the transport PEQUOD chose to use

  if (!strcmp(t, "hello"))          { sendHello(tr); }
  else if (!strcmp(t, "ping"))      { /* keep-alive */ }
  else if (!strcmp(t, "getcfg"))    { sendConfig(tr); }
  else if (!strcmp(t, "getdev"))    { sendDevices(tr); }
  else if (!strcmp(t, "rescan"))    { scanBoards(); applyConfig(); markConfigDirty(); sendDevices(tr); sendConfig(tr); }
  else if (!strcmp(t, "setcfg")) {
    String oldSsid = cfg["wifi"]["ssid"] | "", oldPass = cfg["wifi"]["pass"] | "";
    bool oldEnabled = cfg["wifi"]["enabled"] | true;
    JsonDocument incoming;
    incoming.set(m["cfg"]);
    if (incoming["wifi"]["pass"].isNull()) incoming["wifi"]["pass"] = oldPass;  // null = keep
    incoming["wifi"].remove("hasPass");
    cfg = incoming;
    applyConfig();
    markConfigDirty();
    JsonDocument ok; ok["t"] = "cfgok"; sendDoc(tr, ok);
    String newSsid = cfg["wifi"]["ssid"] | "", newPass = cfg["wifi"]["pass"] | "";
    bool newEnabled = cfg["wifi"]["enabled"] | true;
    if (newSsid != oldSsid || newPass != oldPass || newEnabled != oldEnabled) startWifi();
    if (displayOk) { display.ssd1306_command(SSD1306_SETCONTRAST); display.ssd1306_command((uint8_t) (cfg["display"]["contrast"] | 200)); }
  }
  else if (!strcmp(t, "leds")) {
    for (int i = 0; i < numBoards; i++)
      for (int c = 0; c < MAX_CTRLS; c++) { boards[i].ctl[c].pqOn = false; boards[i].ctl[c].pqHasColor = false; }
    for (JsonVariant ev : m["v"].as<JsonArray>()) {
      JsonArray e = ev.as<JsonArray>();
      Board* b = boardByKey(e[0] | "");
      int c = e[1] | -1;
      if (!b || c < 0 || c >= b->numCtrls) continue;
      b->ctl[c].pqOn = (e[2] | 0) != 0;
      if (e[3].is<const char*>()) { b->ctl[c].pqHasColor = true; b->ctl[c].pqColor = parseColor(e[3].as<const char*>(), b->ctl[c].color); }
    }
  }
  else if (!strcmp(t, "disp")) {
    dispBank = m["bank"] | "";
    dispPreset = m["preset"] | "";
    dispIdx = m["idx"] | 0;
    dispMuted = m["muted"] | false;
    String msg = m["msg"] | "";
    if (msg.length()) { dispMsg = msg; dispMsgUntil = millis() + 1500; }
    displayDirty = true;
  }
  else if (!strcmp(t, "tun")) {
    tunNote = m["n"] | "-";
    tunCents = m["c"] | 0.0f;
    tunHz = m["hz"] | 0.0f;
    tunAt = millis();
    displayDirty = true;
  }
  else if (!strcmp(t, "raw"))       { rawStream = (m["on"] | 0) != 0; }
  else if (!strcmp(t, "identify"))  { if (Board* b = boardByKey(m["dev"] | "")) b->identifyUntil = millis() + 3000; }
  else if (!strcmp(t, "wifiscan"))  { WiFi.scanNetworks(true); wifiScanPending = true; }
  else if (!strcmp(t, "reboot"))    { if (cfgDirty) saveConfig(); delay(100); ESP.restart(); }
  else if (!strcmp(t, "factory"))   {
    String name = cfg["name"] | "Longsleeves";
    defaultSettings();
    cfg["name"] = name;
    saveConfig();
    delay(100);
    ESP.restart();
  }
}

void feedFrame(Transport tr, Reassembler& r, const uint8_t* f, size_t n) {
  if (n < 6 || f[0] != 0x50 || f[1] != 0x51 || f[2] != 0x01) return;
  int seq = f[3], part = f[4], parts = f[5];
  if (parts < 1) return;
  if (part == 0) { r.buf = ""; r.seq = seq; r.nextPart = 0; r.parts = parts; r.started = millis(); }
  if (seq != r.seq || part != r.nextPart || parts != r.parts || millis() - r.started > 1000) { r.seq = -1; return; }
  r.buf.concat((const char*) (f + 6), n - 6);
  r.nextPart++;
  if (r.nextPart < r.parts) return;
  r.seq = -1;
  JsonDocument m;
  if (deserializeJson(m, r.buf)) return;
  r.buf = "";
  handleMessage(tr, m);
}

// USB-MIDI input: SysEx carries the link; notes/CCs drive LEDs in "midi" mode.
uint8_t usbSx[1400];
size_t usbSxLen = 0;
bool usbSxActive = false;

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
      if (a.num == d1 && a.ch == ch && ((isNote && a.mode == M_NOTE) || (isCC && a.mode != M_NOTE)))
        ctl.midiOn = on;
    }
}

void usbSysExByte(uint8_t b) {
  if (b == 0xF0) { usbSxActive = true; usbSxLen = 0; return; }
  if (!usbSxActive) return;
  if (b == 0xF7) {
    usbSxActive = false;
    if (usbSxLen >= 1 && usbSx[0] == 0x7D) feedFrame(T_USB, rxUsb, usbSx + 1, usbSxLen - 1);
    return;
  }
  if (usbSxLen < sizeof(usbSx)) usbSx[usbSxLen++] = b;
  else usbSxActive = false;
}

void pollUsbMidi() {
  midiEventPacket_t p;
  int budget = 64;
  while (budget-- > 0 && usbMidi.readPacket(&p)) {
    uint8_t cin = p.header & 0x0F;
    switch (cin) {
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
    if (b >= 0xF8) continue;                      // real-time
    if (b & 0x80) { status = (b < 0xF0) ? b : 0; count = 0; continue; }
    if (!status) continue;
    if (count == 0) { d1 = b; count = 1; }
    else { midiInMessage(status, d1, b); count = 0; }
  }
}

void pollUdp() {
  if (!WiFi.isConnected()) return;
  int size;
  while ((size = udp.parsePacket()) > 0) {
    uint8_t buf[256];
    int n = udp.read(buf, sizeof(buf));
    if (n <= 0) continue;
    udpPeer = udp.remoteIP();
    udpPeerPort = udp.remotePort();
    feedFrame(T_UDP, rxUdp, buf, n);
  }
}

// ── Wi-Fi ───────────────────────────────────────────────────────────────────
void startWifi() {
  WiFi.disconnect(true);
  udp.stop();
  bool enabled = cfg["wifi"]["enabled"] | true;
  String ssid = cfg["wifi"]["ssid"] | "";
  if (!enabled || ssid.length() == 0) { WiFi.mode(WIFI_OFF); return; }
  String host = cfg["wifi"]["hostname"] | "pequod-fc";
  WiFi.mode(WIFI_STA);
  WiFi.setHostname(host.c_str());
  WiFi.setSleep(false);                           // lowest latency
  WiFi.begin(ssid.c_str(), (const char*) (cfg["wifi"]["pass"] | ""));
  wifiWasConnected = false;
}

void loopWifi() {
  bool now = WiFi.isConnected();
  if (now != wifiWasConnected) {
    wifiWasConnected = now;
    if (now) {
      udp.begin(LINK_PORT);
      String host = cfg["wifi"]["hostname"] | "pequod-fc";
      MDNS.end();
      if (MDNS.begin(host.c_str())) MDNS.addService("pequod-fc", "udp", LINK_PORT);
      Serial.printf("[WiFi] %s  %s\n", WiFi.SSID().c_str(), WiFi.localIP().toString().c_str());
    } else {
      udp.stop();
    }
    sendWifiStatus();
    displayDirty = true;
  }
  if (wifiScanPending) {
    int n = WiFi.scanComplete();
    if (n >= 0 || n == WIFI_SCAN_FAILED) {
      wifiScanPending = false;
      JsonDocument d;
      d["t"] = "wifilist";
      JsonArray nets = d["nets"].to<JsonArray>();
      for (int i = 0; i < n && i < 20; i++) {
        JsonObject o = nets.add<JsonObject>();
        o["ssid"] = WiFi.SSID(i); o["rssi"] = WiFi.RSSI(i);
        o["lock"] = WiFi.encryptionType(i) != WIFI_AUTH_OPEN;
      }
      WiFi.scanDelete();
      sendToPequod(d);
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

String fit(const String& s, int maxChars) {
  return (int) s.length() <= maxChars ? s : s.substring(0, maxChars);
}

void drawDisplay() {
  if (!displayOk) return;
  display.clearDisplay();
  display.setTextColor(SSD1306_WHITE);
  bool link = linked();
  bool tunerUp = link && dispMuted && (cfg["display"]["tunerWhenMuted"] | true);

  if (tunerUp) {
    display.setTextSize(1);
    display.setCursor(0, 0);
    display.print("MUTED");
    bool haveNote = tunNote != "-" && millis() - tunAt < 1500;
    if (!haveNote) {
      drawCentered("TUNER", 22, 2);
      display.setTextSize(1);
      drawCentered("play a string", 48, 1);
    } else {
      drawCentered(tunNote, 8, 4);
      // cents bar: centre tick, needle
      display.drawFastHLine(4, 54, 120, SSD1306_WHITE);
      display.drawFastVLine(64, 48, 12, SSD1306_WHITE);
      int x = 64 + (int) constrain(tunCents, -50.0f, 50.0f) * 58 / 50;
      if (fabs(tunCents) < 3) display.fillRect(x - 3, 46, 7, 16, SSD1306_WHITE);
      else display.fillTriangle(x - 4, 46, x + 4, 46, x, 56, SSD1306_WHITE);
      display.setTextSize(1);
      display.setCursor(0, 40); display.print("b");
      display.setCursor(122, 40); display.print("#");
    }
  }
  else if (link) {
    display.setTextSize(1);
    display.setCursor(0, 0);
    display.print(fit(dispBank, numLayers > 1 ? 17 : 21));
    if (numLayers > 1) { display.setCursor(110, 0); display.print("L"); display.print(layer + 1); }
    display.drawFastHLine(0, 10, 128, SSD1306_WHITE);

    if (millis() < dispMsgUntil) {
      drawCentered(fit(dispMsg, 10), 24, 2);
    } else {
      String p = dispPreset;
      if (p.length() <= 10) drawCentered(p, 22, 2);
      else { display.setTextSize(1); display.setCursor(0, 20); display.print(fit(p, 21)); display.setCursor(0, 30); display.print(fit(p.substring(min(21, (int) p.length())), 21)); }
    }
    display.setTextSize(1);
    display.setCursor(0, 56);
    if (dispIdx > 0) { if (dispIdx < 10) display.print("0"); display.print(dispIdx); }
    display.setCursor(80, 56);
    display.print(active == T_USB ? "USB" : "WI-FI");
    display.print(" OK");
  }
  else {
    drawCentered("PEQUOD", 4, 2);
    display.setTextSize(1);
    drawCentered("waiting for host", 26, 1);
    display.setCursor(0, 44);
    if (WiFi.isConnected()) { display.print(WiFi.localIP().toString()); }
    else display.print(String(cfg["wifi"]["ssid"] | "").length() ? "Wi-Fi connecting..." : "USB only");
    display.setCursor(0, 56);
    display.print(numBoards); display.print(numBoards == 1 ? " board" : " boards");
    if (numLayers > 1) { display.setCursor(104, 56); display.print("L"); display.print(layer + 1); }
  }
  display.display();
}

// ── Setup / loop ────────────────────────────────────────────────────────────
void setup() {
  Serial.begin(115200);
  Serial1.begin(31250, SERIAL_8N1, MIDI_RX, MIDI_TX);

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
    display.setTextSize(1);
    drawCentered("v" FW_VERSION, 44, 1);
    display.display();
  }

  scanBoards();
  loadConfig();
  applyConfig();

  // USB name comes from the settings (shown as the MIDI port name on the PC).
  static String usbName;
  usbName = cfg["name"] | "Longsleeves";
  USB.productName(usbName.c_str());
  USB.manufacturerName("PEQUOD");
  usbMidi.begin();
  USB.begin();

  if (displayOk) { display.ssd1306_command(SSD1306_SETCONTRAST); display.ssd1306_command((uint8_t) (cfg["display"]["contrast"] | 200)); }
  for (int i = 0; i < numBoards; i++)
    if (boards[i].type >= 20 && boards[i].type <= 39) slaveSetLayerColor(boards[i].addr, layerColors[layer]);

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

  // keep-alive, and notice when PEQUOD goes away
  static unsigned long lastPing = 0;
  bool link = linked();
  if (link && millis() - lastPing >= 1000) {
    lastPing = millis();
    JsonDocument d; d["t"] = "ping"; sendToPequod(d);
  }
  if (link != wasLinked) {
    wasLinked = link;
    if (!link) { active = T_NONE; rawStream = false; dispMuted = false; }
    else sendLayer();
    displayDirty = true;
  }

  // raw pedal values while PEQUOD's calibration screen is open
  static unsigned long lastRaw = 0;
  if (rawStream && link && millis() - lastRaw >= 50) {
    lastRaw = millis();
    for (int i = 0; i < numBoards; i++)
      if (boards[i].kind == KIND_PEDAL) {
        JsonDocument d; d["t"] = "raw"; d["dev"] = boards[i].key; d["v"] = boards[i].rawPedal;
        sendToPequod(d);
      }
  }

  // save settings shortly after the last change (saves wear on the flash while dragging sliders)
  if (cfgDirty && millis() - cfgDirtySince > 1500) saveConfig();

  static unsigned long lastDraw = 0;
  bool msgExpired = dispMsgUntil != 0 && millis() >= dispMsgUntil;
  if ((displayDirty || msgExpired || (link && dispMuted)) && millis() - lastDraw >= 50) {
    if (msgExpired) dispMsgUntil = 0;
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
