// ═══════════════════════════════════════════════════════════════════════════
//  PEQUOD LINK — native connection between this controller and PEQUOD Host
// ───────────────────────────────────────────────────────────────────────────
//  Transports (same messages on both):
//    USB  : SysEx on the existing USB-MIDI port      F0 7D 'P' 'Q' <frame> F7
//    Wi-Fi: UDP port 7400                            "PQL1" <frame>
//  Frame  : 'J' <msgId> <index> <count> <JSON text, 7-bit ASCII>
//           Long messages are split into <count> frames and joined on arrival.
//
//  The host says {"t":"hello"} and the controller answers {"t":"info",...}.
//  While the host keeps talking (any message at least every 4 s) the link is
//  "connected": controls report events, PEQUOD may drive LEDs and the screen.
//
//  A button / pedal / encoder whose type is "pequod" belongs to PEQUOD: it
//  sends no MIDI of its own and only reports to PEQUOD. Everything else keeps
//  working exactly as before. The slave I2C protocol is not touched at all.
// ═══════════════════════════════════════════════════════════════════════════
#pragma once

#include <Update.h>
#include "mbedtls/base64.h"
#include "esp_ota_ops.h"

#define PQ_FW_VERSION  "1.4.0"
#define PQ_PROTOCOL    1
#define PQ_UDP_PORT    7400
#define PQ_TIMEOUT_MS  4000

// ── link state ──────────────────────────────────────────────────────────────
enum PqTransport { PQ_USB = 0, PQ_UDP = 1 };

struct PqRx { int id = -1; int next = 0; int count = 0; String text; };

struct PqLinkState {
  bool active = false;
  unsigned long lastRx = 0;
  PqRx rx;
} pqLink[2];

WiFiUDP   pqUdp;
bool      pqUdpRunning = false;
IPAddress pqUdpHost;
uint16_t  pqUdpHostPort = 0;
uint8_t   pqMsgId = 0;

bool pqOwnedOverride[8][4] = {{false}};   // LED overrides set by PEQUOD (cleared when it goes away)

struct PqScreen {
  bool   valid = false, dirty = false;
  String mode = "preset";   // "preset" | "tuner" | "text"
  String top, main, sub, note;
  float  cents = 0;
  bool   inTune = false;
  String flash;
  unsigned long flashUntil = 0;
  unsigned long lastDraw = 0;
} pqScreen;

struct PqIdentify { int slot = -1, btn = -1; unsigned long start = 0; } pqIdentify;
bool pqCalWatch[8] = {false};

struct PqFirmware { bool running = false; int lastN = -1; size_t written = 0, size = 0; } pqFw;

bool pqHostConnected() { return pqLink[PQ_USB].active || pqLink[PQ_UDP].active; }
bool pqOwnsDisplay()   { return pqHostConnected() && pqScreen.valid; }

// ── text helpers ────────────────────────────────────────────────────────────
// JSON from the controller must be 7-bit for SysEx: escape anything above 0x7F.
String pqAsciiJson(const String& in) {
  String out; out.reserve(in.length() + 8);
  for (unsigned i = 0; i < in.length(); ) {
    uint8_t c = (uint8_t)in[i];
    if (c < 0x80) { out += (char)c; i++; continue; }
    uint32_t cp = '?'; int n = 1;
    if      ((c & 0xE0) == 0xC0 && i + 1 < in.length()) { cp = ((c & 0x1F) << 6) | (in[i+1] & 0x3F); n = 2; }
    else if ((c & 0xF0) == 0xE0 && i + 2 < in.length()) { cp = ((c & 0x0F) << 12) | ((in[i+1] & 0x3F) << 6) | (in[i+2] & 0x3F); n = 3; }
    else if ((c & 0xF8) == 0xF0) { n = 4; }
    char buf[8]; snprintf(buf, sizeof(buf), "\\u%04X", (unsigned)(cp > 0xFFFF ? '?' : cp));
    out += buf; i += n;
  }
  return out;
}

// ── sending ─────────────────────────────────────────────────────────────────
void pqSendFrames(PqTransport tr, const String& json) {
  const int maxPayload = (tr == PQ_USB) ? 100 : 1000;
  const int len = json.length();
  const int count = max(1, (len + maxPayload - 1) / maxPayload);
  if (count > 127) { Serial.println("[PQ] message too long, dropped"); return; }
  pqMsgId = (pqMsgId + 1) & 0x7F;

  uint8_t buf[1024 + 16];
  for (int i = 0; i < count; i++) {
    int p = 0;
    if (tr == PQ_USB) { buf[p++] = 0xF0; buf[p++] = 0x7D; buf[p++] = 'P'; buf[p++] = 'Q'; }
    else              { buf[p++] = 'P';  buf[p++] = 'Q';  buf[p++] = 'L'; buf[p++] = '1'; }
    buf[p++] = 'J'; buf[p++] = pqMsgId; buf[p++] = (uint8_t)i; buf[p++] = (uint8_t)count;
    for (int k = i * maxPayload; k < min(len, (i + 1) * maxPayload); k++) buf[p++] = (uint8_t)json[k] & 0x7F;
    if (tr == PQ_USB) {
      buf[p++] = 0xF7;
      usbmidi.sendSysEx(buf, (uint16_t)p);
    } else {
      if (!pqUdpRunning || pqUdpHostPort == 0) return;
      pqUdp.beginPacket(pqUdpHost, pqUdpHostPort);
      pqUdp.write(buf, p);
      pqUdp.endPacket();
    }
  }
}

void pqSendDoc(PqTransport tr, JsonDocument& doc) {
  String s; serializeJson(doc, s);
  pqSendFrames(tr, pqAsciiJson(s));
}

// Events go to one place only: USB wins while it is connected.
bool pqPrimary(PqTransport& tr) {
  if (pqLink[PQ_USB].active) { tr = PQ_USB; return true; }
  if (pqLink[PQ_UDP].active) { tr = PQ_UDP; return true; }
  return false;
}

void pqSendEventDoc(JsonDocument& doc) {
  PqTransport tr;
  if (pqPrimary(tr)) pqSendDoc(tr, doc);
}

// ── describing the controller ───────────────────────────────────────────────
String pqKind(uint8_t type) {
  if (type == 41) return "pedal_switch";
  if (type >= TYPE_FOOTSWITCH_MIN && type <= TYPE_FOOTSWITCH_MAX) return "footswitch";
  if (type >= TYPE_PEDAL_MIN && type <= TYPE_PEDAL_MAX) return "pedal";
  if (type >= TYPE_BUTTON_MIN && type <= TYPE_BUTTON_MAX) return "button";
  if (type >= TYPE_KNOB_MIN && type <= TYPE_KNOB_MAX) return "knob";
  if (type >= TYPE_ENCODER_MIN && type <= TYPE_ENCODER_MAX) return "encoder";
  return "other";
}

String pqMacId() {
  uint64_t mac = ESP.getEfuseMac();
  char buf[13]; snprintf(buf, sizeof(buf), "%012llX", mac & 0xFFFFFFFFFFFFULL);
  return String(buf);
}

void pqFillInfo(JsonDocument& doc, PqTransport tr) {
  doc["t"] = "info";
  doc["proto"] = PQ_PROTOCOL;
  doc["fw"] = PQ_FW_VERSION;
  doc["id"] = pqMacId();
  doc["name"] = wifiHostname.length() ? wifiHostname : String("Foot Controller");
  doc["via"] = tr == PQ_USB ? "usb" : "wifi";
  doc["layers"] = configNumLayers;
  doc["layer"] = getCurrentActiveLayer();
  doc["globalLayers"] = globalLayerControl.enabled;
  doc["brightness"] = globalLEDBrightness;

  JsonObject w = doc.createNestedObject("wifi");
  w["connected"] = wifiConnected && WiFi.status() == WL_CONNECTED;
  w["ap"] = apModeActive;
  w["ssid"] = wifiPrimarySSID;
  w["ssid2"] = wifiFallbackSSID;
  w["ip"] = (wifiConnected && WiFi.status() == WL_CONNECTED) ? WiFi.localIP().toString()
                                                             : (apModeActive ? WiFi.softAPIP().toString() : String(""));

  const esp_partition_t* next = esp_ota_get_next_update_partition(NULL);
  JsonObject o = doc.createNestedObject("ota");
  o["ok"] = next != nullptr;
  o["max"] = next ? (uint32_t)next->size : 0;

  JsonArray units = doc.createNestedArray("units");
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* s = getSlaveInfoForSlot(slot);
    if (!s) continue;
    JsonObject u = units.createNestedObject();
    u["slot"] = slot;
    u["type"] = s->type;
    u["kind"] = pqKind(s->type);
    u["name"] = typeToName(s->type);
    u["uid"]  = idToShortKey(s->uniqueID);
    u["layer"] = getSlotActiveLayer(slot);
  }
}

void pqSendInfo(PqTransport tr) {
  DynamicJsonDocument doc(3072);
  pqFillInfo(doc, tr);
  pqSendDoc(tr, doc);
}

// ── device config as JSON (same layout as the SPIFFS file) ──────────────────
void pqWriteDeviceJson(SlaveInfo& s, JsonObject d) {
  DeviceConfig& c = s.config;
  d["deviceId"] = idToShortKey(s.uniqueID);
  d["type"] = s.type;
  JsonObject sh = d.createNestedObject("shift");
  sh["button"] = c.shiftButton; sh["mode"] = c.shiftMode; sh["enabled"] = c.shiftEnabled;

  JsonArray layers = d.createNestedArray("layers");
  for (int L = 0; L < 4; L++) {
    JsonArray buttons = layers.createNestedObject().createNestedArray("buttons");
    for (int b = 0; b < 4; b++) {
      JsonObject o = buttons.createNestedObject();
      o["mapping"] = c.layerMidiMapping[L][b];
      o["type"] = c.layerMidiType[L][b];
      o["channel"] = c.layerMidiChannel[L][b];
      o["behavior"] = c.layerMidiBehavior[L][b];
      o["oscAddr"] = c.layerOscAddr[L][b];
      o["oscOn"] = c.layerOscOn[L][b];
      o["oscOff"] = c.layerOscOff[L][b];
      o["oscArg"] = c.layerOscArg[L][b];
    }
  }
  JsonArray dc = d.createNestedArray("deviceControl");
  for (int b = 0; b < 4; b++) {
    JsonObject o = dc.createNestedObject();
    o["type"] = c.deviceControlType[b]; o["target"] = c.deviceControlTarget[b]; o["mode"] = c.deviceControlMode[b];
  }
  if (s.type >= TYPE_PEDAL_MIN && s.type <= TYPE_PEDAL_MAX) {
    JsonObject p = d.createNestedObject("pedal");
    p["calMin"] = c.pedalCalMin; p["calMax"] = c.pedalCalMax; p["invert"] = c.pedalInvert;
  }
  JsonArray leds = d.createNestedArray("leds");
  for (int b = 0; b < 4; b++) {
    JsonObject o = leds.createNestedObject();
    o["mode"] = c.ledMode[b]; o["color"] = c.ledColor[b];
    o["midiNote"] = c.ledMidiNote[b]; o["midiChannel"] = c.ledMidiChannel[b];
    o["followLayer"] = c.ledFollowLayer[b]; o["oscAddr"] = c.ledOscAddr[b];
  }
  if (s.type >= TYPE_ENCODER_MIN && s.type <= TYPE_ENCODER_MAX) {
    JsonObject e = d.createNestedObject("encoder");
    e["rotationMode"] = c.encoderRotationMode; e["midiCC"] = c.encoderMidiCC; e["midiChannel"] = c.encoderMidiChannel;
    e["minValue"] = c.encoderMinValue; e["maxValue"] = c.encoderMaxValue; e["invertDirection"] = c.encoderInvertDirection;
    e["buttonMode"] = c.encoderButtonMode; e["buttonMidiNote"] = c.encoderButtonMidiNote;
    e["buttonMidiChannel"] = c.encoderButtonMidiChannel; e["stepsPerDetent"] = c.encoderStepsPerDetent;
    e["noteNumber"] = c.encoderNoteNumber; e["noteChannel"] = c.encoderNoteChannel;
    e["oscAddr"] = c.encoderOscAddr; e["oscMode"] = c.encoderOscMode; e["btnOscAddr"] = c.encoderButtonOscAddr;
  }
  d["preferredSlot"] = c.preferredSlot;
}

// Only the keys present in `d` are changed, so PEQUOD can send small patches.
#define PQ_SET(dst, obj, key) do { if (obj.containsKey(key)) dst = obj[key]; } while (0)
#define PQ_SETS(dst, obj, key) do { if (obj.containsKey(key)) dst = obj[key].as<String>(); } while (0)

void pqApplyDeviceJson(SlaveInfo& s, JsonObjectConst d) {
  DeviceConfig& c = s.config;
  if (d.containsKey("shift")) {
    JsonObjectConst sh = d["shift"];
    PQ_SET(c.shiftButton, sh, "button"); PQ_SET(c.shiftMode, sh, "mode"); PQ_SET(c.shiftEnabled, sh, "enabled");
  }
  if (d.containsKey("layers")) {
    JsonArrayConst layers = d["layers"];
    for (int L = 0; L < 4 && L < (int)layers.size(); L++) {
      if (layers[L].isNull() || !layers[L].containsKey("buttons")) continue;
      JsonArrayConst buttons = layers[L]["buttons"];
      for (int b = 0; b < 4 && b < (int)buttons.size(); b++) {
        JsonObjectConst o = buttons[b];
        if (o.isNull()) continue;
        PQ_SET(c.layerMidiMapping[L][b], o, "mapping");
        PQ_SETS(c.layerMidiType[L][b], o, "type");
        PQ_SET(c.layerMidiChannel[L][b], o, "channel");
        PQ_SETS(c.layerMidiBehavior[L][b], o, "behavior");
        PQ_SETS(c.layerOscAddr[L][b], o, "oscAddr");
        PQ_SET(c.layerOscOn[L][b], o, "oscOn");
        PQ_SET(c.layerOscOff[L][b], o, "oscOff");
        PQ_SETS(c.layerOscArg[L][b], o, "oscArg");
      }
    }
  }
  if (d.containsKey("deviceControl")) {
    JsonArrayConst dc = d["deviceControl"];
    for (int b = 0; b < 4 && b < (int)dc.size(); b++) {
      JsonObjectConst o = dc[b]; if (o.isNull()) continue;
      PQ_SETS(c.deviceControlType[b], o, "type"); PQ_SET(c.deviceControlTarget[b], o, "target"); PQ_SETS(c.deviceControlMode[b], o, "mode");
    }
  }
  if (d.containsKey("pedal")) {
    JsonObjectConst p = d["pedal"];
    PQ_SET(c.pedalCalMin, p, "calMin"); PQ_SET(c.pedalCalMax, p, "calMax"); PQ_SET(c.pedalInvert, p, "invert");
  }
  if (d.containsKey("leds")) {
    JsonArrayConst leds = d["leds"];
    for (int b = 0; b < 4 && b < (int)leds.size(); b++) {
      JsonObjectConst o = leds[b]; if (o.isNull()) continue;
      PQ_SETS(c.ledMode[b], o, "mode"); PQ_SETS(c.ledColor[b], o, "color");
      PQ_SET(c.ledMidiNote[b], o, "midiNote"); PQ_SET(c.ledMidiChannel[b], o, "midiChannel");
      PQ_SET(c.ledFollowLayer[b], o, "followLayer"); PQ_SETS(c.ledOscAddr[b], o, "oscAddr");
    }
  }
  if (d.containsKey("encoder")) {
    JsonObjectConst e = d["encoder"];
    PQ_SETS(c.encoderRotationMode, e, "rotationMode"); PQ_SET(c.encoderMidiCC, e, "midiCC"); PQ_SET(c.encoderMidiChannel, e, "midiChannel");
    PQ_SET(c.encoderMinValue, e, "minValue"); PQ_SET(c.encoderMaxValue, e, "maxValue"); PQ_SET(c.encoderInvertDirection, e, "invertDirection");
    PQ_SETS(c.encoderButtonMode, e, "buttonMode"); PQ_SET(c.encoderButtonMidiNote, e, "buttonMidiNote");
    PQ_SET(c.encoderButtonMidiChannel, e, "buttonMidiChannel"); PQ_SET(c.encoderStepsPerDetent, e, "stepsPerDetent");
    PQ_SET(c.encoderNoteNumber, e, "noteNumber"); PQ_SET(c.encoderNoteChannel, e, "noteChannel");
    PQ_SETS(c.encoderOscAddr, e, "oscAddr"); PQ_SETS(c.encoderOscMode, e, "oscMode"); PQ_SETS(c.encoderButtonOscAddr, e, "btnOscAddr");
  }
}

void pqSendConfig(PqTransport tr, int slot) {
  SlaveInfo* s = getSlaveInfoForSlot(slot);
  if (!s) return;
  DynamicJsonDocument doc(12288);
  doc["t"] = "cfg";
  doc["slot"] = slot;
  pqWriteDeviceJson(*s, doc.createNestedObject("cfg"));
  pqSendDoc(tr, doc);
}

void pqSendSettings(PqTransport tr) {
  DynamicJsonDocument doc(1024);
  doc["t"] = "settings";
  doc["layers"] = configNumLayers;
  doc["brightness"] = globalLEDBrightness;
  doc["globalLayers"] = globalLayerControl.enabled;
  JsonObject w = doc.createNestedObject("wifi");
  w["ssid"] = wifiPrimarySSID; w["hasPass"] = wifiPrimaryPass.length() > 0;
  w["ssid2"] = wifiFallbackSSID; w["hasPass2"] = wifiFallbackPass.length() > 0;
  w["host"] = wifiHostname;
  pqSendDoc(tr, doc);
}

void pqSendError(PqTransport tr, const String& what) {
  DynamicJsonDocument doc(256);
  doc["t"] = "err"; doc["msg"] = what;
  pqSendDoc(tr, doc);
}

// ── events from the hardware (called from the polling code) ────────────────
void pqOnButton(int slot, int btnIdx, bool pressed) {
  if (!pqHostConnected()) return;
  StaticJsonDocument<128> d;
  d["t"] = "ev"; d["k"] = "btn"; d["s"] = slot; d["b"] = btnIdx; d["v"] = pressed ? 1 : 0; d["l"] = getSlotActiveLayer(slot);
  pqSendEventDoc(d);
}

void pqOnPedal(int slot, int v127, int raw) {
  if (!pqHostConnected()) return;
  StaticJsonDocument<128> d;
  d["t"] = "ev"; d["k"] = "pedal"; d["s"] = slot; d["v"] = v127; d["raw"] = raw; d["l"] = getSlotActiveLayer(slot);
  pqSendEventDoc(d);
}

void pqOnEncoder(int slot, int delta) {
  if (!pqHostConnected()) return;
  StaticJsonDocument<128> d;
  d["t"] = "ev"; d["k"] = "enc"; d["s"] = slot; d["d"] = delta; d["l"] = getSlotActiveLayer(slot);
  pqSendEventDoc(d);
}

void pqOnEncoderButton(int slot, bool pressed) {
  if (!pqHostConnected()) return;
  StaticJsonDocument<128> d;
  d["t"] = "ev"; d["k"] = "encbtn"; d["s"] = slot; d["v"] = pressed ? 1 : 0; d["l"] = getSlotActiveLayer(slot);
  pqSendEventDoc(d);
}

// ── host went away: hand LEDs and the screen back ──────────────────────────
void pqReleaseHost() {
  for (int s = 0; s < 8; s++)
    for (int b = 0; b < 4; b++)
      if (pqOwnedOverride[s][b]) { pqOwnedOverride[s][b] = false; ledOverrideOn[s][b] = false; }
  pqScreen.valid = false;
  for (int s = 0; s < 8; s++) pqCalWatch[s] = false;
  if (pqFw.running) { Update.abort(); pqFw.running = false; Serial.println("[PQ] firmware update aborted (host lost)"); }
  Serial.println("[PQ] PEQUOD disconnected");
}

// ── firmware update ─────────────────────────────────────────────────────────
void pqFwAck(PqTransport tr, int n, bool ok, const String& err = "") {
  StaticJsonDocument<192> d;
  d["t"] = "fw_ack"; d["n"] = n; d["ok"] = ok;
  if (err.length()) d["err"] = err;
  pqSendDoc(tr, d);
}

void pqFwBegin(PqTransport tr, JsonDocument& in) {
  size_t size = in["size"] | 0;
  const esp_partition_t* next = esp_ota_get_next_update_partition(NULL);
  if (!next) { pqFwAck(tr, -1, false, "This controller was flashed without an update partition"); return; }
  if (size == 0 || size > next->size) { pqFwAck(tr, -1, false, "Firmware is too big for this controller"); return; }
  if (pqFw.running) Update.abort();
  if (!Update.begin(size, U_FLASH)) { pqFwAck(tr, -1, false, String("Update.begin failed: ") + Update.errorString()); return; }
  if (in.containsKey("md5")) Update.setMD5(in["md5"].as<const char*>());
  pqFw.running = true; pqFw.lastN = -1; pqFw.written = 0; pqFw.size = size;
  Serial.printf("[PQ] firmware update started, %u bytes\n", (unsigned)size);
  pqFwAck(tr, -1, true);
}

void pqFwData(PqTransport tr, JsonDocument& in) {
  int n = in["n"] | -1;
  if (!pqFw.running) { pqFwAck(tr, n, false, "No update in progress"); return; }
  if (n == pqFw.lastN) { pqFwAck(tr, n, true); return; }           // a resend: already written
  if (n != pqFw.lastN + 1) { pqFwAck(tr, n, false, "Out of order"); return; }
  const char* b64 = in["d"] | "";
  size_t inLen = strlen(b64);
  static uint8_t bin[1024];
  size_t outLen = 0;
  if (mbedtls_base64_decode(bin, sizeof(bin), &outLen, (const unsigned char*)b64, inLen) != 0) { pqFwAck(tr, n, false, "Bad data"); return; }
  if (Update.write(bin, outLen) != outLen) { pqFwAck(tr, n, false, String("Write failed: ") + Update.errorString()); return; }
  pqFw.lastN = n; pqFw.written += outLen;
  pqFwAck(tr, n, true);
}

void pqFwEnd(PqTransport tr) {
  if (!pqFw.running) { pqFwAck(tr, -2, false, "No update in progress"); return; }
  pqFw.running = false;
  bool ok = Update.end(true);
  StaticJsonDocument<192> d;
  d["t"] = "fw_done"; d["ok"] = ok;
  if (!ok) d["err"] = Update.errorString();
  pqSendDoc(tr, d);
  if (ok) {
    Serial.println("[PQ] firmware updated, restarting");
    display.clearDisplay(); display.setTextSize(1); display.setCursor(0, 24);
    display.println("  UPDATED - RESTARTING"); display.display();
    delay(400);
    ESP.restart();
  }
}

// ── incoming messages ───────────────────────────────────────────────────────
void pqApplyLeds(JsonArrayConst list) {
  for (JsonArrayConst e : list) {
    int slot = e[0] | -1, btn = e[1] | -1;
    if (slot < 0 || slot > 7 || btn < 0 || btn > 3) continue;
    const char* col = e[2] | "";
    if (!col[0]) {                         // "" = give the LED back to the controller
      ledOverrideOn[slot][btn] = false;
      pqOwnedOverride[slot][btn] = false;
    } else {
      ledOverrideOn[slot][btn] = true;
      ledOverrideColor[slot][btn] = hexToRGB(String(col));
      pqOwnedOverride[slot][btn] = true;
    }
    // The polling loop pushes overrides to the slaves on its next pass.
  }
}

void pqApplySettings(PqTransport tr, JsonDocument& in) {
  if (in.containsKey("layers")) {
    int n = constrain((int)(in["layers"] | 4), 1, 4);
    configNumLayers = n;
    prefs.begin("ui", false); prefs.putInt("numLayers", n); prefs.end();
  }
  if (in.containsKey("brightness")) {
    int v = constrain((int)(in["brightness"] | 255), 5, 255);
    globalLEDBrightness = (uint8_t)v;
    prefs.begin("ui", false); prefs.putInt("ledBright", v); prefs.end();
    FastLED.setBrightness(globalLEDBrightness);
    updateLayerLED(getCurrentActiveLayer());
  }
  if (in.containsKey("wifi")) {
    JsonObject w = in["wifi"];
    if (w.containsKey("ssid"))  wifiPrimarySSID  = w["ssid"].as<String>();
    if (w.containsKey("pass"))  wifiPrimaryPass  = w["pass"].as<String>();
    if (w.containsKey("ssid2")) wifiFallbackSSID = w["ssid2"].as<String>();
    if (w.containsKey("pass2")) wifiFallbackPass = w["pass2"].as<String>();
    if (w.containsKey("host"))  wifiHostname     = w["host"].as<String>();
    spiffsConfig.saveWiFiConfig(wifiPrimarySSID, wifiPrimaryPass, wifiFallbackSSID, wifiFallbackPass, wifiHostname);
    WiFi.setHostname(wifiHostname.c_str());
    startWiFi();   // same as saving in the web page
  }
  pqSendSettings(tr);
}

void pqDispatch(PqTransport tr, const String& json) {
  DynamicJsonDocument in(json.length() + 1024);
  if (deserializeJson(in, json)) { Serial.println("[PQ] bad JSON"); return; }
  const String t = in["t"] | "";

  bool wasConnected = pqHostConnected();
  pqLink[tr].active = true;
  pqLink[tr].lastRx = millis();
  if (!wasConnected) Serial.printf("[PQ] PEQUOD connected over %s\n", tr == PQ_USB ? "USB" : "Wi-Fi");

  if (t == "hello")          { pqSendInfo(tr); return; }
  if (t == "ping")           { StaticJsonDocument<64> d; d["t"] = "pong"; pqSendDoc(tr, d); return; }
  if (t == "get_cfg")        { pqSendConfig(tr, in["slot"] | -1); return; }
  if (t == "get_settings")   { pqSendSettings(tr); return; }
  if (t == "set_settings")   { pqApplySettings(tr, in); return; }
  if (t == "leds")           { pqApplyLeds(in["l"].as<JsonArrayConst>()); return; }
  if (t == "layer")          { applyGlobalLayer((int)(in["l"] | 0)); return; }

  if (t == "set_cfg") {
    int slot = in["slot"] | -1;
    SlaveInfo* s = getSlaveInfoForSlot(slot);
    if (!s) { pqSendError(tr, "No unit in slot " + String(slot)); return; }
    pqApplyDeviceJson(*s, in["cfg"].as<JsonObjectConst>());
    saveDeviceConfigWithLayers(deviceMapping[slot]);
    pqSendConfig(tr, slot);
    return;
  }

  if (t == "screen") {
    pqScreen.valid = true;
    pqScreen.mode  = in["mode"] | "preset";
    pqScreen.top   = in["top"]  | "";
    pqScreen.main  = in["main"] | "";
    pqScreen.sub   = in["sub"]  | "";
    pqScreen.note  = in["note"] | "";
    pqScreen.cents = in["cents"] | 0.0f;
    pqScreen.inTune = in["inTune"] | false;
    if (in.containsKey("flash")) { pqScreen.flash = in["flash"].as<String>(); pqScreen.flashUntil = millis() + 1200; }
    pqScreen.dirty = true;
    return;
  }

  if (t == "identify") {
    pqIdentify.slot = in["s"] | -1; pqIdentify.btn = in["b"] | -1; pqIdentify.start = millis();
    return;
  }
  if (t == "cal") {
    int s = in["s"] | -1;
    if (s >= 0 && s < 8) pqCalWatch[s] = in["on"] | false;
    return;
  }
  if (t == "rescan") {
    scanSlaves();
    for (int i = 0; i < numDetected; i++) loadDeviceConfigWithLayers(i);
    autoMapDevices();
    initializeShiftStates();
    initializeEncoderStates();
    pqSendInfo(tr);
    return;
  }
  if (t == "fw_begin") { pqFwBegin(tr, in); return; }
  if (t == "fw_data")  { pqFwData(tr, in); return; }
  if (t == "fw_end")   { pqFwEnd(tr); return; }
  if (t == "fw_abort") { if (pqFw.running) { Update.abort(); pqFw.running = false; } return; }
  if (t == "reboot")   { delay(100); ESP.restart(); return; }
}

// Joins frames into whole messages. `frame` points at 'J'.
void pqOnFrame(PqTransport tr, const uint8_t* frame, int len) {
  if (len < 4 || frame[0] != 'J') return;
  const int id = frame[1], index = frame[2], count = frame[3];
  PqRx& rx = pqLink[tr].rx;
  if (index == 0) { rx.id = id; rx.next = 0; rx.count = count; rx.text = ""; }
  if (rx.id != id || index != rx.next || count != rx.count) { rx.id = -1; return; }   // lost a piece: drop it
  rx.text.concat((const char*)(frame + 4), len - 4);
  rx.next++;
  if (rx.next == rx.count) {
    String msg = rx.text;
    rx.id = -1; rx.text = "";
    pqDispatch(tr, msg);
  }
}

void pqOnSysEx(const uint8_t* data, uint16_t length) {
  // F0 7D 'P' 'Q' <frame> F7
  if (length < 9 || data[0] != 0xF0 || data[1] != 0x7D || data[2] != 'P' || data[3] != 'Q') return;
  int end = (data[length - 1] == 0xF7) ? length - 1 : length;
  pqOnFrame(PQ_USB, data + 4, end - 4);
}

// ── USB MIDI input (SysEx for PEQUOD, channel messages for midi_in LEDs) ────
struct PqMidiCallbacks : CS::MIDI_Callbacks {
  void onChannelMessage(CS::MIDI_Interface&, CS::ChannelMessage msg) override {
    handleMidiInputMessage(msg.header & 0xF0, (msg.header & 0x0F) + 1, msg.data1, msg.data2);
  }
  void onSysExMessage(CS::MIDI_Interface&, CS::SysExMessage msg) override {
    pqOnSysEx(msg.data, msg.length);
  }
} pqMidiCallbacks;

// ── controller screen ───────────────────────────────────────────────────────
void pqCentered(const String& s, int y, int size) {
  display.setTextSize(size);
  int w = s.length() * 6 * size;
  display.setCursor(max(0, (SCREEN_WIDTH - w) / 2), y);
  display.print(s);
}

void pqDrawScreen() {
  display.clearDisplay();
  display.setTextColor(SSD1306_WHITE);

  if (pqScreen.mode == "tuner") {
    pqCentered(pqScreen.note.length() ? pqScreen.note : String("-"), 6, 4);
    // cents scale
    const int y = 50, cx = SCREEN_WIDTH / 2;
    display.drawFastHLine(8, y, SCREEN_WIDTH - 16, SSD1306_WHITE);
    for (int c = -50; c <= 50; c += 25) display.drawFastVLine(cx + c * (SCREEN_WIDTH - 16) / 100, y - (c == 0 ? 6 : 3), c == 0 ? 12 : 6, SSD1306_WHITE);
    if (pqScreen.note.length()) {
      int x = cx + (int)(constrain(pqScreen.cents, -50.0f, 50.0f) * (SCREEN_WIDTH - 16) / 100.0f);
      display.fillRect(x - 2, y - 8, 5, 16, SSD1306_WHITE);
    }
    if (pqScreen.inTune) { display.setTextSize(1); display.setCursor(0, 0); display.print("IN TUNE"); }
  } else {
    pqCentered(pqScreen.top, 0, 1);
    display.drawFastHLine(0, 10, SCREEN_WIDTH, SSD1306_WHITE);
    String m = pqScreen.main;
    if ((int)m.length() <= 10) pqCentered(m, 22, 2);
    else {
      int cut = m.lastIndexOf(' ', 10);
      if (cut <= 0) cut = 10;
      pqCentered(m.substring(0, cut), 14, 2);
      String rest = m.substring(cut); rest.trim();
      if (rest.length() > 10) rest = rest.substring(0, 10);
      pqCentered(rest, 32, 2);
    }
    pqCentered(pqScreen.sub, 56, 1);
  }

  if (pqScreen.flash.length() && millis() < pqScreen.flashUntil) {
    display.fillRect(0, 52, SCREEN_WIDTH, 12, SSD1306_WHITE);
    display.setTextColor(SSD1306_BLACK);
    pqCentered(pqScreen.flash, 54, 1);
    display.setTextColor(SSD1306_WHITE);
  }
  display.display();
}

// ── setup / loop hooks ──────────────────────────────────────────────────────
void pqSetup() {
  usbmidi.setCallbacks(&pqMidiCallbacks);
}

void pqLoop() {
  // Wi-Fi socket follows the network coming up.
  if (!pqUdpRunning && (wifiConnected || apModeActive)) {
    if (pqUdp.begin(PQ_UDP_PORT)) {
      pqUdpRunning = true;
      ensureMDNS();
      MDNS.addService("pequod", "udp", PQ_UDP_PORT);
      Serial.printf("[PQ] listening for PEQUOD on UDP %d\n", PQ_UDP_PORT);
    }
  }
  if (pqUdpRunning) {
    static uint8_t buf[1500];
    for (int k = 0; k < 8; k++) {
      int sz = pqUdp.parsePacket();
      if (sz <= 0) break;
      int n = pqUdp.read(buf, sizeof(buf));
      if (n >= 8 && buf[0] == 'P' && buf[1] == 'Q' && buf[2] == 'L' && buf[3] == '1') {
        pqUdpHost = pqUdp.remoteIP();
        pqUdpHostPort = pqUdp.remotePort();
        pqOnFrame(PQ_UDP, buf + 4, n - 4);
      }
    }
  }

  // Time out a silent host.
  const unsigned long now = millis();
  bool before = pqHostConnected();
  for (int tr = 0; tr < 2; tr++)
    if (pqLink[tr].active && now - pqLink[tr].lastRx > PQ_TIMEOUT_MS) pqLink[tr].active = false;
  if (before && !pqHostConnected()) pqReleaseHost();
  if (!pqHostConnected()) return;

  // Layer changes (shift, encoder, web page) are reported so PEQUOD shows the right page.
  static int lastLayer[8] = {-1, -1, -1, -1, -1, -1, -1, -1};
  static unsigned long lastLayerCheck = 0;
  if (now - lastLayerCheck > 50) {
    lastLayerCheck = now;
    bool changed = false;
    for (int s = 0; s < 8; s++) {
      int L = getSlaveInfoForSlot(s) ? getSlotActiveLayer(s) : -1;
      if (L != lastLayer[s]) { lastLayer[s] = L; changed = true; }
    }
    if (changed) {
      StaticJsonDocument<256> d;
      d["t"] = "layer"; d["l"] = getCurrentActiveLayer();
      JsonArray a = d.createNestedArray("s");
      for (int s = 0; s < 8; s++) a.add(lastLayer[s]);
      pqSendEventDoc(d);
    }
  }

  // Pedal calibration readout.
  static unsigned long lastCal = 0;
  if (now - lastCal > 66) {
    lastCal = now;
    for (int s = 0; s < 8; s++) {
      if (!pqCalWatch[s]) continue;
      StaticJsonDocument<128> d;
      d["t"] = "raw"; d["s"] = s; d["raw"] = pedalRawReadouts[s].rawValue; d["v"] = pedalRawReadouts[s].calibratedValue;
      pqSendEventDoc(d);
    }
  }

  // "Identify": blink the chosen LED white three times.
  if (pqIdentify.slot >= 0) {
    unsigned long t = now - pqIdentify.start;
    int s = pqIdentify.slot, b = pqIdentify.btn;
    if (t > 1200 || s > 7 || b < 0 || b > 3) {
      if (s <= 7 && b >= 0 && b <= 3 && !pqOwnedOverride[s][b]) ledOverrideOn[s][b] = false;
      pqIdentify.slot = -1;
    } else {
      ledOverrideOn[s][b] = true;
      ledOverrideColor[s][b] = ((t / 200) % 2 == 0) ? CRGB::White : CRGB::Black;
    }
  }

  // Screen: redraw on change (tuner at most ~8x a second; each redraw holds the display bus ~25 ms).
  if (pqScreen.valid) {
    bool flashEnded = pqScreen.flash.length() && now >= pqScreen.flashUntil;
    if (flashEnded) { pqScreen.flash = ""; pqScreen.dirty = true; }
    unsigned long minGap = pqScreen.mode == "tuner" ? 120 : 40;
    if (pqScreen.dirty && now - pqScreen.lastDraw >= minGap) {
      pqScreen.dirty = false;
      pqScreen.lastDraw = now;
      pqDrawScreen();
    }
  }
}
