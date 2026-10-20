/* =========================
   ESP32 Master: I2C Button → MIDI (UART)
   Wi-Fi Config + Web UI + AJAX + Live Status
   Dynamic device cards, settings editable inline
   Live button updates with blur border selection
   CUSTOMIZABLE COLOR THEME + FUNCTIONAL MENU SYSTEM
   CLEANED VERSION - NO LEGACY ARRAYS - FIXED JAVASCRIPT
   Version: 1.3.1 - FIXED DUPLICATE PEDAL CODE + ENHANCED UI
   Version: 1.4.0 - PEQUOD LINK (native connection to PEQUOD Host, see PequodLink.h)
   ========================= */

#include <Wire.h>
#include <WiFi.h>
#include <WiFiUdp.h>
#include <ESPmDNS.h>
#include <WebServer.h>
#include <Preferences.h>

#include <esp_heap_caps.h>
#include "SPIFFS.h"
#include <ArduinoJson.h>

// Include FastLED first
#include <FastLED.h>

// Screen support
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>

// USB MIDI Support - include after FastLED
#include <Control_Surface.h>
#include "USB.h"

#include "base64.h"


// Resolve naming conflicts by using explicit namespacing
namespace CS = cs;  // Control_Surface uses 'cs' namespace

// Create alias for FastLED's Selectable to avoid ambiguity
using FastLEDSelectable = ::Selectable;

// USB MIDI Interface using Control_Surface namespace
CS::USBMIDI_Interface usbmidi;
struct SlaveInfo;
struct DeviceConfig;

// ── OSC types (defined early so Arduino's auto-prototypes can see them) ──
struct OscArg { char t; int32_t i; float f; String s; };
// ── message builder ─────────────────────────────────────────────────────────
class OscMsg {
public:
  String addr, txt;
  char tags[10]; int nt = 0;
  uint8_t args[112]; int al = 0;
  explicit OscMsg(const String& a) : addr(a), txt(a) {}
  OscMsg& i(int32_t v) {
    if (nt < 8 && al + 4 <= (int)sizeof(args)) { tags[nt++] = 'i'; put32((uint32_t)v); txt += " " + String(v); }
    return *this;
  }
  OscMsg& f(float v) {
    if (nt < 8 && al + 4 <= (int)sizeof(args)) { uint32_t u; memcpy(&u, &v, 4); tags[nt++] = 'f'; put32(u); txt += " " + String(v, 3); }
    return *this;
  }
  OscMsg& str(const String& v) {
    int need = ((v.length() + 4) & ~3);
    if (nt < 8 && al + need <= (int)sizeof(args)) {
      tags[nt++] = 's';
      for (unsigned k = 0; k < v.length(); k++) args[al++] = v[k];
      args[al++] = 0; while (al % 4) args[al++] = 0;
      txt += " \"" + v + "\"";
    }
    return *this;
  }
  int build(uint8_t* out, int cap) {
    int n = 0;
    for (unsigned k = 0; k < addr.length() && n < 120; k++) out[n++] = addr[k];
    out[n++] = 0; while (n % 4) out[n++] = 0;
    out[n++] = ',';
    for (int k = 0; k < nt; k++) out[n++] = tags[k];
    out[n++] = 0; while (n % 4) out[n++] = 0;
    if (n + al > cap) return 0;
    memcpy(out + n, args, al);
    return n + al;
  }
private:
  void put32(uint32_t u) { args[al++] = u >> 24; args[al++] = u >> 16; args[al++] = u >> 8; args[al++] = u; }
};

// ── PEQUOD Link hooks (implemented in PequodLink.h) ─────────────────────────
void pqOnButton(int slot, int btnIdx, bool pressed);
void pqOnPedal(int slot, int v127, int raw);
void pqOnEncoder(int slot, int delta);
void pqOnEncoderButton(int slot, bool pressed);
bool pqHostConnected();
bool pqOwnsDisplay();
void pqSetup();
void pqLoop();

// Move these helper functions BEFORE the SPIFFSConfig class
String idToHex(uint64_t id) {
  char buf[17];
  sprintf(buf, "%016llX", id);
  return String(buf);
}

String idToShortKey(uint64_t id) {
  return idToHex(id).substring(8);
}

// MIDI output selection
enum MidiOutputMode {
  MIDI_UART_ONLY = 0,
  MIDI_USB_ONLY = 1,
  MIDI_BOTH = 2
};

MidiOutputMode midiOutputMode = MIDI_BOTH;  // Default to both outputs

// First: Forward declarations and constants
constexpr int TYPE_MASTER_MIN = 0;
constexpr int TYPE_MASTER_MAX = 19;
constexpr int TYPE_FOOTSWITCH_MIN = 20;
constexpr int TYPE_FOOTSWITCH_MAX = 39;
constexpr int TYPE_PEDAL_MIN = 40;
constexpr int TYPE_PEDAL_MAX = 59;
constexpr int TYPE_BUTTON_MIN = 60;
constexpr int TYPE_BUTTON_MAX = 79;
constexpr int TYPE_KNOB_MIN = 80;
constexpr int TYPE_KNOB_MAX = 99;
constexpr int TYPE_ENCODER_MIN = 100;
constexpr int TYPE_ENCODER_MAX = 119;

// Second: Complete struct definitions
struct DeviceConfig {
  // Original single-layer config (for backward compatibility)
  int midiMapping[4];
  String midiType[4];
  int midiChannel[4];

  // Multi-layer configuration (up to 4 layers)
  int layerMidiMapping[4][4];  // [layer][button]
  String layerMidiType[4][4];
  int layerMidiChannel[4][4];
  String layerMidiBehavior[4][4];
  // OSC action per layer/button (used when layerMidiType == "osc")
  String layerOscAddr[4][4];   // e.g. "/looper/record"
  float  layerOscOn[4][4];     // value sent on press / latch on / pedal max
  float  layerOscOff[4][4];    // value sent on release / latch off / pedal min
  String layerOscArg[4][4];    // "f" float, "i" int, "none" no argument

  // Device control configuration
  String deviceControlType[4];
  int deviceControlTarget[4];
  String deviceControlMode[4];

  // LED configuration (per button)
  String ledMode[4];           // "button_state", "midi_in", "layer", "off"
  String ledColor[4];          // Hex color like "#FF0000"
  int ledMidiNote[4];          // For midi_in mode
  int ledMidiChannel[4];       // For midi_in mode
  bool ledFollowLayer[4];      // If true, use layer colors
  String ledOscAddr[4];        // For osc_in mode: incoming OSC address that drives the LED

  // Shift button configuration
  int shiftButton;
  bool shiftMode;
  bool shiftEnabled;

  // Pedal calibration
  int pedalCalMin;
  int pedalCalMax;
  bool pedalInvert;
  int preferredSlot;


// Add these fields to the existing encoder configuration in DeviceConfig
String encoderRotationMode;      // "layer_device", "layer_global", "midi_cc", "midi_note"
int encoderNoteNumber;           // NEW: For MIDI note mode
int encoderNoteChannel;          // NEW: For MIDI note mode
int encoderMidiCC;
int encoderMidiChannel;
int encoderMinValue;
int encoderMaxValue;
bool encoderInvertDirection;
String encoderButtonMode;
int encoderButtonMidiNote;
int encoderButtonMidiChannel;
int encoderStepsPerDetent;
String encoderOscAddr;           // rotation mode "osc"
String encoderOscMode;           // "relative" (i ±steps) or "absolute" (f 0..1)
String encoderButtonOscAddr;     // button mode "osc" (i 1|0)
};

// Use a unique name to avoid conflicts with Control_Surface library
struct I2CEncoderState {
  int position = 0;
  int lastPosition = 0;
  bool buttonPressed = false;
  bool lastButtonState = false;
  unsigned long lastUpdateTime = 0;
};

// Per-slot encoder states
I2CEncoderState encoderStates[8];



struct SlaveInfo {
  uint8_t i2cAddress;
  uint64_t uniqueID;
  uint8_t type;
  unsigned long lastFailTime;
  uint8_t lastState;
  uint8_t prevState;
  DeviceConfig config;
};


// Global layer control settings
struct GlobalLayerControl {
  bool enabled = false;        // Enable global layer switching
  int currentGlobalLayer = 0;  // Current global layer (0-3)
} globalLayerControl;

// Forward declarations for functions
void updateDeviceLED(int slot, int button, bool outputState);
void handleDeviceControl(int slot, int button, bool pressed, DeviceConfig& config);
void updateLayerLED(int activeLayer);
void updateAllDeviceLayerLEDs(int layerIndex);
void handleI2CEncoderRotation(int slot, int detents, DeviceConfig& config);
void handleI2CEncoderButton(int slot, bool pressed, DeviceConfig& config);
void setAllDeviceLEDs(uint8_t i2cAddress, CRGB leds[5]);
void oscMirrorMidiOut(bool isNote, uint8_t ch, uint8_t d1, uint8_t d2);
void oscOnMidiIn(uint8_t type, uint8_t ch, uint8_t d1, uint8_t d2);
bool oscSendAction(const String& addr, const String& argType, float val);
void midi2NetNoteOn(uint8_t ch, uint8_t note, uint8_t vel);
void midi2NetNoteOff(uint8_t ch, uint8_t note, uint8_t vel);
void midi2NetCC(uint8_t ch, uint8_t cc, uint8_t val);
void startMidi2Network();
void stopMidi2Network();
void saveDeviceConfigWithLayers(int slaveIndex);




String wifiPrimarySSID, wifiPrimaryPass;
String wifiFallbackSSID, wifiFallbackPass;
String wifiHostname;



String THEME_COLOR = "#f5b227";
CRGB hexToRGB(String hexColor) {
  // Remove # if present
  if (hexColor.startsWith("#")) {
    hexColor = hexColor.substring(1);
  }
  
  // Convert hex string to RGB values
  long rgb = strtol(hexColor.c_str(), NULL, 16);
  uint8_t r = (rgb >> 16) & 0xFF;
  uint8_t g = (rgb >> 8) & 0xFF;
  uint8_t b = rgb & 0xFF;
  
  return CRGB(r, g, b);
}

// Layer color cache - avoids opening SPIFFS on every LED update call
// Must be declared before getCustomLayerColor() which references them
CRGB cachedLayerColors[4] = {CRGB(0,0,0), CRGB(0,0,0), CRGB(0,0,0), CRGB(0,0,0)};
bool layerColorCacheValid = false;
void invalidateLayerColorCache() { layerColorCacheValid = false; }

CRGB getCustomLayerColor(int layer) {
  if (layer < 0 || layer >= 4) return CRGB::Black;
  if (!layerColorCacheValid) {
    const CRGB defaultColors[4] = {CRGB(244,67,54),CRGB(156,39,176),CRGB(33,150,243),CRGB(76,175,80)};
    for (int i = 0; i < 4; i++) cachedLayerColors[i] = defaultColors[i];
    if (SPIFFS.exists("/layer_colors.json")) {
      File file = SPIFFS.open("/layer_colors.json", "r");
      if (file) {
        DynamicJsonDocument doc(512);
        if (!deserializeJson(doc, file)) {
          for (int i = 0; i < 4; i++) {
            String key = "layer" + String(i);
            if (doc.containsKey(key)) cachedLayerColors[i] = hexToRGB(doc[key].as<String>());
          }
        }
        file.close();
      }
    }
    layerColorCacheValid = true;
  }
  return cachedLayerColors[layer];
}


// Number of active layers — loaded from NVS, kept in sync with web UI preference
int configNumLayers = 4;
// Track when button LEDs were last updated to prevent conflicts
static unsigned long lastButtonLEDUpdate[8][4] = {{0}};
// Desired LED on/off state per button
bool buttonLEDDesiredState[8][4] = {{false}};
// LED state driven by incoming MIDI (midi_in mode)
bool midiInLEDState[8][4] = {{false}};
// Timestamp of last MIDI output for midi_out_triggered flash mode (0 = not active)
unsigned long midiOutTriggerTime[8][4] = {{0}};




bool advancedOptionsEnabled = false;
bool midi2Enabled = false;  // MIDI 2.0 UMP over TCP (set by startMidi2Network)
bool midi2Pref    = false;  // loaded from NVS on boot


String serialBuffer = "";
const int MAX_SERIAL_BUFFER = 5000;  // Max characters to keep
bool webSerialEnabled = false;

class WebSerial {
private:
  String buffer;
  const int maxSize = 5000;
  
public:
  void print(const String& text) {
    buffer += text;
    if (buffer.length() > maxSize) {
      buffer = buffer.substring(buffer.length() - maxSize);
    }
    if (webSerialEnabled) {
      Serial.print(text);  // Still output to hardware serial
    }
  }
  
  void println(const String& text) {
    print(text + "\n");
  }
  
  void printf(const char* format, ...) {
    char temp[256];
    va_list args;
    va_start(args, format);
    vsnprintf(temp, sizeof(temp), format, args);
    va_end(args);
    print(String(temp));
  }
  
  String getBuffer() {
    return buffer;
  }
  
  void clear() {
    buffer = "";
  }
};

WebSerial webSerial;










// Add this SPIFFSConfig class BEFORE your existing global variables
class SPIFFSConfig {
private:
  bool ensureSPIFFS() {
    if (!SPIFFS.begin(true)) {
      Serial.println("SPIFFS Mount Failed");
      return false;
    }
    return true;
  }

  bool saveJSON(const char* path, DynamicJsonDocument& doc) {
    if (!ensureSPIFFS()) return false;

    File file = SPIFFS.open(path, "w");
    if (!file) {
      Serial.printf("Failed to open file for writing: %s\n", path);
      return false;
    }

    size_t bytesWritten = serializeJson(doc, file);
    file.close();

    Serial.printf("Saved %d bytes to %s\n", bytesWritten, path);
    return bytesWritten > 0;
  }

  bool loadJSON(const char* path, DynamicJsonDocument& doc) {
    if (!ensureSPIFFS()) return false;

    if (!SPIFFS.exists(path)) {
      Serial.printf("Config file doesn't exist: %s\n", path);
      return false;
    }

    File file = SPIFFS.open(path, "r");
    if (!file) {
      Serial.printf("Failed to open file for reading: %s\n", path);
      return false;
    }

    DeserializationError error = deserializeJson(doc, file);
    file.close();

    if (error) {
      Serial.printf("JSON parse error in %s: %s\n", path, error.c_str());
      return false;
    }

    return true;
  }

public:

  void listConfigs() {
    if (!ensureSPIFFS()) return;

    Serial.println("=== SPIFFS Configuration Files ===");
    File root = SPIFFS.open("/");
    File file = root.openNextFile();

    while (file) {
      String fileName = file.name();
      size_t fileSize = file.size();
      Serial.printf("  %s (%d bytes)\n", fileName.c_str(), fileSize);
      file.close();
      file = root.openNextFile();
    }
    root.close();
    Serial.println("=== End Configuration Files ===");
  }

  // Save device configuration
  bool saveDeviceConfig(int slaveIndex, const SlaveInfo& slave) {
    String deviceKey = idToShortKey(slave.uniqueID);
    String filename = "/device_" + deviceKey + ".json";

    DynamicJsonDocument doc(12288);  // sized for MIDI + OSC fields on 4 layers x 4 buttons

    // Basic device info
    doc["deviceId"] = deviceKey;
    doc["i2cAddress"] = slave.i2cAddress;
    doc["type"] = slave.type;

    // Shift configuration
    doc["shift"]["button"] = slave.config.shiftButton;
    doc["shift"]["mode"] = slave.config.shiftMode;
    doc["shift"]["enabled"] = slave.config.shiftEnabled;

    // Layer configurations
    JsonArray layers = doc.createNestedArray("layers");
    for (int layer = 0; layer < 4; layer++) {
      JsonObject layerObj = layers.createNestedObject();
      JsonArray buttons = layerObj.createNestedArray("buttons");

      for (int btn = 0; btn < 4; btn++) {
        JsonObject btnObj = buttons.createNestedObject();
        btnObj["mapping"] = slave.config.layerMidiMapping[layer][btn];
        btnObj["type"] = slave.config.layerMidiType[layer][btn];
        btnObj["channel"] = slave.config.layerMidiChannel[layer][btn];
        btnObj["behavior"] = slave.config.layerMidiBehavior[layer][btn];
        btnObj["oscAddr"] = slave.config.layerOscAddr[layer][btn];
        btnObj["oscOn"] = slave.config.layerOscOn[layer][btn];
        btnObj["oscOff"] = slave.config.layerOscOff[layer][btn];
        btnObj["oscArg"] = slave.config.layerOscArg[layer][btn];
      }
    }

    // Device control settings
    JsonArray deviceControls = doc.createNestedArray("deviceControl");
    for (int btn = 0; btn < 4; btn++) {
      JsonObject dcObj = deviceControls.createNestedObject();
      dcObj["type"] = slave.config.deviceControlType[btn];
      dcObj["target"] = slave.config.deviceControlTarget[btn];
      dcObj["mode"] = slave.config.deviceControlMode[btn];
    }

    // Pedal calibration
    if (slave.type >= TYPE_PEDAL_MIN && slave.type <= TYPE_PEDAL_MAX) {
      doc["pedal"]["calMin"] = slave.config.pedalCalMin;
      doc["pedal"]["calMax"] = slave.config.pedalCalMax;
      doc["pedal"]["invert"] = slave.config.pedalInvert;
    }
// LED configuration
JsonArray leds = doc.createNestedArray("leds");
for (int btn = 0; btn < 4; btn++) {
  JsonObject ledObj = leds.createNestedObject();
  ledObj["mode"] = slave.config.ledMode[btn];
  ledObj["color"] = slave.config.ledColor[btn];
  ledObj["midiNote"] = slave.config.ledMidiNote[btn];
  ledObj["midiChannel"] = slave.config.ledMidiChannel[btn];
  ledObj["followLayer"] = slave.config.ledFollowLayer[btn];
  ledObj["oscAddr"] = slave.config.ledOscAddr[btn];
}
    if (slave.type >= TYPE_ENCODER_MIN && slave.type <= TYPE_ENCODER_MAX) {
  JsonObject encoder = doc.createNestedObject("encoder");
  encoder["rotationMode"] = slave.config.encoderRotationMode;
  encoder["midiCC"] = slave.config.encoderMidiCC;
  encoder["midiChannel"] = slave.config.encoderMidiChannel;
  encoder["minValue"] = slave.config.encoderMinValue;
  encoder["maxValue"] = slave.config.encoderMaxValue;
  encoder["invertDirection"] = slave.config.encoderInvertDirection;
  encoder["buttonMode"] = slave.config.encoderButtonMode;
  encoder["buttonMidiNote"] = slave.config.encoderButtonMidiNote;
  encoder["buttonMidiChannel"] = slave.config.encoderButtonMidiChannel;
  encoder["stepsPerDetent"] = slave.config.encoderStepsPerDetent;
  encoder["noteNumber"] = slave.config.encoderNoteNumber;
encoder["noteChannel"] = slave.config.encoderNoteChannel;
  encoder["oscAddr"] = slave.config.encoderOscAddr;
  encoder["oscMode"] = slave.config.encoderOscMode;
  encoder["btnOscAddr"] = slave.config.encoderButtonOscAddr;
}

    doc["preferredSlot"] = slave.config.preferredSlot;

    return saveJSON(filename.c_str(), doc);
  }

  // Load device configuration
  bool loadDeviceConfig(int slaveIndex, SlaveInfo& slave) {
    String deviceKey = idToShortKey(slave.uniqueID);
    String filename = "/device_" + deviceKey + ".json";

    DynamicJsonDocument doc(12288);
    if (!loadJSON(filename.c_str(), doc)) {
      return false;  // Caller should initialize defaults
    }

    // Load shift configuration
    if (doc.containsKey("shift")) {
      slave.config.shiftButton = doc["shift"]["button"] | -1;
      slave.config.shiftMode = doc["shift"]["mode"] | true;
      slave.config.shiftEnabled = doc["shift"]["enabled"] | false;
    }

    // Load layer configurations
    if (doc.containsKey("layers")) {
      JsonArray layers = doc["layers"];
      for (int layer = 0; layer < 4 && layer < layers.size(); layer++) {
        JsonArray buttons = layers[layer]["buttons"];
        for (int btn = 0; btn < 4 && btn < buttons.size(); btn++) {
          slave.config.layerMidiMapping[layer][btn] = buttons[btn]["mapping"] | (60 + layer * 16 + btn);
          slave.config.layerMidiType[layer][btn] = buttons[btn]["type"] | "note";
          slave.config.layerMidiChannel[layer][btn] = buttons[btn]["channel"] | 1;
          slave.config.layerMidiBehavior[layer][btn] = buttons[btn]["behavior"] | "momentary";
          if (buttons[btn].containsKey("oscAddr")) slave.config.layerOscAddr[layer][btn] = buttons[btn]["oscAddr"].as<String>();
          slave.config.layerOscOn[layer][btn]  = buttons[btn]["oscOn"]  | slave.config.layerOscOn[layer][btn];
          slave.config.layerOscOff[layer][btn] = buttons[btn]["oscOff"] | slave.config.layerOscOff[layer][btn];
          if (buttons[btn].containsKey("oscArg")) slave.config.layerOscArg[layer][btn] = buttons[btn]["oscArg"].as<String>();
        }
      }
    }

    // Load device control settings
    if (doc.containsKey("deviceControl")) {
      JsonArray deviceControls = doc["deviceControl"];
      for (int btn = 0; btn < 4 && btn < deviceControls.size(); btn++) {
        slave.config.deviceControlType[btn] = deviceControls[btn]["type"] | "none";
        slave.config.deviceControlTarget[btn] = deviceControls[btn]["target"] | 0;
        slave.config.deviceControlMode[btn] = deviceControls[btn]["mode"] | "momentary";
      }
    }

    // Load pedal calibration
    if (doc.containsKey("pedal") && slave.type >= TYPE_PEDAL_MIN && slave.type <= TYPE_PEDAL_MAX) {
      slave.config.pedalCalMin = doc["pedal"]["calMin"] | 0;
      slave.config.pedalCalMax = doc["pedal"]["calMax"] | 4095;
      slave.config.pedalInvert = doc["pedal"]["invert"] | false;
    }
// Load LED configuration
if (doc.containsKey("leds")) {
  JsonArray leds = doc["leds"];
  for (int btn = 0; btn < 4 && btn < leds.size(); btn++) {
    slave.config.ledMode[btn] = leds[btn]["mode"] | "button_state";
    slave.config.ledColor[btn] = leds[btn]["color"] | "#FFFFFF";
    slave.config.ledMidiNote[btn] = leds[btn]["midiNote"] | (60 + btn);
    slave.config.ledMidiChannel[btn] = leds[btn]["midiChannel"] | 1;
    slave.config.ledFollowLayer[btn] = leds[btn]["followLayer"] | true;
    if (leds[btn].containsKey("oscAddr")) slave.config.ledOscAddr[btn] = leds[btn]["oscAddr"].as<String>();
  }
}

    if (doc.containsKey("encoder") && slave.type >= TYPE_ENCODER_MIN && slave.type <= TYPE_ENCODER_MAX) {
  JsonObject encoder = doc["encoder"];
  slave.config.encoderRotationMode = encoder["rotationMode"] | "layer_global";
  slave.config.encoderMidiCC = encoder["midiCC"] | 1;
  slave.config.encoderMidiChannel = encoder["midiChannel"] | 1;
  slave.config.encoderMinValue = encoder["minValue"] | 0;
  slave.config.encoderMaxValue = encoder["maxValue"] | 127;
  slave.config.encoderInvertDirection = encoder["invertDirection"] | false;
  slave.config.encoderButtonMode = encoder["buttonMode"] | "layer_toggle";
  slave.config.encoderButtonMidiNote = encoder["buttonMidiNote"] | 60;
  slave.config.encoderButtonMidiChannel = encoder["buttonMidiChannel"] | 1;
  slave.config.encoderStepsPerDetent = encoder["stepsPerDetent"] | 4;
  slave.config.encoderNoteNumber = encoder["noteNumber"] | 60;
slave.config.encoderNoteChannel = encoder["noteChannel"] | 1;
  if (encoder.containsKey("oscAddr"))    slave.config.encoderOscAddr = encoder["oscAddr"].as<String>();
  if (encoder.containsKey("oscMode"))    slave.config.encoderOscMode = encoder["oscMode"].as<String>();
  if (encoder.containsKey("btnOscAddr")) slave.config.encoderButtonOscAddr = encoder["btnOscAddr"].as<String>();
}

    slave.config.preferredSlot = doc["preferredSlot"] | -1;

    return true;
  }

  // Save WiFi configuration
  bool saveWiFiConfig(const String& primarySSID, const String& primaryPass,
                      const String& fallbackSSID, const String& fallbackPass,
                      const String& hostname) {
    DynamicJsonDocument doc(1024);
    doc["primarySSID"] = primarySSID;
    doc["primaryPass"] = primaryPass;
    doc["fallbackSSID"] = fallbackSSID;
    doc["fallbackPass"] = fallbackPass;
    doc["hostname"] = hostname;

    return saveJSON("/wifi.json", doc);
  }

  // Load WiFi configuration
  bool loadWiFiConfig(String& primarySSID, String& primaryPass,
                      String& fallbackSSID, String& fallbackPass,
                      String& hostname) {
    DynamicJsonDocument doc(1024);
    if (!loadJSON("/wifi.json", doc)) {
      primarySSID = "";
      primaryPass = "";
      fallbackSSID = "";
      fallbackPass = "";
      hostname = "ESP32_MIDI";
      return false;
    }

    primarySSID = doc["primarySSID"] | "";
    primaryPass = doc["primaryPass"] | "";
    fallbackSSID = doc["fallbackSSID"] | "";
    fallbackPass = doc["fallbackPass"] | "";
    hostname = doc["hostname"] | "ESP32_MIDI";

    return true;
  }

  // Save UI settings
  bool saveUIConfig(const String& themeColor, bool advancedEnabled, MidiOutputMode outputMode = MIDI_BOTH) {
  DynamicJsonDocument doc(512);
  doc["themeColor"] = themeColor;
  doc["advancedEnabled"] = advancedEnabled;
  doc["midiOutputMode"] = (int)outputMode;

  return saveJSON("/ui.json", doc);
}

// Load UI settings including MIDI output mode
bool loadUIConfig(String& themeColor, bool& advancedEnabled, MidiOutputMode& outputMode) {
  DynamicJsonDocument doc(512);
  if (!loadJSON("/ui.json", doc)) {
    themeColor = "#4CAF50";
    advancedEnabled = false;
    outputMode = MIDI_BOTH;
    return false;
  }

  themeColor = doc["themeColor"] | "#4CAF50";
  advancedEnabled = doc["advancedEnabled"] | false;
  outputMode = (MidiOutputMode)(doc["midiOutputMode"] | MIDI_BOTH);

  return true;
}

  // Save global layer control
  bool saveGlobalConfig(bool enabled, int currentLayer) {
    DynamicJsonDocument doc(256);
    doc["globalLayerEnabled"] = enabled;
    doc["currentGlobalLayer"] = currentLayer;

    return saveJSON("/global.json", doc);
  }

  // Load global layer control
  bool loadGlobalConfig(bool& enabled, int& currentLayer) {
    DynamicJsonDocument doc(256);
    if (!loadJSON("/global.json", doc)) {
      enabled = false;
      currentLayer = 0;
      return false;
    }

    enabled = doc["globalLayerEnabled"] | false;
    currentLayer = doc["currentGlobalLayer"] | 0;

    return true;
  }

  // Factory reset
  bool factoryReset() {
    if (!ensureSPIFFS()) return false;

    Serial.println("Performing SPIFFS factory reset...");

    File root = SPIFFS.open("/");
    File file = root.openNextFile();

    while (file) {
      String fileName = file.name();
      file.close();

      if (fileName.endsWith(".json")) {
        Serial.printf("Deleting: %s\n", fileName.c_str());
        SPIFFS.remove(fileName);
      }

      file = root.openNextFile();
    }
    root.close();

    return true;
  }
};

SPIFFSConfig spiffsConfig;


TwoWire lcd = TwoWire(1);





// I2C Config
constexpr int MAX_SLAVES = 16;
constexpr int SDA_PIN = 8;
constexpr int SCL_PIN = 18;

// MIDI UART
#define MIDI_UART_TX 17
#define MIDI_UART_RX 16

//LED
#define LED_PIN 48          // ESP32-S3 onboard LED pin (adjust if different)
#define NUM_LEDS 1          // Just one onboard LED
#define LED_BRIGHTNESS 255  // Adjust brightness (0-255)
CRGB leds[NUM_LEDS];

// Global LED brightness (5-255). Applied to the onboard layer LED via
// FastLED.setBrightness() and to every color transmitted to slave devices,
// so slave firmware needs no changes. Persisted in NVS ("ui"/"ledBright").
uint8_t globalLEDBrightness = 255;
static inline CRGB scaleForBrightness(CRGB c) {
  CRGB out = c;
  out.nscale8_video(globalLEDBrightness);  // _video keeps dim colors from going fully black
  return out;
}

// Layer color definitions (matching your CSS colors)
const CRGB LAYER_COLORS[4] = {
  CRGB(244, 67, 54),   // Layer 1: Red (#F44336)
  CRGB(156, 39, 176),  // Layer 2: Purple (#9C27B0)
  CRGB(33, 150, 243),  // Layer 3: Blue (#2196F3)
  CRGB(76, 175, 80)    // Layer 4: Green (#4CAF50)
};

// Screen
#define SCREEN_WIDTH 128
#define SCREEN_HEIGHT 64
#define OLED_ADDR 0x3C
Adafruit_SSD1306 display(SCREEN_WIDTH, SCREEN_HEIGHT, &lcd, -1);

// Global shift state tracking
struct ShiftState {
  bool isShiftPressed;
  bool shiftToggled;
  int currentLayer;
  unsigned long shiftPressTime;
} shiftStates[8];  // One per slot




// Pedal raw data structure
struct PedalRawData {
  int rawValue = 0;
  int calibratedValue = 0;
  unsigned long lastUpdate = 0;
};
PedalRawData pedalRawReadouts[8];  // Store raw data for up to 8 pedal slots

SlaveInfo detectedSlaves[MAX_SLAVES];
int numDetected = 0;
int deviceMapping[8] = { -1, -1, -1, -1, -1, -1, -1, -1 };

WebServer server(80);


#define MAX_DEVICES 16             // Limit number of devices
#define MAX_DEVICE_NAME_LENGTH 32  // Limit device name strings
#define MAX_JSON_BUFFER_SIZE 1024  // Limit JSON buffer size
#define MIN_FREE_HEAP 8192         // Minimum free heap threshold
#define HEAP_CHECK_INTERVAL 10000  // Check heap every 10 seconds

// Memory monitoring structure (REQUIRED)
struct MemoryStats {
  size_t freeHeap;
  size_t minFreeHeap;
  size_t maxAllocHeap;
  uint32_t heapFragmentation;
  unsigned long lastCheck;
} memStats = { 0 };  // Initialize to zero

// Stack overflow protection array (REQUIRED)
uint32_t stackCanary[8] = { 0xDEADBEEF, 0xDEADBEEF, 0xDEADBEEF, 0xDEADBEEF,
                            0xDEADBEEF, 0xDEADBEEF, 0xDEADBEEF, 0xDEADBEEF };

// Timing variables for memory checks (REQUIRED)
unsigned long lastMemoryCheck = 0;
unsigned long lastHeapReport = 0;



Preferences prefs;

// Wi-Fi management
enum WiFiState { WIFI_PRIMARY,
                 WIFI_FALLBACK,
                 WIFI_AP_MODE,
                 WIFI_DONE } wifiState;
unsigned long wifiStartTime = 0;
bool wifiConnected = false;
bool apModeActive = false;
bool webServerStarted = false;


String scannedSSIDs[32];
int numScannedSSIDs = 0;


bool anyButtonPressed = false;  // Track if any button has been pressed since startup
unsigned long startupTime = 0;  // Track startup time




const char HTML_TEMPLATE[] PROGMEM = R"rawliteral(
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>I2C MIDI Dashboard</title>
    <meta name='viewport' content='width=device-width, initial-scale=1'>
    <link href='https://fonts.googleapis.com/css2?family=Roboto:wght@400;500;600&display=swap' rel='stylesheet'>
    <style>.material-icons{font-style:normal;font-size:20px;line-height:1;display:inline-block;vertical-align:middle;}</style>
    
    <script>
        let currentDevice=-1,currentButton=-1,currentIsPedal=false;
let deviceConfigs={};
let wifiPrimarySSID='%WIFI_PRIMARY_SSID%';
let rawUpdateInterval=null;
let selectedSlot = -1;
let selectedButton = -1;
let selectedIsPedal = false;
let selectedIsEncoder = false;
let selectedLED = -1;
let currentLayer = 0;

// ── Auto-save ─────────────────────────────────────────────
let autoSaveTimer = null;
let lastSelectTime = 0;
function scheduleAutoSave() {
    // Suppress saves triggered by programmatic form population right after a selection
    if (Date.now() - lastSelectTime < 800) return;
    if (autoSaveTimer) clearTimeout(autoSaveTimer);
    autoSaveTimer = setTimeout(() => { autoSaveTimer = null; saveSettings(); }, 600);
}
function flushAutoSave() {
    // Save pending edits immediately before selection/layer changes
    if (autoSaveTimer) {
        clearTimeout(autoSaveTimer);
        autoSaveTimer = null;
        saveSettings();
    }
}
function showSavedFeedback() {
    const fb = document.getElementById('saveFeedback');
    if (fb) { fb.classList.add('visible'); setTimeout(() => fb.classList.remove('visible'), 1200); }
}

// ── Select by press: tapping a physical button selects it in the UI ──
let sbpInterval = null;
let sbpPrevStates = {};
function toggleSelectByPress() {
    const on = document.getElementById('selectByPressToggle').checked;
    if (on && !sbpInterval) {
        sbpPrevStates = {};
        sbpInterval = setInterval(pollSelectByPress, 400);
    } else if (!on && sbpInterval) {
        clearInterval(sbpInterval);
        sbpInterval = null;
    }
}
function pollSelectByPress() {
    if (!window._lastMappings) return;
    fetch('/device_state_json').then(r => r.json()).then(states => {
        for (let slot = 0; slot < 8; slot++) {
            const m = window._lastMappings.mappings.find(x => x.slot === slot);
            if (!m || !m.deviceId) continue;
            // Only bitmask-style devices; pedal values change constantly and encoders are separate
            if (m.deviceType !== 'Footswitch' && m.deviceType !== 'Button' && m.deviceType !== 'Vertical') continue;
            const cur = states[slot] || 0;
            const prev = (slot in sbpPrevStates) ? sbpPrevStates[slot] : cur; // ignore buttons already held when enabling
            sbpPrevStates[slot] = cur;
            const rising = cur & ~prev;
            if (rising) {
                for (let b = 0; b < 4; b++) {
                    if (rising & (1 << b)) {
                        if (!(currentDevice === slot && currentButton === b && !currentIsPedal)) {
                            selectButton(slot, b);
                        }
                        return;
                    }
                }
            }
        }
    }).catch(() => {});
}

// ── Global LED brightness ─────────────────────────────────
function onLedBrightnessInput(v) {
    const lbl = document.getElementById('ledBrightLabel');
    if (lbl) lbl.textContent = Math.round((v / 255) * 100) + '%';
}
function saveLedBrightness(v) {
    fetch('/save_led_brightness?value=' + v).catch(err => console.error('Brightness save error:', err));
}
function loadLedBrightness() {
    fetch('/get_led_brightness').then(r => r.text()).then(v => {
        const el = document.getElementById('ledBrightness');
        if (el) { el.value = parseInt(v) || 255; onLedBrightnessInput(el.value); }
    }).catch(() => {});
}

// ── Mapping overview ──────────────────────────────────────
function loadMappingOverview() {
    const wrap = document.getElementById('overviewTableWrap');
    if (!wrap) return;
    wrap.innerHTML = '<p style="color:#79747E;">Loading...</p>';
    fetch('/get_full_mapping').then(r => r.json()).then(data => {
        const nL = data.numLayers || 4;
        // Count message usage for duplicate detection (mirrors what is displayed)
        const counts = {};
        data.slots.forEach(s => {
            if (s.isEncoder) return;
            (s.buttons || []).forEach((btn, bi) => {
                if (btn.dc && btn.dc !== 'none') return;
                const lays = (s.isPedal && bi === 0) ? [btn.layers[0]] : btn.layers.slice(0, nL);
                lays.forEach(l => {
                    if (l.n >= 0) { const k = l.t === 'osc' ? 'osc' + l.a : l.t + l.n + ':' + l.c; counts[k] = (counts[k] || 0) + 1; }
                });
            });
        });
        const fmt = (l) => {
            if (l.t === 'osc') {
                const dup = counts['osc' + l.a] > 1 ? ' class="dup"' : '';
                return '<span' + dup + '>OSC ' + String(l.a || '').replace(/</g, '&lt;') + (l.b === 'latching' ? ' &#9095;' : '') + '</span>';
            }
            if (l.n < 0) return '<span class="dim">&mdash;</span>';
            const cls = counts[l.t + l.n + ':' + l.c] > 1 ? ' class="dup"' : '';
            const beh = l.b === 'latching' ? ' &#9095;' : '';
            return '<span' + cls + '>' + (l.t === 'cc' ? 'CC ' : 'Note ') + l.n + ' Ch' + l.c + beh + '</span>';
        };
        let h = '<table class="map-table"><tr><th>Slot</th><th>Device</th><th>Control</th>';
        for (let l = 0; l < nL; l++) h += '<th>Layer ' + (l + 1) + '</th>';
        h += '</tr>';
        data.slots.forEach(s => {
            const devName = s.type + ' ' + s.deviceId.substring(4);
            if (s.isEncoder) {
                const e = s.encoder || {};
                const desc = e.rotationMode === 'midi_cc'   ? ('CC ' + e.midiCC + ' Ch' + e.midiChannel)
                           : e.rotationMode === 'midi_note' ? ('Note ' + e.noteNumber + ' Ch' + e.noteChannel)
                           : ('Layer scroll (' + (e.rotationMode || '').replace('layer_', '') + ')');
                h += '<tr><td>' + (s.slot + 1) + '</td><td>' + devName + '</td><td>Rotation</td><td colspan="' + nL + '">' + desc + '</td></tr>';
                return;
            }
            (s.buttons || []).forEach((btn, bi) => {
                const label = s.isPedal ? (bi === 0 ? 'Pedal' : 'Button') : ('Btn ' + (bi + 1));
                h += '<tr>';
                if (bi === 0) {
                    h += '<td rowspan="' + s.buttons.length + '">' + (s.slot + 1) + '</td>';
                    h += '<td rowspan="' + s.buttons.length + '">' + devName + '</td>';
                }
                h += '<td>' + label + (btn.shift ? ' <span class="dim">(shift)</span>' : '') + '</td>';
                if (btn.dc && btn.dc !== 'none') {
                    h += '<td colspan="' + nL + '"><span class="dim">Device control: ' + btn.dc + ' &rarr; layer ' + (btn.dcTarget + 1) + ' (' + btn.dcMode + ')</span></td>';
                } else if (s.isPedal && bi === 0) {
                    h += '<td colspan="' + nL + '">' + fmt(btn.layers[0]) + '</td>';
                } else {
                    for (let l = 0; l < nL; l++) h += '<td>' + fmt(btn.layers[l]) + '</td>';
                }
                h += '</tr>';
            });
        });
        h += '</table>';
        if (!data.slots.length) h = '<p style="color:#79747E;">No devices detected.</p>';
        wrap.innerHTML = h;
    }).catch(err => {
        wrap.innerHTML = '<p style="color:#FF8A80;">Failed to load mapping: ' + err + '</p>';
    });
}
        
        function startRawUpdates(){
            if(rawUpdateInterval)clearInterval(rawUpdateInterval);
            rawUpdateInterval=setInterval(()=>{
                if(currentDevice>=0&&currentIsPedal){
                    fetch('/get_pedal_raw?device='+currentDevice)
                    .then(r=>{
                        if(!r.ok) throw new Error('HTTP '+r.status);
                        return r.json();
                    })
                    .then(data=>{
                        const rawEl = document.getElementById('rawValue');
                        const calEl = document.getElementById('calValue');
                        const moEl  = document.getElementById('midiOut');
                        if(!data.connected){
                            if(rawEl) rawEl.innerText = 'No device';
                            if(calEl) calEl.innerText = '----';
                            if(moEl)  moEl.innerText  = '----';
                        } else {
                            if(rawEl) rawEl.innerText = data.raw;
                            if(calEl) calEl.innerText = data.calibrated;
                            if(moEl)  moEl.innerText  = data.midi;
                            if(rawEl) rawEl.style.color = data.active ? '#f5b227' : '%THEME_COLOR%';
                        }
                    })
                    .catch(err=>{
                        const rawEl = document.getElementById('rawValue');
                        if(rawEl) rawEl.innerText = 'Error';
                    });
                }
            }, 100);
        }

        function updateDeviceCards() {
    // Store current selection
    selectedSlot = currentDevice;
    selectedButton = currentButton;
    selectedIsPedal = currentIsPedal;
    // Add encoder selection tracking
    if (typeof selectedIsEncoder === 'undefined') {
        selectedIsEncoder = false;
    }
    if (typeof selectedLED === 'undefined') {
        selectedLED = -1;
    }
    
    // Get slot mappings and device states in parallel
    Promise.all([
        fetch('/get_slot_mappings').then(r => r.json()),
        fetch('/device_state_json').then(r => r.json()),
        fetch('/get_device_configs').then(r => r.json())
    ]).then(([mappings, states, configs]) => {
        window._lastMappings = mappings;
        window._lastConfigs = {};
        if(configs&&configs.devices) configs.devices.forEach(d=>{if(d&&d.slot!=null)window._lastConfigs[d.slot]=d;});
        const container = document.getElementById('deviceCardsContainer');
        container.innerHTML = ''; // clear old cards

        // Create cards for all 8 slots
        for(let slot = 0; slot < 8; slot++) {
            const slotData = mappings.mappings.find(m => m.slot === slot);
            const deviceState = states[slot] || 0;
            const deviceConfig = configs.devices.find(d => d.slot === slot);
            
            const card = document.createElement('div');
            card.className = 'slot-area';
            card.setAttribute('data-slot', slot);
            
            if(slotData && slotData.deviceId) {
                // Device is present
                const isPedal = slotData.deviceType === 'Pedal';
                const isEncoder = slotData.deviceType === 'Encoder';
                const hasPedalButton = isPedal && slotData.hasButton;
                
                card.innerHTML = `
                    <div class="slot-header">
                        <div class="slot-title">Slot ${slot + 1}</div>
                        <div class="slot-badge">${slotData.deviceType}</div>
                    </div>
                    <div class="deviceCard" draggable="true" data-device-id="${slotData.deviceId}" data-slot="${slot}">
                        <div class="device-header">
                            <div class="device-title">${slotData.deviceType} ${slotData.deviceId.substring(4)}${deviceConfig ? `<span class="dev-layer-chip" style="background:var(--layer${(deviceConfig.activeLayer||0)+1}-color)">L${(deviceConfig.activeLayer||0)+1}</span>` : ''}</div>
                            <div class="drag-handle">
                                <i class="material-icons" title="Drag">⠿</i>
                            </div>
                        </div>
                        ${isPedal ? 
    `<div class="pedal-container" onclick="selectPedal(${slot})">
        <div class="pedal-label">${deviceConfig && deviceConfig.midiType && deviceConfig.midiType[0] === 'osc' ? 'OSC' : `CC: ${deviceConfig ? deviceConfig.midiMapping[0] : 7} | Ch: ${deviceConfig ? deviceConfig.midiChannel[0] : 1}`}</div>
        <div class="pedal-slider">
            <div class="pedal-thumb" id="pedal${slot}" style="left: ${(deviceState * 100) / 127}%"></div>
        </div>
        <div class="pedal-value" id="pedalValue${slot}">${deviceState}/127</div>
    </div>
    ${hasPedalButton ? `
    <div style="margin-top:8px;">
        <div style="font-size:11px;color:#79747E;margin-bottom:4px;">Button</div>
        <div class="pads" style="width:44px;height:44px;grid-template-columns:1fr;grid-template-rows:1fr;">
            <div class="pad${selectedSlot===slot&&selectedButton===1&&!selectedIsPedal?' selected':''}"
                 id="d${slot}b1"
                 onclick="event.stopPropagation();selectButton(${slot},1)"
                 style="width:40px;height:40px;"></div>
        </div>
    </div>` : ''}` : 
isEncoder ? 
    `<div class="encoder-container" onclick="selectEncoder(${slot})">
        <div class="encoder-visual">
            <div class="encoder-knob">
                <div class="encoder-indicator"></div>
            </div>
            <div class="encoder-button ${deviceState & 0x01 ? 'pressed' : ''}"></div>
        </div>
        <div class="encoder-label">
            ${deviceConfig ? (deviceConfig.encoderRotationMode || 'layer_global') : 'layer_global'}
            ${deviceConfig && deviceConfig.encoderRotationMode === 'midi_cc' ? 
                `<br>CC: ${deviceConfig.encoderMidiCC || 1} | Ch: ${deviceConfig.encoderMidiChannel || 1}` : ''}
        </div>
    </div>` :
    `<div class="${slotData.deviceType === 'Vertical' ? 'pads-vertical' : 'pads'}">
        ${[0,1,2,3].map(btn => {
            const isActive = (deviceState & (1 << btn));
            const isSelected = (selectedSlot === slot && selectedButton === btn && !selectedIsPedal && !selectedIsEncoder && selectedLED === -1);
            const isShift = (deviceConfig && deviceConfig.shiftEnabled && deviceConfig.shiftButton === btn);
            // Small tag on buttons that switch layers (e.g. "→2", "▲", "▼")
            const _dct = deviceConfig && deviceConfig.dcType ? deviceConfig.dcType[btn] : 'none';
            const _dctg = deviceConfig && deviceConfig.dcTarget ? deviceConfig.dcTarget[btn] : 0;
            let layerDisplay = '';
            if((!_dct || _dct === 'none') && deviceConfig && deviceConfig.midiType && deviceConfig.midiType[btn] === 'osc') {
                layerDisplay = '<span class="pad-tag" title="Sends OSC">OSC</span>';
            }
            if(_dct && _dct !== 'none') {
                const _sym = _dct.indexOf('_up_') > 0 ? '&#9650;' : _dct.indexOf('_down_') > 0 ? '&#9660;' : ('&rarr;' + (_dctg + 1));
                layerDisplay = `<span class="pad-tag" title="Layer switch">${_sym}</span>`;
            }
            const layerClass = '';
            const padClass = slotData.deviceType === 'Vertical' ? 'pad-vertical' : 'pad';
            
            // Compute LED ring style for this button
            const _lmode = deviceConfig && deviceConfig.ledMode ? deviceConfig.ledMode[btn] : 'button_state';
            const _lcol  = deviceConfig && deviceConfig.ledColor ? deviceConfig.ledColor[btn] : '#FFFFFF';
            const _lfl   = !deviceConfig || deviceConfig.ledFollowLayer == null || deviceConfig.ledFollowLayer[btn] !== false;
            const _layer = deviceConfig ? (deviceConfig.activeLayer || 0) : 0;
            let _lon = false;
            if(_lmode === 'button_state') _lon = !!isActive;
            else if(_lmode === 'layer') _lon = true;
            else if(_lmode === 'off') _lon = false;
            const _rc = _lfl ? `var(--layer${_layer+1}-color)` : _lcol;
            const _ring = _lon
                ? `box-shadow:0 0 0 3px ${_rc},0 0 10px ${_rc}60;`
                : 'box-shadow:0 0 0 2px #3d3a42;';
            return `<div class="${padClass}${isActive ? ' active' : ''}${isSelected ? ' selected' : ''}${isShift ? ' shift-button' : ''}${layerClass}" 
                  id="d${slot}b${btn}" 
                  style="${_ring}" onclick="selectButton(${slot}, ${btn})">${layerDisplay}</div>`
        }).join('')}
    </div>
    <div class="${slotData.deviceType === 'Vertical' ? 'leds-vertical' : 'leds'}">
        ${[0,1,2,3].map(btn => {
            const isActive = (deviceState & (1 << btn));
            const ledSelected = (selectedSlot === slot && selectedLED === btn);
            
            // Determine LED state based on config
            let ledOn = false;
            if (deviceConfig && deviceConfig.ledMode && deviceConfig.ledMode[btn]) {
                const mode = deviceConfig.ledMode[btn];
                if (mode === 'button_state') {
                    ledOn = isActive;
                } else if (mode === 'layer') {
                    ledOn = true;
                } else if (mode === 'off') {
                    ledOn = false;
                }
                // midi_in would need additional logic
            } else {
                ledOn = isActive; // Default: follow button state
            }
            
            return ''   }).join('')}
    </div>`
}
                    </div>
                    <div class="drop-indicator"></div>
                `;
                
                // Restore encoder selection if applicable
                if(isEncoder && selectedSlot === slot && selectedIsEncoder) {
                    setTimeout(() => {
                        const encoderContainer = card.querySelector('.encoder-container');
                        if(encoderContainer) {
                            encoderContainer.style.border = '2px solid %THEME_COLOR%';
                        }
                    }, 50);
                }
                
                // Restore pedal selection if applicable
                if(isPedal && selectedSlot === slot && selectedIsPedal) {
                    setTimeout(() => {
                        const pedalContainer = card.querySelector('.pedal-container');
                        if(pedalContainer) {
                            pedalContainer.style.border = '2px solid %THEME_COLOR%';
                        }
                    }, 50);
                }
                
                // Restore LED selection if applicable
                if(!isPedal && !isEncoder && selectedSlot === slot && selectedLED >= 0) {
                    setTimeout(() => {
                        const ledIcon = card.querySelector(`#led${slot}b${selectedLED}`);
                        if(ledIcon) {
                            ledIcon.classList.add('selected');
                        }
                    }, 50);
                }
            } else {
                // Empty slot
                card.innerHTML = `
                    <div class="slot-header">
                        <div class="slot-title">Slot ${slot + 1}</div>
                        <div class="slot-badge" style="background: #49454F; color: #79747E;">Empty</div>
                    </div>
                    <div class="empty-slot">
                        <i class="material-icons" title="Empty slot">⊕</i>
                        <div class="empty-slot-text">Drag device here</div>
                    </div>
                    <div class="drop-indicator"></div>
                `;
            }
            
            container.appendChild(card);
        }
        
        // Reinitialize drag and drop after updating cards
        setTimeout(() => {
            if (typeof initializeDragAndDrop === 'function') initializeDragAndDrop();
        }, 100);
        
    }).catch(error => {
        console.error('Error updating device cards:', error);
        // Fallback to simple display
        const container = document.getElementById('deviceCardsContainer');
        container.innerHTML = '<div class="info-card"><p>Error loading devices. Please refresh the page.</p></div>';
    });
}

        // Initial update and every 2 seconds
        updateDeviceCards();
        setInterval(updateDeviceCards, 2000);

        function stopRawUpdates(){
            if(rawUpdateInterval){clearInterval(rawUpdateInterval);rawUpdateInterval=null;}
            document.getElementById('rawValue').innerText='----';
            document.getElementById('calValue').innerText='----';
            document.getElementById('midiOut').innerText='----';
        }

        function updateLCDMirror() {
    fetch('/get_lcd_content')
        .then(r => r.json())
        .then(data => {
            document.getElementById('lcdLine1').textContent = data.line1 || '';
            document.getElementById('lcdLine2').textContent = data.line2 || '';
            document.getElementById('lcdLine3').textContent = data.line3 || '';
            document.getElementById('lcdLine4').textContent = data.line4 || '';
        })
        .catch(err => {
            console.error('LCD mirror update error:', err);
            document.getElementById('lcdLine1').textContent = 'Connection Error';
        });
}

        // Function to set the current raw ADC value as the pedal's minimum calibration
        function setCurrentRawAsMin() {
            if (!currentIsPedal || currentDevice === -1 || currentButton === -1) return;

            // Get current raw value from live display
            const raw = parseInt(document.getElementById('rawValue').textContent);
            if (isNaN(raw)) return;

            // Update the min input field
            const minInput = document.getElementById('calMin');
            minInput.value = raw;

            // Enable save button for calibration
            document.getElementById('saveCalBtn').disabled = false;
        }

        // Function to set the current raw ADC value as the pedal's maximum calibration
        function setCurrentRawAsMax() {
            if (!currentIsPedal || currentDevice === -1 || currentButton === -1) return;

            // Get current raw value from live display
            const raw = parseInt(document.getElementById('rawValue').textContent);
            if (isNaN(raw)) return;

            // Update the max input field
            const maxInput = document.getElementById('calMax');
            maxInput.value = raw;

            // Enable save button for calibration
            document.getElementById('saveCalBtn').disabled = false;
        }

        // Load device configurations from server
        function loadDeviceConfigs(){
            fetch('/get_device_configs').then(r=>r.json()).then(data=>{
                deviceConfigs={};
                data.devices.forEach(dev=>{
                    deviceConfigs[dev.slot]={
                        midiMapping:dev.midiMapping,
                        midiType:dev.midiType,
                        midiChannel:dev.midiChannel};
                });
                console.log('Loaded device configs:',deviceConfigs);
            });
        }

        function showTab(tabName){
            document.querySelectorAll('.tab-content').forEach(t=>t.classList.remove('active'));
            document.querySelectorAll('.icon').forEach(i=>i.classList.remove('active'));
            document.getElementById(tabName+'Content').classList.add('active');
            document.getElementById(tabName+'Tab').classList.add('active');
            let sidebar=document.getElementById('sidebar');
            let body=document.body;
            if(tabName==='devices'){
                sidebar.style.display='flex';
                body.style.marginRight='320px';
                // Re-apply layer count immediately in case it changed on another tab
                if(typeof applyLayerCount === 'function') applyLayerCount();
            }else{
                sidebar.style.display='none';
                body.style.marginRight='24px';
                stopRawUpdates();
            }
            if(tabName==='about')updateSystemInfo();
            if(tabName==='midi')loadMidiChannelNames();
            if(tabName==='overview')loadMappingOverview();
            if(tabName==='osc'){ loadOsc(); startOscPolling(); } else stopOscPolling();
        }

        // Layer functionality
        function selectLayer(layerIndex) {
    flushAutoSave();               // save pending edits against the layer they were made on
    lastSelectTime = Date.now();
    currentLayer = layerIndex;
    
const layerIndicator = document.getElementById('layerIndicator');
if(layerIndicator) {
    layerIndicator.style.background = `var(--layer${layerIndex + 1}-color)`;
}

    // Update tab appearance
    document.querySelectorAll('.layer-tab').forEach((tab, index) => {
        tab.classList.toggle('active', index === layerIndex);
    });
    
    // Update the theme colors based on selected layer
    const body = document.body;
    // Remove all existing layer classes
    body.classList.remove('layer-color-theme', 'layer-0', 'layer-1', 'layer-2', 'layer-3');
    // Add the new layer class
    body.classList.add('layer-color-theme', `layer-${layerIndex}`);
    
    // Load layer-specific settings (panel may switch between MIDI and OSC per layer)
    if(currentDevice >= 0 && currentButton >= 0 && !currentIsPedal && !selectedIsEncoder) {
        refreshConfigTypeForLayer();
    }
    
    console.log(`Switched to layer ${layerIndex + 1} with color theme`);
}

        function loadLayerMidiSettings(device, layer, button) {
            fetch(`/get_layer_midi?device=${device}&layer=${layer}&button=${button}`)
                .then(r => r.json())
                .then(data => {
                    document.getElementById('midiType').value = data.type || 'note';
                    document.getElementById('midiNumber').value = data.number || 0;
                    document.getElementById('midiChannel').value = data.channel || 1;
                })
                .catch(err => {
                    console.error('Error loading layer MIDI settings:', err);
                    // Set defaults based on layer
                    document.getElementById('midiType').value = 'note';
                    document.getElementById('midiNumber').value = 60 + (layer * 16) + button;
                    document.getElementById('midiChannel').value = 1;
                });
        }

        function saveShiftConfig() {
            if(currentDevice === -1) return;
            
            const shiftButton = parseInt(document.getElementById('shiftButton').value);
            const shiftEnabled = document.getElementById('shiftEnabled').checked;
            const shiftMode = document.getElementById('shiftMode').checked;
            
            const payload = {
                device: currentDevice,
                shiftButton: shiftButton,
                shiftEnabled: shiftEnabled,
                shiftMode: shiftMode
            };
            
            fetch('/save_shift_config', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(payload)
            })
            .then(response => {
                if(response.ok) {
                    safeDisableButton('saveShiftBtn', true); // Disable save button safely
                    console.log('Shift config saved');
                    // Refresh device cards to show shift button indicators
                    setTimeout(updateDeviceCards, 500);
                }
            })
            .catch(err => {
                console.error('Error saving shift config:', err);
            });
        }

        function loadShiftConfiguration(device) {
    fetch(`/get_device_shift_config?device=${device}`)
        .then(r => r.json())
        .then(data => {
            // Use safe element access
            const shiftButtonEl = document.getElementById('shiftButton');
            const shiftEnabledEl = document.getElementById('shiftEnabled');
            const shiftModeEl = document.getElementById('shiftMode');
            
            if(shiftButtonEl) shiftButtonEl.value = data.shiftButton || -1;
            if(shiftEnabledEl) shiftEnabledEl.checked = data.shiftEnabled || false;
            if(shiftModeEl) shiftModeEl.checked = data.shiftMode !== false;
            
            // Update layer tabs to show which have configuration
            if(data.layers) {
                data.layers.forEach((layer, index) => {
                    let hasConfig = layer.buttons.some(btn => 
                        btn.midiMapping > 0 || btn.midiType !== 'note'
                    );
                    const layerTabs = document.querySelectorAll('.layer-tab');
                    if(layerTabs[index]) {
                        layerTabs[index].classList.toggle('has-config', hasConfig);
                    }
                });
            }
            
            // Keep save button disabled until user makes a change
            safeDisableButton('saveShiftBtn', true);
        })
        .catch(err => {
            console.error('Error loading shift config:', err);
            // Set defaults safely
            const shiftButtonEl = document.getElementById('shiftButton');
            const shiftEnabledEl = document.getElementById('shiftEnabled');
            const shiftModeEl = document.getElementById('shiftMode');
            
            if(shiftButtonEl) shiftButtonEl.value = -1;
            if(shiftEnabledEl) shiftEnabledEl.checked = false;
            if(shiftModeEl) shiftModeEl.checked = true;
        });
}


function saveMidiOutput() {
    const mode = document.getElementById('midiOutputSelect').value;
    fetch('/save_midi_output?mode=' + mode)
        .then(r => r.text())
        .then(response => {
            if(response === 'OK') {
                console.log('MIDI output mode saved');
            }
        });
}

function loadMidiOutputMode() {
    fetch('/get_midi_output')
        .then(r => r.json())
        .then(data => {
            document.getElementById('midiOutputSelect').value = data.mode;
        })
        .catch(err => console.error('Error loading MIDI output mode:', err));
}

        // Enhanced button selection with layer support
        function selectButton(deviceId, buttonIndex) {
    flushAutoSave();
    lastSelectTime = Date.now();
    safeDisableButton('saveBtn', true);
    currentLayer = 0;
    currentDevice = deviceId;
    currentButton = buttonIndex;
    currentIsPedal = false;
    selectedIsEncoder = false;
    selectedLED = -1;
    hideAllPanels();
    document.querySelector('.config-type-pills').style.display = 'flex';
    stopRawUpdates();
    
    // Clear previous selections
    document.querySelectorAll('.pad').forEach(p => p.classList.remove('selected'));
    document.querySelectorAll('.pad-vertical').forEach(p => p.classList.remove('selected'));
    document.querySelectorAll('.pedal-container').forEach(p => p.style.border = 'none');
    
    // Highlight selected button
    let selectedPad = document.querySelector('#d' + deviceId + 'b' + buttonIndex);
    if(selectedPad) {
        selectedPad.classList.add('selected');
    }
    
    // Get device info from slot mappings
    fetch('/get_slot_mappings').then(r => r.json()).then(mappings => {
        const slotData = mappings.mappings.find(m => m.slot === deviceId);
        if(slotData && slotData.deviceId) {
            const deviceName = slotData.deviceType + " " + slotData.deviceId.substring(4);
            const deviceLabel = document.getElementById('deviceIDLabel');
            const buttonLabel = document.getElementById('buttonLabel');
            
            if(deviceLabel) deviceLabel.innerText = deviceName;
            if(buttonLabel) buttonLabel.innerText = slotData.deviceType === "Pedal" ? "Button" : `Button ${buttonIndex + 1}`;
            
            // Show layer section for button devices, hide calibration
            const layerSection = document.getElementById('layerTabs');
            const calSection = document.getElementById('calSection');
            
            if(calSection) calSection.style.display = 'none';
            // Pedal's extra button only supports MIDI (firmware has no layer control for it)
            const pills = document.querySelector('.config-type-pills');
            if(pills) pills.style.display = 'flex';
            const pd = document.getElementById('pillDevice');
            if(pd) pd.style.display = (slotData.deviceType === 'Pedal') ? 'none' : '';
            applyLayerCount();
            
            currentLayer = 0;  // keep in sync with visual tabs
            document.querySelectorAll('.layer-tab').forEach((tab, index) => {
                tab.classList.toggle('active', index === 0);
            });
            
            // Initialize layer color theme
            const body = document.body;
            body.classList.remove('layer-color-theme', 'layer-0', 'layer-1', 'layer-2', 'layer-3');
            body.classList.add('layer-color-theme', 'layer-0');
            
            // Update layer indicator
            const layerIndicator = document.getElementById('layerIndicator');
            if(layerIndicator) {
                layerIndicator.style.background = 'var(--layer1-color)';
            }
            
            // Load button settings based on configuration type
            loadButtonConfiguration(deviceId, buttonIndex);
            
            // Show sidebar
            const sidebar = document.getElementById('sidebar');
            if(sidebar) {
                sidebar.style.display = 'flex';
                document.body.style.marginRight = '320px';
            }
        }
    });
}

// Hide every config panel in the sidebar; each select* function then shows what it needs.
function hideAllPanels() {
    ['layerTabs','midiConfigSection','deviceConfigSection','encoderConfigSection','oscActionSection',
     'ledConfigSection','pedalMidiSection','calSection','sidebarEmpty'].forEach(id => {
        const el = document.getElementById(id); if(el) el.style.display = 'none';
    });
    const pills = document.querySelector('.config-type-pills');
    if(pills) pills.style.display = 'none';
    const info = document.getElementById('selInfo'); if(info) info.style.display = 'block';
}
function showEmptySidebar() {
    hideAllPanels();
    const info = document.getElementById('selInfo'); if(info) info.style.display = 'none';
    const e = document.getElementById('sidebarEmpty'); if(e) e.style.display = 'block';
}

// While panels are being filled from the device, field "change" handlers must not
// trigger an auto-save (that used to race with loading and overwrite settings).
var populating = false;

// Pick the panel matching what this button does on the current layer.
function refreshConfigTypeForLayer() {
    return fetch(`/get_layer_midi?device=${currentDevice}&layer=${currentLayer}&button=${currentButton}`)
        .then(r => r.json())
        .then(d => {
            const dc = d.deviceControl && d.deviceControl !== 'none' && d.deviceControl !== '';
            return setConfigType(dc ? 'device' : (d.type === 'osc' ? 'osc' : 'midi'));
        });
}
function isPedalButtonSelected() {
    const m = window._lastMappings && window._lastMappings.mappings.find(x => x.slot === currentDevice);
    return !!(m && m.deviceType === 'Pedal' && !currentIsPedal);
}

function loadButtonConfiguration(device, button) {
    const ledSec = document.getElementById('ledConfigSection');
    if(ledSec) ledSec.style.display = 'block';
    loadLEDConfiguration(device, button);
    return refreshConfigTypeForLayer()
        .catch(err => {
            console.error('loadButtonConfiguration error:', err);
            return setConfigType('midi');
        });
}

        function selectPedal(slot) {
            flushAutoSave();
            lastSelectTime = Date.now();
            safeDisableButton('saveBtn', true);
            currentDevice = slot;
            currentButton = 0;
            currentIsPedal = true;

            hideAllPanels();
            selectedIsEncoder = false;
            // Highlight pedal, clear button selections
            document.querySelectorAll('.pad').forEach(p => p.classList.remove('selected'));
            document.querySelectorAll('.pedal-container').forEach(p => p.style.border = 'none');
            const pedalContainer = document.querySelector('[data-slot="' + slot + '"] .pedal-container');
            if(pedalContainer) pedalContainer.style.border = '2px solid %THEME_COLOR%';

            // Show sidebar
            document.getElementById('sidebar').style.display = 'flex';
            document.body.style.marginRight = '320px';

            // ── Hide button-only sections ──────────────────────────────────
            document.getElementById('layerTabs').style.display        = 'none';
            document.getElementById('midiConfigSection').style.display = 'none';
            document.getElementById('deviceConfigSection').style.display = 'none';
            document.getElementById('encoderConfigSection').style.display = 'none';
            document.getElementById('ledConfigSection').style.display  = 'none';
            document.querySelector('.config-type-pills').style.display = 'none';

            // ── Show pedal-only sections ───────────────────────────────────
            document.getElementById('pedalMidiSection').style.display = 'block';
            document.getElementById('calSection').style.display       = 'block';

            document.getElementById('buttonLabel').innerText = 'Expression Pedal';

            // Load MIDI CC + calibration
            fetch('/get_slot_mappings').then(r => r.json()).then(mappings => {
                const slotData = mappings.mappings.find(m => m.slot === slot);
                if(slotData) {
                    document.getElementById('deviceIDLabel').innerText =
                        slotData.deviceType + (slotData.deviceId ? ' ' + slotData.deviceId.substring(4) : '');
                }
                // CC number and channel
                const cfg = deviceConfigs[slot];
                document.getElementById('pedalCCNumber').value  = cfg ? (cfg.midiMapping[0] || 7) : 7;
                document.getElementById('pedalCCChannel').value = cfg ? (cfg.midiChannel[0] || 1) : 1;
                safeDisableButton('saveBtn', false);
            }).catch(() => {
                document.getElementById('pedalCCNumber').value  = 7;
                document.getElementById('pedalCCChannel').value = 1;
                safeDisableButton('saveBtn', false);
            });

            // Load output type (MIDI CC or OSC) and OSC fields
            fetch(`/get_layer_midi?device=${slot}&layer=0&button=0`).then(r => r.json()).then(d => {
                populating = true;
                document.getElementById('pedalOscAddr').value = d.oscAddr || '/pedal';
                document.getElementById('pedalOscMin').value  = d.oscOff != null ? d.oscOff : 0;
                document.getElementById('pedalOscMax').value  = d.oscOn  != null ? d.oscOn  : 1;
                document.getElementById('pedalOscArg').value  = d.oscArg === 'i' ? 'i' : 'f';
                setPedalOut(d.type === 'osc' ? 'osc' : 'cc', false);
                populating = false;
            }).catch(() => { populating = false; setPedalOut('cc', false); });

            // Load calibration values
            fetch('/get_pedal_cal?device=' + slot).then(r => r.json()).then(c => {
                document.getElementById('calMin').value    = c.min;
                document.getElementById('calMax').value    = c.max;
                const inv = document.getElementById('calInvert');
                if(inv) inv.checked = (c.invert == 1);
                document.getElementById('saveCalBtn').disabled = false;
            }).catch(() => {
                document.getElementById('calMin').value    = 0;
                document.getElementById('calMax').value    = 4095;
                document.getElementById('saveCalBtn').disabled = false;
            });

            // Reset live value displays
            document.getElementById('rawValue').innerText  = '----';
            document.getElementById('calValue').innerText  = '----';
            const mo = document.getElementById('midiOut');
            if(mo) mo.innerText = '----';
            startRawUpdates();
        }        function saveLayerSettings() {
    if(currentDevice === -1 || currentButton === -1) return;
    
    const midiTypeElement = document.getElementById('midiType');
    if(!midiTypeElement) {
        console.error('midiType element not found');
        return;
    }
    
    const type = midiTypeElement.value;
    
    // Handle device control separately
    if(type === 'device_control') {
        saveDeviceControlSettings();
        return;
    }
    
    // Handle regular MIDI settings
    const numberElement = document.getElementById('midiNumber');
    const channelElement = document.getElementById('midiChannel');
    
    if(!numberElement || !channelElement) {
        console.error('Required MIDI elements not found');
        return;
    }
    
    const number = parseInt(numberElement.value);
    const channel = parseInt(channelElement.value);
    
    // Validate number and channel
    if(isNaN(number) || number < 0 || number > 127) {
        alert('MIDI Number must be between 0 and 127.');
        return;
    }
    if(isNaN(channel) || channel < 1 || channel > 16) {
        alert('MIDI Channel must be between 1 and 16.');
        return;
    }
    
    // Create payload that includes both MIDI and device control clearing
    const payload = {
        device: currentDevice,
        layer: currentLayer,
        button: currentButton,
        type: type,
        number: number,
        channel: channel,
        clearDeviceControl: true  // Flag to clear device control
    };
    
    fetch('/save_layer_midi', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
    })
    .then(response => {
        if(response.ok) {
            safeDisableButton('saveBtn', true);
            console.log('Layer MIDI settings saved and device control cleared');
            
            // Update layer tab to show it has configuration
            const layerTabs = document.querySelectorAll('.layer-tab');
            if(layerTabs[currentLayer]) {
                layerTabs[currentLayer].classList.add('has-config');
            }
            
            // Update device configs cache
            if(!deviceConfigs[currentDevice]) deviceConfigs[currentDevice] = {};
            if(!deviceConfigs[currentDevice][currentButton]) deviceConfigs[currentDevice][currentButton] = {};
            deviceConfigs[currentDevice][currentButton] = {
                type: type,
                number: number,
                channel: channel
            };
        } else {
            throw new Error('Failed to save layer settings');
        }
    })
    .catch(err => {
        console.error('Error saving layer settings:', err);
        alert('Error saving settings.');
    });
}

function saveDeviceControlSettings() {
    if(currentDevice === -1 || currentButton === -1) return;
    
    const controlScopeElement = document.getElementById('deviceControlScope');
    const targetLayerElement = document.getElementById('targetLayer');
    const controlModeElement = document.getElementById('controlMode');  // CHANGED
    
    if(!controlScopeElement || !controlModeElement) {  // CHANGED
        console.error('Device control elements not found');
        return;
    }
    
    const controlType = controlScopeElement.value;
    const targetLayer = targetLayerElement ? parseInt(targetLayerElement.value) : 0;
    const mode = controlModeElement.value;  // CHANGED
    
    const payload = {
        device: currentDevice,
        button: currentButton,
        controlType: controlType,
        targetLayer: targetLayer,
        mode: mode  // CHANGED from momentary
    };
    
    fetch('/save_device_control', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
    })
    .then(response => {
        if(response.ok) {
            safeDisableButton('saveBtn', true);
            showSavedFeedback();
            console.log('Device control settings saved');
            setTimeout(updateDeviceCards, 300);
        } else {
            throw new Error('Failed to save device control settings');
        }
    })
    .catch(err => {
        console.error('Error saving device control settings:', err);
        alert('Error saving device control settings.');
    });
}




        function savePedalSettings() {
            if(document.getElementById('pedalOutType').value === 'osc') {
                let addr = document.getElementById('pedalOscAddr').value.trim();
                if(addr && addr[0] !== '/') addr = '/' + addr;
                if(!addr) return;
                const lo = parseFloat(document.getElementById('pedalOscMin').value);
                const hi = parseFloat(document.getElementById('pedalOscMax').value);
                const payload = { device: currentDevice, layer: 0, button: 0, type: 'osc', oscAddr: addr,
                    oscArg: document.getElementById('pedalOscArg').value, oscOff: isNaN(lo) ? 0 : lo, oscOn: isNaN(hi) ? 1 : hi };
                fetch('/save_layer_midi', {method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify(payload)})
                    .then(r => { if (r.ok) { safeDisableButton('saveBtn', true); showSavedFeedback(); } })
                    .catch(err => console.error('savePedalSettings error:', err));
                return;
            }
            const ccNum = parseInt(document.getElementById('pedalCCNumber').value) || 7;
            const ccCh  = parseInt(document.getElementById('pedalCCChannel').value) || 1;
            fetch(`/save_midi?device=${currentDevice}&button=0&val=${ccNum}&type=cc&channel=${ccCh}`)
                .then(r => { if (r.ok) { safeDisableButton('saveBtn', true); showSavedFeedback(); } })
                .catch(err => console.error('savePedalSettings error:', err));
        }

        function savePedalCal() {
            // Ensure a pedal is selected
            if(!currentIsPedal || currentDevice === -1 || currentButton === -1) return;

            // Get calibration values from input fields
            const min = parseInt(document.getElementById('calMin').value);
            const max = parseInt(document.getElementById('calMax').value);
            const invert = document.getElementById('calInvert').checked;

            // Validate values
            if(isNaN(min) || isNaN(max) || min < 0 || max > 4095 || min >= max) {
                alert('Invalid calibration values. Ensure 0 ≤ min < max ≤ 4095.');
                return;
            }

            // Save locally in deviceConfigs
            if(!deviceConfigs[currentDevice]) deviceConfigs[currentDevice] = {};
            if(!deviceConfigs[currentDevice][currentButton]) deviceConfigs[currentDevice][currentButton] = {};
            deviceConfigs[currentDevice][currentButton].calibration = { min, max, invert };

            // Prepare payload for server
            const payload = {
                device: currentDevice,
                button: currentButton,
                min: min,
                max: max,
                invert: invert
            };

            // Send to correct endpoint with URL params
            fetch(`/save_pedal_cal?device=${currentDevice}&min=${min}&max=${max}&invert=${invert ? 1 : 0}`)
                .then(r => r.ok ? r.text() : Promise.reject(r.status))
                .then(() => { document.getElementById('saveCalBtn').disabled = true; })
                .catch(err => alert('Error saving pedal calibration: ' + err));
        }

        function safeDisableButton(buttonId, disabled = true) {
            if(!disabled && populating) return;   // loading values isn't an edit
            const button = document.getElementById(buttonId);
            if(button) button.disabled = disabled;
            if(buttonId === 'saveBtn') {
                const dot = document.getElementById('unsavedDot');
                if(dot) dot.style.display = disabled ? 'none' : 'block';
                if(!disabled) scheduleAutoSave();  // every "unsaved change" now auto-saves
            }
        }
        
        function onMidiTypeChange() {
    const midiType = document.getElementById('midiType');
    const deviceControlOptions = document.getElementById('deviceControlOptions');
    const regularMidiOptions = document.getElementById('regularMidiOptions');
    
    if(!midiType || !deviceControlOptions || !regularMidiOptions) {
        console.error('Required UI elements not found');
        return;
    }
    
    if(midiType.value === 'device_control') {
        deviceControlOptions.style.display = 'block';
        regularMidiOptions.style.display = 'none';
        loadDeviceControlSettings();
    } else {
        deviceControlOptions.style.display = 'none';
        regularMidiOptions.style.display = 'block';
    }
    
    safeDisableButton('saveBtn', false);
}
        
        function onDeviceControlTypeChange() {
            const controlTypeElement = document.getElementById('deviceControlType');
            const layerOptions = document.getElementById('layerControlOptions');
            
            if(!controlTypeElement || !layerOptions) {
                console.error('Device control elements not found');
                return;
            }
            
            const controlType = controlTypeElement.value;
            if(controlType === 'layer_device' || controlType === 'layer_global') {
                layerOptions.style.display = 'block';
            } else {
                layerOptions.style.display = 'none';
            }
            
            safeDisableButton('saveBtn', false); // Enable save button safely
        }
        
// ── OSC action panel ─────────────────────────────────────
let oscNetCache = null;
function oscWarnText(c) {
    const link = ' <a onclick="showTab(\'osc\')">Open the OSC tab</a>';
    if(!c) return '';
    if(!c.enabled) return 'OSC is turned off, so nothing will be sent.' + link;
    if(!c.targetIP) return 'No Send-to IP set. The controller will only reply to whoever last sent it OSC.' + link;
    return '';
}
function refreshOscWarnings() {
    return fetch('/get_osc').then(r => r.json()).then(c => {
        oscNetCache = c;
        ['oscWarn', 'pedalOscWarn'].forEach(id => {
            const el = document.getElementById(id); if(!el) return;
            const t = oscWarnText(c); el.innerHTML = t; el.style.display = t ? 'block' : 'none';
        });
    }).catch(() => {});
}
function onOscArgChange() {
    const arg = document.getElementById('btnOscArg').value;
    const beh = document.getElementById('btnOscBehavior').value;
    document.getElementById('btnOscVals').style.display = arg === 'none' ? 'none' : '';
    document.getElementById('btnOscOffGroup').style.display = beh === 'trigger' ? 'none' : '';
    document.getElementById('btnOscOnLabel').textContent  = beh === 'latching' ? 'On value'  : 'Press value';
    document.getElementById('btnOscOffLabel').textContent = beh === 'latching' ? 'Off value' : 'Release value';
    onOscSettingsChange();
}
function onOscSettingsChange() { safeDisableButton('saveBtn', false); }
function loadOscSettings() {
    refreshOscWarnings();
    if(currentDevice === -1 || currentButton === -1) return Promise.resolve();
    return fetch(`/get_layer_midi?device=${currentDevice}&layer=${currentLayer}&button=${currentButton}`)
        .then(r => r.json())
        .then(d => {
            populating = true;
            document.getElementById('btnOscAddr').value = d.oscAddr || ('/button/' + (currentButton + 1));
            document.getElementById('btnOscArg').value = d.oscArg || 'f';
            document.getElementById('btnOscOn').value  = d.oscOn  != null ? d.oscOn  : 1;
            document.getElementById('btnOscOff').value = d.oscOff != null ? d.oscOff : 0;
            const beh = d.behavior || 'momentary';
            document.getElementById('btnOscBehavior').value = ['momentary','latching','trigger'].includes(beh) ? beh : 'momentary';
            onOscArgChange();
            populating = false;
        })
        .catch(err => { populating = false; console.error('loadOscSettings error:', err); });
}
function saveOscSettings() {
    if(currentDevice === -1 || currentButton === -1) return;
    let addr = document.getElementById('btnOscAddr').value.trim();
    if(addr && addr[0] !== '/') { addr = '/' + addr; document.getElementById('btnOscAddr').value = addr; }
    if(!addr) return;   // wait until an address is entered
    const on  = parseFloat(document.getElementById('btnOscOn').value);
    const off = parseFloat(document.getElementById('btnOscOff').value);
    const payload = { device: currentDevice, layer: currentLayer, button: currentButton, type: 'osc',
        oscAddr: addr, oscArg: document.getElementById('btnOscArg').value,
        oscOn: isNaN(on) ? 1 : on, oscOff: isNaN(off) ? 0 : off,
        behavior: document.getElementById('btnOscBehavior').value, clearDeviceControl: true };
    fetch('/save_layer_midi', {method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify(payload)})
        .then(r => {
            if(r.ok) { safeDisableButton('saveBtn', true); showSavedFeedback(); setTimeout(updateDeviceCards, 300); }
            else r.text().then(t => alert('Save failed: ' + t));
        })
        .catch(err => alert('Save error: ' + err.message));
}
function oscSendNow(addrId, argId, valId) {
    const addr = document.getElementById(addrId).value.trim();
    const q = new URLSearchParams({ addr, arg: document.getElementById(argId).value, val: document.getElementById(valId).value || '1' });
    fetch('/osc_send_test?' + q).then(r => r.text().then(t => ({ok: r.ok, t})))
        .then(({ok, t}) => { if(ok) showSavedFeedbackText('Sent ' + addr); else alert(t); });
}
function showSavedFeedbackText(t) {
    const fb = document.getElementById('saveFeedback');
    if(!fb) return;
    const old = fb.textContent; fb.textContent = t; fb.classList.add('visible');
    setTimeout(() => { fb.classList.remove('visible'); fb.textContent = old; }, 1500);
}

// ── Pedal output: MIDI CC or OSC ─────────────────────────
function setPedalOut(type, userClick) {
    document.getElementById('pedalOutType').value = type;
    document.getElementById('pedalOutCC').classList.toggle('on', type === 'cc');
    document.getElementById('pedalOutOsc').classList.toggle('on', type === 'osc');
    document.getElementById('pedalCCFields').style.display  = type === 'cc'  ? '' : 'none';
    document.getElementById('pedalOscFields').style.display = type === 'osc' ? '' : 'none';
    if(type === 'osc') refreshOscWarnings();
    if(userClick) savePedalSettings();
}

// ── Layer control panel ──────────────────────────────────
let lcState = { action: 'jump', target: 1, mode: 'jump', scope: 'global' };

function lcFromConfig(ct, target, mode) {
    const s = { action: 'jump', target: target || 0, mode: mode || 'jump', scope: 'global' };
    if(!ct || ct === 'none') return null;
    s.scope  = ct.endsWith('_device') ? 'device' : 'global';
    s.action = ct.indexOf('_up_') > 0 ? 'up' : ct.indexOf('_down_') > 0 ? 'down' : 'jump';
    return s;
}
function lcControlType() {
    if(lcState.action === 'up')   return 'layer_up_'   + lcState.scope;
    if(lcState.action === 'down') return 'layer_down_' + lcState.scope;
    return 'layer_' + lcState.scope;
}
function lcRender() {
    const mark = (id, v) => document.querySelectorAll('#' + id + ' button')
        .forEach(b => b.classList.toggle('on', b.dataset.v === String(v)));
    mark('lcActionSeg', lcState.action);
    mark('lcLayerBtns', lcState.target);
    mark('lcModeList',  lcState.mode);
    mark('lcScopeSeg',  lcState.scope);
    document.querySelectorAll('#lcLayerBtns button').forEach((b, i) => b.style.display = i < numLayers ? '' : 'none');
    const isJump = lcState.action === 'jump';
    document.getElementById('lcTargetGroup').style.display = isJump ? '' : 'none';
    document.getElementById('lcModeGroup').style.display   = isJump ? '' : 'none';

    const who = lcState.scope === 'global' ? 'all devices' : 'this device';
    const L = lcState.target + 1;
    let txt;
    if(lcState.action === 'up')        txt = `Each press moves ${who} to the next layer (wraps after ${numLayers}).`;
    else if(lcState.action === 'down') txt = `Each press moves ${who} to the previous layer (wraps to ${numLayers}).`;
    else if(lcState.mode === 'jump')   txt = `Pressing switches ${who} to <b>Layer ${L}</b> and stays there.`;
    else if(lcState.mode === 'latching') txt = `Press to switch ${who} to <b>Layer ${L}</b>, press again to return to Layer 1.`;
    else                               txt = `${who[0].toUpperCase() + who.slice(1)} use <b>Layer ${L}</b> while held, back to Layer 1 on release.`;
    if(isJump && lcState.target === 0 && lcState.mode !== 'jump') txt += ' <span style="color:#FF8A80;">(Target is Layer 1, so this does nothing. Pick another layer.)</span>';
    if(lcState.scope === 'device') txt += '<br><span style="color:#79747E;font-size:11px;">Per-device layers override the global layer for this device until it returns to Layer 1.</span>';
    document.getElementById('lcSummary').innerHTML = txt;

    // Mirror into the hidden fields the save function reads
    document.getElementById('deviceControlScope').value = lcControlType();
    document.getElementById('targetLayer').value = String(lcState.target);
    document.getElementById('controlMode').value = lcState.mode;
}
function lcSet(key, val) {
    lcState[key] = val;
    lcRender();
    saveDeviceControlSettings();   // explicit click → save right away
}

function loadDeviceControlSettings() {
    if(currentDevice === -1 || currentButton === -1) return Promise.resolve();
    return fetch(`/get_device_control?device=${currentDevice}&button=${currentButton}`)
        .then(r => r.json())
        .then(data => {
            const s = lcFromConfig(data.controlType, data.targetLayer, data.mode);
            // Unconfigured: sensible default = jump all devices to Layer 2 and stay
            lcState = s || { action: 'jump', target: numLayers > 1 ? 1 : 0, mode: 'jump', scope: 'global' };
            if(lcState.target >= numLayers) lcState.target = 0;
            lcRender();
        })
        .catch(err => {
            console.error('Error loading device control settings:', err);
            lcState = { action: 'jump', target: numLayers > 1 ? 1 : 0, mode: 'jump', scope: 'global' };
            lcRender();
        });
}

function onDeviceControlScopeChange() {
    // This function can be used for future UI updates if needed
    // For now, just enable the save button
    safeDisableButton('saveBtn', false);
}



        function toggleGlobalLayerControl() {
            const toggle = document.getElementById('globalLayerToggle');
            if(!toggle) return;
            
            const enabled = toggle.checked;
            fetch('/save_global_layer_control?enabled=' + enabled)
                .then(r => r.text())
                .then(response => {
                    if(response === 'OK') {
                        console.log('Global layer control ' + (enabled ? 'enabled' : 'disabled'));
                        // Refresh device cards to show current state
                        setTimeout(updateDeviceCards, 500);
                    }
                });
        }



        function saveWifi() {
            const params = new URLSearchParams();
            params.append('ssid_primary',  document.getElementById('ssid_primary').value);
            params.append('pass_primary',  document.getElementById('pass_primary').value);
            params.append('ssid_fallback', document.getElementById('ssid_fallback').value);
            params.append('pass_fallback', document.getElementById('pass_fallback').value);
            params.append('hostname',      document.getElementById('wifi_hostname').value);
            const st = document.getElementById('wifiSaveStatus');
            fetch('/save_wifi', { method:'POST', headers:{'Content-Type':'application/x-www-form-urlencoded'}, body:params.toString() })
            .then(()=>{ if(st){st.style.display='block';st.style.background='#1B5E20';st.style.color='#A5D6A7';st.textContent='Saved. Reconnecting...'; }})
            .catch(()=>{ if(st){st.style.display='block';st.style.background='#1B5E20';st.style.color='#A5D6A7';st.textContent='Settings sent. Device reconnecting.'; }});
        }

        function scanWiFi(){
            fetch('/scan_wifi_ajax').then(r=>r.json()).then(d=>{
                let sel=document.getElementById('ssid_primary');
                sel.innerHTML='';
                d.forEach(ssid=>{
                    let o=document.createElement('option');
                    o.value=ssid;o.text=ssid;sel.appendChild(o);
                });
            });
        }

        function loadMidiChannelNames(){
            fetch('/get_midi_channel_names').then(r=>r.json()).then(names=>{
                for(let ch=1;ch<=16;ch++){
                    let input=document.getElementById('ch'+ch+'Name');
                    if(names['ch'+ch])input.value=names['ch'+ch];
                }
            });
        }

        function saveMidiChannelName(ch){
            let name=document.getElementById('ch'+ch+'Name').value;
            fetch('/save_midi_channel_name?channel='+ch+'&name='+encodeURIComponent(name));
        }

        function changeTheme(){
            let color=document.getElementById('themeSelect').value;
            fetch('/save_theme?color='+encodeURIComponent(color)).then(()=>setTimeout(()=>location.reload(),500));
        }

        function sendMidiPanic(){fetch('/midi_panic');}

        function updateSystemInfo(){
            fetch('/system_info').then(r=>r.json()).then(info=>{
                document.getElementById('freeHeap').innerText=info.freeHeap;
                document.getElementById('uptime').innerText=info.uptime;
            });
        }

        function factoryReset(){
            if(confirm('Are you sure? This will erase all settings and restart the device.')){
                fetch('/factory_reset',{method:'POST'}).then(()=>alert('Factory reset sent. Device is restarting.')).catch(()=>alert('Settings sent. Device is restarting.'));
            }
        }

        function toggleAdvancedOptions() {
            const toggle = document.getElementById('advancedToggle');
            const advancedSection = document.getElementById('advancedOptions');
            
            if (toggle.checked) {
                advancedSection.style.display = 'block';
                // Save state to server
                fetch('/save_advanced_toggle?enabled=true');
            } else {
                advancedSection.style.display = 'none';
                // Save state to server
                fetch('/save_advanced_toggle?enabled=false');
            }
        }

        function saveI2CSpeed(speed) {
            fetch('/save_i2c_speed?speed=' + speed)
                .then(r => r.text())
                .then(response => {
                    if (response === 'OK') {
                        alert('I2C speed updated. Device will restart.');
                    }
                });
        }

        function saveMidiPollingRate(rate) {
            fetch('/save_midi_polling?rate=' + rate)
                .then(r => r.text())
                .then(response => {
                    if (response === 'OK') {
                        console.log('MIDI polling rate updated to ' + rate + 'ms');
                    }
                });
        }

        function toggleDebugMode() {
            const debugMode = document.getElementById('debugMode').checked;
            fetch('/toggle_debug?enabled=' + debugMode);
        }

        function exportSettings() { alert('Export not yet implemented.'); }
        function importSettings() { alert('Import not yet implemented.'); }

        // Live updates - FIXED version
        setInterval(() => {
            fetch('/device_state_json').then(r => r.json()).then(states => {
                // Update button states and pedal positions
                for(let slot = 0; slot < 8; slot++) {
                    const slotElement = document.querySelector('[data-slot="' + slot + '"]');
                    if(!slotElement) continue;
                    
                    const deviceState = states[slot];
                    if(deviceState === undefined) continue;
                    
                    // Check if this is a pedal or button device
                    const pedalContainer = slotElement.querySelector('.pedal-container');
                    if(pedalContainer) {
                        // This is a pedal - update position and value
                        const pedalThumb = slotElement.querySelector('#pedal' + slot);
                        const pedalValue = slotElement.querySelector('#pedalValue' + slot);
                        
                        if(pedalThumb && pedalValue) {
                            const pedalPos = (deviceState * 100) / 127;
                            pedalThumb.style.left = pedalPos + '%';
                            pedalValue.innerText = deviceState + '/127';
                        }
                    } else {
    // This is a button device - update pad states
    for(let btn = 0; btn < 4; btn++) {
        const pad = slotElement.querySelector('#d' + slot + 'b' + btn);
        if(pad) {
            const isPressed = deviceState & (1 << btn);
            const isSelected = pad.classList.contains('selected');
            const isShift = pad.classList.contains('shift-button');
            const isLayerActive = pad.classList.contains('layer-active');
            
            // Determine if this is vertical or horizontal layout
            const isVertical = pad.classList.contains('pad-vertical');
            const baseClass = isVertical ? 'pad-vertical' : 'pad';
            
            // Clear all classes and rebuild
            pad.className = baseClass;
            if(isPressed) pad.classList.add('active');
            if(isSelected) pad.classList.add('selected');
            if(isShift) pad.classList.add('shift-button');
            if(isLayerActive) pad.classList.add('layer-active');
            const lc = ['layer-1','layer-2','layer-3','layer-4'].find(c=>pad.classList.contains(c)) || null;
            if(lc) pad.classList.add(lc);
            // Update LED ring
            const _dc = window._lastConfigs && window._lastConfigs[slot];
            if(_dc && _dc.ledMode && _dc.ledMode[btn] === 'button_state') {
                const _fl = !_dc.ledFollowLayer || _dc.ledFollowLayer[btn] !== false;
                const _rc2 = _fl ? `var(--layer${(_dc.activeLayer||0)+1}-color)` : (_dc.ledColor?.[btn]||'#fff');
                pad.style.boxShadow = isPressed ? `0 0 0 3px ${_rc2},0 0 10px ${_rc2}60` : '0 0 0 2px #3d3a42';
            }
        }
    }
}
                }
            }).catch(err => console.error('Live update error:', err));
            
            // Update WiFi status
            fetch('/wifi_status_json').then(r => r.json())
                .then(d => {
                    const statusEl = document.getElementById('wifi_status');
                    if(statusEl) {
                        statusEl.innerText = d.status + ' | IP: ' + d.ip;
                    }
                })
                .catch(err => console.error('WiFi status error:', err));
        }, 3000);

        // Update LCD mirror every 2 seconds
setInterval(updateLCDMirror, 2000);

// Initial update
updateLCDMirror();

        // FIXED: Load device configs on page load

        let numLayers = 4;
        function setLayerCount(n) {
            numLayers = n;
            // Update preference pill buttons
            for(let i=1;i<=4;i++){const b=document.getElementById('lc'+i);if(b)b.classList.toggle('active',i===n);}
            // Update live feedback label
            const lbl = document.getElementById('layerCountLabel');
            if(lbl) lbl.textContent = '(' + n + ' active)';
            fetch('/save_num_layers?n='+n);
            applyLayerCount();
            // If a button is currently selected, force the layer tabs visible so the
            // user can see the change immediately without re-clicking the button
            if(currentDevice >= 0 && currentButton >= 0 && !currentIsPedal) {
                const lt = document.getElementById('layerTabs');
                if(lt) lt.style.display = 'flex';
            }
        }
        function applyLayerCount() {
            // Show/hide layer tabs
            document.querySelectorAll('.layer-tab').forEach((tab,i)=>{ tab.style.display=i<numLayers?'':'none'; });
            // If current layer is now out of range, reset to 0
            if(currentLayer>=numLayers) {
                currentLayer=0;
                document.querySelectorAll('.layer-tab').forEach((t,i)=>t.classList.toggle('active',i===0));
            }
            // Also update the Target Layer dropdown in Device Control
            const targetSel = document.getElementById('targetLayer');
            if(targetSel) {
                Array.from(targetSel.options).forEach((opt,i)=>{ opt.style.display = i<numLayers?'':'none'; });
                if(parseInt(targetSel.value) >= numLayers) targetSel.value = '0';
            }
        }
        function loadNumLayers() {
            fetch('/get_num_layers').then(r=>r.text()).then(n=>{
                const v=parseInt(n)||4;
                setLayerCount(v);
            }).catch(()=>{});
        }

        function saveMidi2() {
            const en = document.getElementById('midi2Toggle').checked ? '1' : '0';
            fetch('/save_midi2?enabled=' + en)
                .catch(err => console.error('MIDI 2.0 save error:', err));
        }
        function loadMidi2() {
            fetch('/get_midi2').then(r=>r.text()).then(v=>{
                const el = document.getElementById('midi2Toggle');
                if(el) el.checked = v.trim() === '1';
            }).catch(()=>{});
        }

        function handleMidiWheel(e, el) {
            e.preventDefault();
            const v = Math.max(0, Math.min(127, (parseInt(el.value)||0) + (e.deltaY<0?1:-1)));
            el.value = v; onMidiSettingsChange();
        }

        // ── OSC tab ────────────────────────────────────────────
        let oscPoll = null;
        function oscMsg(text, ok) {
            const el = document.getElementById('oscMsg');
            if(!el) return;
            el.textContent = text;
            el.style.color = ok ? '#A5D6A7' : '#FF8A80';
            if(text) setTimeout(() => { if(el.textContent === text) el.textContent = ''; }, 4000);
        }
        function renderOscStatus(c) {
            const st = document.getElementById('oscStatus');
            st.textContent = c.running ? 'Listening' : (c.enabled ? 'Enabled (waiting for Wi-Fi)' : 'Off');
            st.className = 'osc-pill' + (c.running ? ' on' : '');
            const conn = document.getElementById('oscConn');
            if(c.running) {
                const dest = c.targetIP ? (c.targetIP + ':' + c.outPort)
                           : (c.lastSender ? (c.lastSender + ':' + c.outPort + ' (last sender)') : 'nobody yet. Send the device any message, or set a Send-to IP');
                conn.innerHTML = 'Point your OSC app at <b>' + (c.deviceIP || '?') + ':' + c.inPort + '</b><br>Sending to <b>' + dest + '</b>';
            } else conn.innerHTML = '';
            document.getElementById('oscCounts').textContent = 'in ' + c.rx + ' / out ' + c.tx;
        }
        function loadOsc() {
            fetch('/get_osc').then(r => r.json()).then(c => {
                document.getElementById('oscEnabled').checked = c.enabled;
                document.getElementById('oscInPort').value = c.inPort;
                document.getElementById('oscOutPort').value = c.outPort;
                document.getElementById('oscTargetIP').value = c.targetIP;
                document.getElementById('oscPrefix').value = c.prefix;
                document.getElementById('oscSendControls').checked = c.sendControls;
                document.getElementById('oscMirrorOut').checked = c.mirrorMidiOut;
                document.getElementById('oscMirrorIn').checked = c.mirrorMidiIn;
                renderOscStatus(c);
                renderOscRef();
            }).catch(err => oscMsg('Could not load OSC settings: ' + err, false));
        }
        function saveOsc() {
            const ip = document.getElementById('oscTargetIP').value.trim();
            if(ip && !/^\d{1,3}(\.\d{1,3}){3}$/.test(ip)) { oscMsg('Send-to IP must look like 192.168.1.20 (or be blank).', false); return; }
            const q = new URLSearchParams({
                enabled:       document.getElementById('oscEnabled').checked ? '1' : '0',
                inPort:        document.getElementById('oscInPort').value,
                outPort:       document.getElementById('oscOutPort').value,
                targetIP:      ip,
                prefix:        document.getElementById('oscPrefix').value,
                sendControls:  document.getElementById('oscSendControls').checked ? '1' : '0',
                mirrorMidiOut: document.getElementById('oscMirrorOut').checked ? '1' : '0',
                mirrorMidiIn:  document.getElementById('oscMirrorIn').checked ? '1' : '0'
            });
            fetch('/save_osc?' + q.toString()).then(r => r.ok ? r.json() : r.text().then(t => Promise.reject(t)))
                .then(c => { renderOscStatus(c); document.getElementById('oscPrefix').value = c.prefix; renderOscRef(); oscMsg('Saved.', true); })
                .catch(err => oscMsg('Save failed: ' + err, false));
        }
        function oscTest() {
            fetch('/osc_test').then(r => r.ok ? 'Test message and state sync sent.' : r.text().then(t => Promise.reject(t)))
                .then(t => oscMsg(t, true)).catch(err => oscMsg(String(err), false));
        }
        function pollOsc() {
            fetch('/get_osc_log').then(r => r.json()).then(lines => {
                const el = document.getElementById('oscLog');
                el.textContent = lines.length ? lines.slice().reverse().join('\n') : 'Waiting for messages...';
            }).catch(() => {});
            fetch('/get_osc').then(r => r.json()).then(renderOscStatus).catch(() => {});
        }
        function startOscPolling() { if(!oscPoll) oscPoll = setInterval(pollOsc, 1000); pollOsc(); }
        function stopOscPolling()  { if(oscPoll) { clearInterval(oscPoll); oscPoll = null; } }
        function renderOscRef() {
            let p = (document.getElementById('oscPrefix').value || '').trim();
            if(p && p[0] !== '/') p = '/' + p;
            p = p.replace(/\/+$/, '');
            const rows = [
                ['Out', '/slot/S/button/B', 'i 1|0', 'Button pressed / released'],
                ['Out', '/slot/S/pedal', 'f 0-1, i 0-127', 'Expression pedal moved'],
                ['Out', '/slot/S/encoder', 'i +1|-1', 'Encoder turned one step'],
                ['Out', '/slot/S/encoder/button', 'i 1|0', 'Encoder push'],
                ['Out', '/layer', 'i L', 'Active layer changed'],
                ['Out', '/slot/S/layer', 'i L', 'Per-device layer changed'],
                ['Out', '/midi/out/note, /midi/out/cc', 'i ch, i num, i val', 'Every MIDI message sent (if mirroring)'],
                ['Out', '/midi/in/note, /midi/in/cc', 'i ch, i num, i val', 'MIDI received on DIN / network (if mirroring)'],
                ['In', '/slot/S/button/B', 'i|f 1|0, or no arg = tap', 'Press the button remotely (sends its MIDI / switches layers, same as stomping it)'],
                ['In', '/slot/S/pedal', 'f 0-1 or i 0-127', 'Send the pedal CC'],
                ['In', '/slot/S/encoder', 'i steps (+/-)', 'Turn the encoder'],
                ['In', '/slot/S/encoder/button', 'i 1|0', 'Push the encoder'],
                ['In', '/slot/S/led/B', 'i 1|0 | i r,g,b | s "#RRGGBB"', 'Take over a button LED (on / off / colour)'],
                ['In', '/slot/S/led/B/auto', '-', 'Give that LED back to normal behaviour'],
                ['In', '/led/auto', '-', 'Give all LEDs back'],
                ['In', '/layer', 'i L', 'Switch all devices to layer L'],
                ['In', '/layer/next, /layer/prev', '(press)', 'Step the layer'],
                ['In', '/slot/S/layer', 'i L', 'Set the layer of one device (1 = follow global)'],
                ['In', '/midi/note', 'i ch, i note, i vel', 'Send a MIDI note (vel 0 = off)'],
                ['In', '/midi/cc', 'i ch, i cc, i val', 'Send a MIDI CC'],
                ['In', '/midi/panic', '-', 'All notes off'],
                ['In', '/sync', '-', 'Device replies with /layer, /layers, /slot/S/type and /slot/S/layer'],
                ['In', '/ping', '-', 'Device replies /pong'],
            ];
            let h = '<tr><th></th><th>Address</th><th>Args</th><th>What it does</th></tr>';
            rows.forEach(r => {
                const addr = r[1].split(', ').map(a => '<code>' + p + a + '</code>').join('<br>');
                h += '<tr><td><span class="osc-dir ' + r[0].toLowerCase() + '">' + r[0] + '</span></td><td>' + addr + '</td><td class="dim">' + r[2] + '</td><td style="white-space:normal;">' + r[3] + '</td></tr>';
            });
            document.getElementById('oscRefTable').innerHTML = h;
        }

        // ── Live layer bar ─────────────────────────────────────
        function renderLayerBar(st) {
            document.querySelectorAll('#lbChips button').forEach((b, i) => {
                b.style.display = i < (st.numLayers || 4) ? '' : 'none';
                b.classList.toggle('on', i === st.active);
            });
            const m = document.getElementById('lbMode');
            if(m) m.textContent = st.globalEnabled ? 'Global layer' : 'Per-device layers (click a number to switch all devices)';
        }
        function pollLayerState() {
            fetch('/get_layer_state').then(r => r.json()).then(renderLayerBar).catch(() => {});
        }
        function setGlobalLayer(i) {
            fetch('/set_global_layer?layer=' + i).then(r => r.json()).then(st => {
                renderLayerBar(st);
                setTimeout(updateDeviceCards, 150);
            }).catch(err => console.error('Set layer error:', err));
        }
        setInterval(pollLayerState, 1000);

        window.onload = () => {
            showEmptySidebar();
            pollLayerState();
            loadNumLayers();
            loadMidi2();
            loadLedBrightness();
            scanWiFi();
            loadDeviceConfigs();
            loadMidiOutputMode();
            loadThemeSettings(); // Add this line
            showTab("devices");
            updateLCDMirror();
            setTimeout(() => {
                if (typeof initializeDragAndDrop === 'function') initializeDragAndDrop();
                if (typeof addTouchSupport === 'function') addTouchSupport();
            }, 100);
        };

        // Drag and Drop Variables
        let draggedElement = null;
        let draggedDeviceId = null;
        let draggedFromSlot = null;
        let dragPreview = null;

        // Initialize drag and drop when page loads
        function initializeDragAndDrop() {
            console.log('Initializing drag and drop...');
            
            // Add event listeners to all device cards
            document.querySelectorAll('.deviceCard[draggable="true"]').forEach(card => {
                card.addEventListener('dragstart', handleDragStart);
                card.addEventListener('dragend', handleDragEnd);
            });
            
            // Add event listeners to all slot areas
            document.querySelectorAll('.slot-area').forEach(slot => {
                slot.addEventListener('dragover', handleDragOver);
                slot.addEventListener('dragenter', handleDragEnter);
                slot.addEventListener('dragleave', handleDragLeave);
                slot.addEventListener('drop', handleDrop);
            });
            
            console.log('Drag and drop initialized');
        }

        // Drag Start Handler
        function handleDragStart(e) {
            draggedElement = this;
            draggedDeviceId = this.getAttribute('data-device-id');
            draggedFromSlot = parseInt(this.getAttribute('data-slot'));
            
            console.log('Drag start:', draggedDeviceId, 'from slot', draggedFromSlot);
            
            // Create drag preview
            createDragPreview(this, e);
            
            // Set drag data
            e.dataTransfer.effectAllowed = 'move';
            e.dataTransfer.setData('text/plain', draggedDeviceId);
            
            // Add visual feedback
            this.classList.add('dragging');
            
            // Hide default drag image (we're using our custom preview)
            const emptyImg = new Image();
            emptyImg.src = 'data:image/gif;base64,R0lGODlhAQABAIAAAAUEBAAAACwAAAAAAQABAAACAkQBADs=';
            e.dataTransfer.setDragImage(emptyImg, 0, 0);
            
            // Add visual cues to all slots
            document.querySelectorAll('.slot-area').forEach(slot => {
                const slotNumber = parseInt(slot.getAttribute('data-slot'));
                if (slotNumber !== draggedFromSlot) {
                    slot.style.transform = 'scale(1.02)';
                    slot.style.transition = 'all 0.2s ease';
                }
            });
        }

        // Create custom drag preview
        function createDragPreview(element, e) {
            dragPreview = element.cloneNode(true);
            dragPreview.classList.add('drag-preview');
            dragPreview.style.position = 'fixed';
            dragPreview.style.pointerEvents = 'none';
            dragPreview.style.zIndex = '2000';
            dragPreview.style.width = element.offsetWidth + 'px';
            dragPreview.style.height = element.offsetHeight + 'px';
            dragPreview.style.transform = 'rotate(2deg) scale(1.05)';
            dragPreview.style.opacity = '0.9';
            
            document.body.appendChild(dragPreview);
            
            // Update preview position
            updateDragPreview(e);
            
            // Add mousemove listener for preview positioning
            document.addEventListener('dragover', updateDragPreview);
        }

        // Update drag preview position
        function updateDragPreview(e) {
            if (dragPreview) {
                dragPreview.style.left = (e.clientX - dragPreview.offsetWidth / 2) + 'px';
                dragPreview.style.top = (e.clientY - dragPreview.offsetHeight / 2) + 'px';
            }
        }

        // Drag Over Handler
        function handleDragOver(e) {
            e.preventDefault();
            e.dataTransfer.dropEffect = 'move';
        }

        // Drag Enter Handler
        function handleDragEnter(e) {
            e.preventDefault();
            const targetSlot = parseInt(this.getAttribute('data-slot'));
            
            if (targetSlot !== draggedFromSlot) {
                this.classList.add('drag-over');
                const dropIndicator = this.querySelector('.drop-indicator');
                if (dropIndicator) {
                    dropIndicator.classList.add('show');
                }
            }
        }

        // Drag Leave Handler
        function handleDragLeave(e) {
            // Only remove highlight if we're actually leaving the slot area
            if (!this.contains(e.relatedTarget)) {
                this.classList.remove('drag-over');
                const dropIndicator = this.querySelector('.drop-indicator');
                if (dropIndicator) {
                    dropIndicator.classList.remove('show');
                }
            }
        }

        // Drop Handler
        function handleDrop(e) {
            e.preventDefault();
            
            const targetSlot = parseInt(this.getAttribute('data-slot'));
            const deviceId = e.dataTransfer.getData('text/plain');
            
            console.log('Drop:', deviceId, 'to slot', targetSlot);
            
            // Remove visual feedback
            this.classList.remove('drag-over');
            const dropIndicator = this.querySelector('.drop-indicator');
            if (dropIndicator) {
                dropIndicator.classList.remove('show');
            }
            
            // Don't drop on same slot
            if (targetSlot === draggedFromSlot) {
                console.log('Same slot, ignoring drop');
                return;
            }
            
            // Save slot assignment
            saveSlotAssignment(deviceId, targetSlot);
        }

        // Drag End Handler
        function handleDragEnd(e) {
            console.log('Drag end');
            
            // Remove visual feedback
            this.classList.remove('dragging');
            
            // Remove drag preview
            if (dragPreview) {
                document.body.removeChild(dragPreview);
                dragPreview = null;
                document.removeEventListener('dragover', updateDragPreview);
            }
            
            // Reset all slot visual cues
            document.querySelectorAll('.slot-area').forEach(slot => {
                slot.classList.remove('drag-over');
                slot.style.transform = '';
                slot.style.transition = '';
                const dropIndicator = slot.querySelector('.drop-indicator');
                if (dropIndicator) {
                    dropIndicator.classList.remove('show');
                }
            });
            
            // Reset drag state
            draggedElement = null;
            draggedDeviceId = null;
            draggedFromSlot = null;
        }

        // Save slot assignment to server
        function saveSlotAssignment(deviceId, newSlot) {
            console.log('Saving slot assignment:', deviceId, 'to slot', newSlot);
            
            const url = '/save_slot_assignment?deviceId=' + encodeURIComponent(deviceId) + '&newSlot=' + newSlot;
            
            fetch(url)
                .then(response => {
                    console.log('Response status:', response.status);
                    if (response.ok) {
                        console.log('Slot assignment saved successfully');
                        // Just reload device configs and update cards - NO page reload
                        loadDeviceConfigs();
                        setTimeout(() => {
                            updateDeviceCards();
                        }, 300);
                    } else {
                        console.error('Server responded with error:', response.status);
                        alert('Error saving slot assignment: ' + response.status);
                    }
                    return response.text();
                })
                .then(responseText => {
                    console.log('Response body:', responseText);
                })
                .catch(error => {
                    console.error('Network error:', error);
                    alert('Network error: ' + error.message);
                });
        }

        // Also reinitialize drag and drop after I2C scan
        function scanI2C(){
            fetch('/scan_i2c_ajax').then(()=>{
                setTimeout(()=>{
                    loadDeviceConfigs();
                    location.reload();
                },1000);
            });
        }

        // Touch support for mobile devices (optional enhancement)
        function addTouchSupport() {
            let touchStartX, touchStartY;
            let touchElement = null;
            
            document.querySelectorAll('.drag-handle').forEach(handle => {
                handle.addEventListener('touchstart', function(e) {
                    e.preventDefault();
                    touchElement = this.closest('.deviceCard');
                    const touch = e.touches[0];
                    touchStartX = touch.clientX;
                    touchStartY = touch.clientY;
                    
                    touchElement.style.transition = 'none';
                    touchElement.style.zIndex = '1000';
                    touchElement.style.opacity = '0.8';
                });
                
                handle.addEventListener('touchmove', function(e) {
                    if (!touchElement) return;
                    e.preventDefault();
                    
                    const touch = e.touches[0];
                    const deltaX = touch.clientX - touchStartX;
                    const deltaY = touch.clientY - touchStartY;
                    
                    touchElement.style.transform = `translate(${deltaX}px, ${deltaY}px) scale(1.05)`;
                    
                    // Highlight drop zones
                    document.elementFromPoint(touch.clientX, touch.clientY)?.closest('.slot-area')?.classList.add('drag-over');
                });
                
                handle.addEventListener('touchend', function(e) {
                    if (!touchElement) return;
                    e.preventDefault();
                    
                    const touch = e.changedTouches[0];
                    const dropTarget = document.elementFromPoint(touch.clientX, touch.clientY)?.closest('.slot-area');
                    
                    if (dropTarget) {
                        const targetSlot = parseInt(dropTarget.getAttribute('data-slot'));
                        const deviceId = touchElement.getAttribute('data-device-id');
                        const fromSlot = parseInt(touchElement.getAttribute('data-slot'));
                        
                        if (targetSlot !== fromSlot) {
                            saveSlotAssignment(deviceId, targetSlot);
                        }
                    }
                    
                    // Reset touch element
                    touchElement.style.transition = '';
                    touchElement.style.transform = '';
                    touchElement.style.zIndex = '';
                    touchElement.style.opacity = '';
                    touchElement = null;
                    
                    // Remove all drag-over classes
                    document.querySelectorAll('.slot-area').forEach(slot => {
                        slot.classList.remove('drag-over');
                    });
                });
            });
        }


function onConfigTypeChange() {
    const configType = document.getElementById('configType');
    const midiSection = document.getElementById('midiConfigSection');
    const deviceSection = document.getElementById('deviceConfigSection');
    
    const tabs = document.getElementById('layerTabs');
    const oscSection = document.getElementById('oscActionSection');
    oscSection.style.display = 'none';
    if(configType.value === 'midi') {
        midiSection.style.display = 'block';
        deviceSection.style.display = 'none';
        if(tabs) tabs.style.display = 'flex';   // MIDI is per-layer
        return loadMidiSettings(true);
    } else if(configType.value === 'osc') {
        midiSection.style.display = 'none';
        deviceSection.style.display = 'none';
        oscSection.style.display = 'block';
        if(tabs) tabs.style.display = isPedalButtonSelected() ? 'none' : 'flex';   // OSC is per-layer too
        return loadOscSettings();
    } else {
        midiSection.style.display = 'none';
        deviceSection.style.display = 'block';
        if(tabs) tabs.style.display = 'none';   // layer switching is the same on every layer
        return loadDeviceControlSettings();
    }
    // NOTE: save button managed separately — NOT re-enabled here
}
        function setConfigType(type) {
            document.getElementById('configType').value = type;
            const pm = document.getElementById('pillMidi');
            const pd = document.getElementById('pillDevice');
            const po = document.getElementById('pillOsc');
            if(pm) pm.classList.toggle('active', type === 'midi');
            if(po) po.classList.toggle('active', type === 'osc');
            if(pd) pd.classList.toggle('active', type === 'device');
            return onConfigTypeChange();
            // Note: no auto-save here. Just viewing the other panel must not overwrite
            // the button's config; a save fires when a field in the panel is changed.
        }
        function onConfigTypeUserChange() { setConfigType(document.getElementById('configType').value); }

        // Clicking a pill is an explicit choice, so it takes effect immediately:
        // "Switch Layers" saves the shown layer settings; "Send MIDI" restores MIDI.
        function onPillClick(type) {
            if(currentDevice < 0 || currentButton < 0 || currentIsPedal || selectedIsEncoder) return;
            if(document.getElementById('configType').value === type) return;
            setConfigType(type).then(() => {
                if(type === 'device') saveDeviceControlSettings();
                else if(type === 'osc') saveOscSettings();
                else saveMidiSettings();
            });
        }

function onMidiSettingsChange() {
    safeDisableButton('saveBtn', false);
}

function onDeviceControlSettingsChange() {
    safeDisableButton('saveBtn', false);
}

function loadMidiSettings(skipDeviceControlCheck = false) {
    if(currentDevice === -1 || currentButton === -1) return;
    return fetch(`/get_layer_midi?device=${currentDevice}&layer=${currentLayer}&button=${currentButton}`)
        .then(r => r.json())
        .then(data => {
            const dc = data.deviceControl || (data.type === 'device_control' ? data.controlType : 'none');
            if(!skipDeviceControlCheck && dc && dc !== 'none') {
                return setConfigType('device');
            }
            if(data.type === 'device_control') data = {};   // old firmware response: no MIDI fields
            document.getElementById('midiType').value = data.type || 'note';
            document.getElementById('midiNumber').value = data.number || 0;
            document.getElementById('midiChannel').value = data.channel || 1;
            document.getElementById('midiBehavior').value = data.behavior || 'momentary';
        })
        .catch(err => {
            console.error('loadMidiSettings error:', err);
            document.getElementById('midiType').value = 'note';
            document.getElementById('midiNumber').value = 60 + currentButton;
            document.getElementById('midiChannel').value = 1;
            document.getElementById('midiBehavior').value = 'momentary';
        });
}

function saveMidiSettings() {
    if(currentDevice === -1 || currentButton === -1) return;
    
    const type = document.getElementById('midiType').value;
    const number = parseInt(document.getElementById('midiNumber').value);
    const channel = parseInt(document.getElementById('midiChannel').value);
    const behavior = document.getElementById('midiBehavior').value;
    
    // Silently skip invalid/incomplete values; auto-save fires again on the next change
    if(isNaN(number) || number < 0 || number > 127) return;
    if(isNaN(channel) || channel < 1 || channel > 16) return;
    
    const payload = { device: currentDevice, layer: currentLayer, button: currentButton,
        type, number, channel, behavior, clearDeviceControl: true };
    console.log('Saving MIDI:', JSON.stringify(payload));
    fetch('/save_layer_midi', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)})
    .then(r => {
        if(r.ok) {
            safeDisableButton('saveBtn', true);
            const fb = document.getElementById('saveFeedback');
            if(fb) { fb.classList.add('visible'); setTimeout(()=>fb.classList.remove('visible'),1500); }
            setTimeout(updateDeviceCards, 500);
        } else { r.text().then(t=>alert('Save failed: '+r.status+' '+t)); }
    })
    .catch(err=>alert('Save error: '+err.message))
    .catch(err => {
        console.error('Error saving MIDI settings:', err);
        alert('Error saving MIDI settings.');
    });
}

// Update the main saveSettings function
function saveSettings() {
    const configType = document.getElementById('configType') ? document.getElementById('configType').value : 'midi';
    
    if (currentIsPedal) {
        savePedalSettings();
    } else if (typeof selectedIsEncoder !== 'undefined' && selectedIsEncoder) {
        saveEncoderConfiguration();
    } else if (selectedLED >= 0) {
        saveLEDConfiguration();
    } else if (configType === 'midi') {
        saveMidiSettings();
        saveLEDIfVisible();
    } else if (configType === 'osc') {
        saveOscSettings();
        saveLEDIfVisible();
    } else {
        saveDeviceControlSettings();
        saveLEDIfVisible();
    }
}

// The LED panel is shown alongside the MIDI settings when a button is
// selected, but there are no separate LED icons to click, so selectedLED
// stays -1 and LED changes were never persisted. Save them with the button.
function saveLEDIfVisible() {
    const ledSec = document.getElementById('ledConfigSection');
    if (ledSec && ledSec.style.display !== 'none') {
        saveLEDConfiguration();
    }
}

let serialUpdateInterval = null;

function toggleWebSerial() {
    const enabled = document.getElementById('webSerialToggle').checked;
    fetch('/toggle_web_serial?enabled=' + enabled);
    
    if (enabled) {
        startSerialUpdates();
    } else {
        stopSerialUpdates();
    }
}

function startSerialUpdates() {
    if (serialUpdateInterval) clearInterval(serialUpdateInterval);
    
    serialUpdateInterval = setInterval(() => {
        fetch('/get_serial_log')
            .then(r => r.json())
            .then(data => {
                const output = document.getElementById('serialOutput');
                if (output) {
                    let logText;
                    if (data.encoded) {
                        // Decode base64
                        logText = atob(data.log);
                    } else {
                        logText = data.log;
                    }
                    
                    output.textContent = logText;
                    // Auto-scroll to bottom
                    const monitor = document.getElementById('serialMonitor');
                    monitor.scrollTop = monitor.scrollHeight;
                }
            })
            .catch(err => {
                console.error('Serial update error:', err);
                // Show error in the monitor
                const output = document.getElementById('serialOutput');
                if (output) {
                    output.textContent += '\n[ERROR] Failed to update serial log\n';
                }
            });
    }, 500);
}

function stopSerialUpdates() {
    if (serialUpdateInterval) {
        clearInterval(serialUpdateInterval);
        serialUpdateInterval = null;
    }
}

function clearSerialLog() {
    fetch('/clear_serial_log').then(() => {
        document.getElementById('serialOutput').textContent = '';
    });
}

function changeMainTheme() {
            let color = document.getElementById('mainThemeSelect').value;
            fetch('/save_theme?color=' + encodeURIComponent(color))
                .then(() => {
                    updateThemeColors(color);
                    console.log('Main theme changed to:', color);
                });
        }

        function changeLayerColor(layerIndex, color) {
            const layerNum = layerIndex + 1;
            document.documentElement.style.setProperty(`--layer${layerNum}-color`, color);
            
            fetch('/save_layer_color?layer=' + layerIndex + '&color=' + encodeURIComponent(color))
                .then(response => {
                    if(response.ok) {
                        console.log(`Layer ${layerNum} color changed to:`, color);
                        updateColorLabel(layerIndex, color);
                    }
                });
        }

        function updateColorLabel(layerIndex, color) {
            const colorNames = {
                '#F44336': 'Red', '#E91E63': 'Pink', '#9C27B0': 'Purple', '#673AB7': 'Deep Purple',
                '#3F51B5': 'Indigo', '#2196F3': 'Blue', '#03A9F4': 'Light Blue', '#00BCD4': 'Cyan',
                '#009688': 'Teal', '#4CAF50': 'Green', '#8BC34A': 'Light Green', '#CDDC39': 'Lime',
                '#FFEB3B': 'Yellow', '#FFC107': 'Amber', '#FF9800': 'Orange', '#FF5722': 'Deep Orange'
            };
            
            const label = document.querySelector(`#layer${layerIndex + 1}Color`).parentNode.querySelector('.color-label');
            label.textContent = colorNames[color] || 'Custom';
        }

        function resetThemeDefaults() {
    if(confirm('Reset all theme colors to defaults?')) {
        document.getElementById('mainThemeSelect').value = '#4CAF50';
        
        const defaultColors = ['#F44336', '#9C27B0', '#2196F3', '#4CAF50'];
        defaultColors.forEach((color, index) => {
            document.getElementById(`layer${index + 1}Color`).value = color;
            changeLayerColor(index, color);
        });
        
        changeMainTheme();
        
        // Refresh LED with new colors
        fetch('/refresh_led').then(() => {
            console.log('LED refreshed with default colors');
        });
        
        console.log('Theme reset to defaults');
    }
}

        function loadThemeSettings() {
            fetch('/get_theme_settings')
                .then(r => r.json())
                .then(settings => {
                    if(settings.mainColor) {
                        document.getElementById('mainThemeSelect').value = settings.mainColor;
                    }
                    
                    if(settings.layerColors) {
                        settings.layerColors.forEach((color, index) => {
                            const input = document.getElementById(`layer${index + 1}Color`);
                            if(input) {
                                input.value = color;
                                updateColorLabel(index, color);
                                document.documentElement.style.setProperty(`--layer${index + 1}-color`, color);
                            }
                        });
                    }
                })
                .catch(err => {
                    console.log('Using default theme settings');
                    const defaultColors = ['#F44336', '#9C27B0', '#2196F3', '#4CAF50'];
                    defaultColors.forEach((color, index) => {
                        document.getElementById(`layer${index + 1}Color`).value = color;
                        updateColorLabel(index, color);
                    });
                });
        }

function saveTheme() {
    const mainColor = document.getElementById('mainThemeSelect').value;
    const layerColors = [];
    for(let i = 0; i < 4; i++) {
        layerColors.push(document.getElementById(`layer${i + 1}Color`).value);
    }

    // Capture button reference now — event.target is undefined in async callbacks
    const saveBtn = document.querySelector('button[onclick="saveTheme()"]');

    fetch('/save_theme?color=' + encodeURIComponent(mainColor))
        .then(r => r.ok ? r : Promise.reject('Failed to save main theme: ' + r.status))
        .then(() => Promise.all(
            layerColors.map((color, index) =>
                fetch('/save_layer_color?layer=' + index + '&color=' + encodeURIComponent(color))
                    .then(r => r.ok ? r : Promise.reject('Failed to save layer ' + index + ': ' + r.status))
            )
        ))
        .then(() => {
            console.log('Theme saved successfully');
            if(saveBtn) {
                const origText = saveBtn.textContent;
                saveBtn.textContent = 'Saved!';
                saveBtn.style.backgroundColor = '#4CAF50';
                saveBtn.disabled = true;
                setTimeout(() => {
                    saveBtn.textContent = origText;
                    saveBtn.style.backgroundColor = '';
                    saveBtn.disabled = false;
                }, 2000);
            }
        })
        .catch(error => {
            console.error('Error saving theme:', error);
            alert('Error saving theme: ' + error);
        });
}




function onEncoderModeChange() {
    const encoderMode = document.getElementById('encoderMode').value;
    const midiOptions = document.getElementById('encoderMidiOptions');
    
    if (encoderMode === 'layer_control') {
        midiOptions.style.display = 'none';
    } else {
        midiOptions.style.display = 'block';
    }
    
    safeDisableButton('saveBtn', false);
}





function onEncoderRotationModeChange() {
    const rotationMode = document.getElementById('encoderRotationMode').value;
    const midiOptions = document.getElementById('encoderMidiOptions');
    const noteOptions = document.getElementById('encoderNoteOptions');
    
    if (rotationMode === 'layer_device' || rotationMode === 'layer_global') {
        midiOptions.style.display = 'none';
        noteOptions.style.display = 'none';
    } else if (rotationMode === 'midi_cc') {
        midiOptions.style.display = 'block';
        noteOptions.style.display = 'none';
    } else if (rotationMode === 'midi_note') {
        midiOptions.style.display = 'none';
        noteOptions.style.display = 'block';
    } else if (rotationMode === 'osc') {
        midiOptions.style.display = 'none';
        noteOptions.style.display = 'none';
    }
    const oscOpt = document.getElementById('encoderOscOptions');
    if (oscOpt) oscOpt.style.display = rotationMode === 'osc' ? 'block' : 'none';
    
    safeDisableButton('saveBtn', false);
}

function onEncoderButtonModeChange() {
    const buttonMode = document.getElementById('encoderButtonMode').value;
    const buttonMidiOptions = document.getElementById('encoderButtonMidiOptions');
    
    buttonMidiOptions.style.display = (buttonMode === 'midi_momentary' || buttonMode === 'midi_latching') ? 'block' : 'none';
    const bo = document.getElementById('encoderButtonOscOptions');
    if (bo) bo.style.display = buttonMode === 'osc' ? 'block' : 'none';
    
    safeDisableButton('saveBtn', false);
}

// Function to select encoder device
function selectEncoder(slot) {
    flushAutoSave();
    lastSelectTime = Date.now();
    currentDevice = slot;
    currentButton = 0; // Encoders use button 0 for configuration
    currentIsPedal = false;
    selectedIsEncoder = true;
    
    // Initialize with layer 0 colors for encoders
    const body = document.body;
    body.classList.remove('layer-color-theme', 'layer-0', 'layer-1', 'layer-2', 'layer-3');
    body.classList.add('layer-color-theme', 'layer-0');
    
    // Clear previous selections
    document.querySelectorAll('.pad').forEach(p => p.classList.remove('selected'));
    document.querySelectorAll('.pad-vertical').forEach(p => p.classList.remove('selected'));
    document.querySelectorAll('.pedal-container').forEach(p => p.style.border = 'none');
    document.querySelectorAll('.encoder-container').forEach(p => p.style.border = 'none');
    
    // Highlight selected encoder
    const encoderContainer = document.querySelector(`[data-slot="${slot}"] .encoder-container`);
    if (encoderContainer) {
        encoderContainer.style.border = '2px solid %THEME_COLOR%';
    }
    
    // Get device info
    fetch('/get_slot_mappings').then(r => r.json()).then(mappings => {
        const slotData = mappings.mappings.find(m => m.slot === slot);
        if (slotData && slotData.deviceId) {
            const deviceName = slotData.deviceType + " " + slotData.deviceId.substring(4);
            document.getElementById('deviceIDLabel').innerText = deviceName;
            document.getElementById('buttonLabel').innerText = 'Encoder';
            
            hideAllPanels();
            document.getElementById('encoderConfigSection').style.display = 'block';
            
            // Load encoder configuration
            loadEncoderConfiguration(slot);
            
            // Show sidebar
            const sidebar = document.getElementById('sidebar');
            if (sidebar) {
                sidebar.style.display = 'flex';
                document.body.style.marginRight = '320px';
            }

        }
    });
}

function loadEncoderConfiguration(slot) {
    fetch(`/get_encoder_config?device=${slot}`)
        .then(r => r.json())
        .then(data => {
            // Encoder push button
            document.getElementById('encoderButtonMode').value = data.buttonMode || 'layer_toggle';
            document.getElementById('encoderButtonNote').value = data.buttonMidiNote != null ? data.buttonMidiNote : 60;
            document.getElementById('encoderButtonChannel').value = data.buttonMidiChannel || 1;
            document.getElementById('encoderOscAddr').value = data.oscAddr || '/encoder';
            document.getElementById('encoderOscMode').value = data.oscMode || 'relative';
            document.getElementById('encoderButtonOscAddr').value = data.buttonOscAddr || '/encoder/button';
            
            // Load encoder rotation configuration
            document.getElementById('encoderRotationMode').value = data.rotationMode || 'layer_global';
            document.getElementById('encoderMidiCC').value = data.midiCC || 1;
            document.getElementById('encoderMidiChannel').value = data.midiChannel || 1;
            document.getElementById('encoderMinValue').value = data.minValue || 0;
            document.getElementById('encoderMaxValue').value = data.maxValue || 127;
            document.getElementById('encoderNoteNumber').value = data.noteNumber || 60;
            document.getElementById('encoderNoteChannel').value = data.noteChannel || 1;
            document.getElementById('encoderInvertDirection').checked = data.invertDirection || false;
            document.getElementById('encoderStepsPerDetent').value = data.stepsPerDetent || 4;
            
            // Update UI visibility
            populating = true;
            onEncoderRotationModeChange();
            onEncoderButtonModeChange();
            populating = false;
        })
        .catch(err => {
            console.error('Error loading encoder config:', err);
            // Set defaults
            document.getElementById('encoderButtonMode').value = 'layer_toggle';
            document.getElementById('encoderRotationMode').value = 'layer_global';
            populating = true;
            onEncoderRotationModeChange();
            onEncoderButtonModeChange();
            populating = false;
        });
}

function saveEncoderConfiguration() {
    if (currentDevice === -1) return;
    
    const payload = {
        device: currentDevice,
        // Encoder push button
        buttonMode: document.getElementById('encoderButtonMode').value,
        buttonNumber: parseInt(document.getElementById('encoderButtonNote').value) || 0,
        buttonChannel: parseInt(document.getElementById('encoderButtonChannel').value) || 1,
        oscAddr: document.getElementById('encoderOscAddr').value.trim(),
        oscMode: document.getElementById('encoderOscMode').value,
        buttonOscAddr: document.getElementById('encoderButtonOscAddr').value.trim(),
        
        // Encoder rotation configuration
        rotationMode: document.getElementById('encoderRotationMode').value,
        midiCC: parseInt(document.getElementById('encoderMidiCC').value) || 1,
        midiChannel: parseInt(document.getElementById('encoderMidiChannel').value) || 1,
        minValue: parseInt(document.getElementById('encoderMinValue').value) || 0,
        maxValue: parseInt(document.getElementById('encoderMaxValue').value) || 127,
        noteNumber: parseInt(document.getElementById('encoderNoteNumber').value) || 60,
        noteChannel: parseInt(document.getElementById('encoderNoteChannel').value) || 1,
        invertDirection: document.getElementById('encoderInvertDirection').checked,
        stepsPerDetent: parseInt(document.getElementById('encoderStepsPerDetent').value) || 4
    };
    
    fetch('/save_encoder_config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
    })
    .then(response => {
        if (response.ok) {
            safeDisableButton('saveBtn', true);
            showSavedFeedback();
            console.log('Encoder configuration saved');
        }
    })
    .catch(err => {
        console.error('Error saving encoder config:', err);
        alert('Error saving encoder configuration.');
    });
}


function selectLED(slot, button) {
    currentDevice = slot;
    currentButton = button;
    selectedLED = button;
    currentIsPedal = false;
    selectedIsEncoder = false;
    
    // Clear other selections
    document.querySelectorAll('.pad').forEach(p => p.classList.remove('selected'));
    document.querySelectorAll('.pad-vertical').forEach(p => p.classList.remove('selected'));
    document.querySelectorAll('.led-icon').forEach(l => l.classList.remove('selected'));
    
    // Highlight selected LED
    const ledIcon = document.querySelector(`#led${slot}b${button}`);
    if(ledIcon) {
        ledIcon.classList.add('selected');
    }
    
    // Get device info and show LED config
    fetch('/get_slot_mappings').then(r => r.json()).then(mappings => {
        const slotData = mappings.mappings.find(m => m.slot === slot);
        if(slotData) {
            document.getElementById('deviceIDLabel').innerText = slotData.deviceType + " " + slotData.deviceId.substring(4);
            document.getElementById('buttonLabel').innerText = `Button ${button + 1} LED`;
            
            // Hide other sections
            document.getElementById('layerTabs').style.display = 'none';
            document.getElementById('midiConfigSection').style.display = 'none';
            document.getElementById('deviceConfigSection').style.display = 'none';
            document.getElementById('calSection').style.display = 'none';
            document.getElementById('encoderConfigSection').style.display = 'none';
            
            // Show LED section
            document.getElementById('ledConfigSection').style.display = 'block';
            
            // Load LED configuration
            loadLEDConfiguration(slot, button);
            
            // Show sidebar
            document.getElementById('sidebar').style.display = 'flex';
            document.body.style.marginRight = '320px';
            
            safeDisableButton('saveBtn', false);
        }
    });
}

function loadLEDConfiguration(slot, button) {
    fetch(`/get_led_config?device=${slot}&button=${button}`)
        .then(r => r.json())
        .then(data => {
            document.getElementById('ledMode').value = data.mode || 'button_state';
            document.getElementById('ledFollowLayer').checked = data.followLayer !== false;
            document.getElementById('ledCustomColor').value = data.color || '#FFFFFF';
            document.getElementById('ledMidiNote').value = data.midiNote || 60;
            document.getElementById('ledMidiChannel').value = data.midiChannel || 1;
            document.getElementById('ledOscAddr').value = data.oscAddr || ('/led/' + (button + 1));
            
            populating = true;
            onLEDModeChange();
            onLEDFollowLayerChange();
            populating = false;
        })
        .catch(err => { populating = false; console.error('Error loading LED config:', err); });
}

function onLEDModeChange() {
    const mode = document.getElementById('ledMode').value;
    const ledOscOpt = document.getElementById('ledOscOptions');
    if (ledOscOpt) ledOscOpt.style.display = mode === 'osc_in' ? 'block' : 'none';
    const colorOptions = document.getElementById('ledColorOptions');
    const midiOptions = document.getElementById('ledMidiOptions');
    
    if (mode === 'off') {
        colorOptions.style.display = 'none';
        midiOptions.style.display = 'none';
    } else if (mode === 'midi_in') {
        colorOptions.style.display = 'block';
        midiOptions.style.display = 'block';
    } else if (mode === 'osc_in') {
        colorOptions.style.display = 'block';
        midiOptions.style.display = 'none';
    } else {
        colorOptions.style.display = 'block';
        midiOptions.style.display = 'none';
    }
    
    safeDisableButton('saveBtn', false);
}

function onLEDFollowLayerChange() {
    const followLayer = document.getElementById('ledFollowLayer').checked;
    const customOption = document.getElementById('ledCustomColorOption');
    
    customOption.style.display = followLayer ? 'none' : 'block';
    safeDisableButton('saveBtn', false);
}

function saveLEDConfiguration() {
    const ledBtn = (selectedLED >= 0) ? selectedLED : currentButton;
    if (currentDevice === -1 || ledBtn === -1 || ledBtn === undefined) return;
    
    const payload = {
        device: currentDevice,
        button: ledBtn,
        mode: document.getElementById('ledMode').value,
        color: document.getElementById('ledCustomColor').value,
        followLayer: document.getElementById('ledFollowLayer').checked,
        midiNote: parseInt(document.getElementById('ledMidiNote').value) || 60,
        midiChannel: parseInt(document.getElementById('ledMidiChannel').value) || 1,
        oscAddr: document.getElementById('ledOscAddr').value.trim()
    };
    
    fetch('/save_led_config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload)
    })
    .then(response => {
        if (response.ok) {
            safeDisableButton('saveBtn', true);
            showSavedFeedback();
            console.log('LED configuration saved');
        }
    })
    .catch(err => {
        console.error('Error saving LED config:', err);
        alert('Error saving LED configuration.');
    });
}


    </script>

    <style>
        /* Layer-specific colors */
:root {
    --layer1-color: #F44336;
    --layer2-color: #9C27B0;
    --layer3-color: #2196F3;
    --layer4-color: #4CAF50;
}
        
        body{font-family:'Roboto',sans-serif;padding:24px;background:#1C1B1F;color:#E6E1E5;margin-left:60px;margin-right:24px;line-height:1.5;}
        body.devices-active{margin-right:320px;}
        .status-card{background:#211F26;border-radius:12px;padding:16px;margin-bottom:24px;box-shadow:0 1px 3px rgba(0,0,0,0.12);}
        .status-indicator{display:flex;align-items:center;gap:12px;}.status-dot{width:8px;height:8px;border-radius:50%;background:#4CAF50;}
        .status-text{font-size:14px;font-weight:500;color:#CAC4D0;}
        .section-card{background:#211F26;border-radius:12px;padding:24px;margin-bottom:24px;box-shadow:0 1px 3px rgba(0,0,0,0.12);}
        .section-title{font-size:20px;font-weight:500;color:#E6E1E5;margin:0 0 20px 0;}
        
        .device-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(240px,1fr));gap:16px;margin-bottom:24px;}
        .slot-area{min-height:300px;border:2px dashed #49454F;border-radius:16px;padding:16px;background:#211F26;transition:all 0.2s ease;position:relative;}
        .slot-area.drag-over{border-color:%THEME_COLOR%;background:%LIGHT_BACKGROUND%;box-shadow:0 4px 12px rgba(0,0,0,0.15);}
        .slot-header{display:flex;align-items:center;justify-content:space-between;margin-bottom:16px;padding:8px 12px;background:#2B2930;border-radius:8px;}
        .slot-title{font-size:14px;font-weight:600;color:#CAC4D0;letter-spacing:0.25px;}
        .slot-badge{background:%THEME_COLOR%;color:#FFFFFF;padding:4px 8px;border-radius:12px;font-size:12px;font-weight:500;}

        .deviceCard{display:block;width:100%;background:#2B2930;border:1px solid #49454F;border-radius:16px;padding:16px;transition:all 0.2s ease;cursor:default;position:relative;margin-bottom:12px;}
        .deviceCard:hover{border-color:%THEME_COLOR%;box-shadow:0 2px 8px rgba(0,0,0,0.12);}
        .deviceCard.dragging{opacity:0.6;transform:scale(0.98);box-shadow:0 8px 24px rgba(0,0,0,0.3);z-index:1000;}
        .deviceCard.drag-preview{position:fixed;pointer-events:none;z-index:2000;transform:rotate(2deg);box-shadow:0 12px 32px rgba(0,0,0,0.4);}

        .device-header{display:flex;align-items:flex-start;justify-content:space-between;margin-bottom:12px;}
        .device-title{font-weight:500;font-size:14px;color:#E6E1E5;line-height:1.4;flex:1;}
        .drag-handle{width:24px;height:24px;border-radius:50%;background:#49454F;display:flex;align-items:center;justify-content:center;cursor:grab;transition:all 0.15s ease;margin-left:8px;flex-shrink:0;}
        .drag-handle:hover{background:%THEME_COLOR%;color:#FFFFFF;transform:scale(1.1);}
        .drag-handle:active{cursor:grabbing;transform:scale(0.95);}
        .drag-handle .material-icons{font-size:16px;}

        .pads{display:grid;grid-template-columns: 1fr 1fr; grid-template-rows: 1fr 1fr; gap: 8px; margin-top: 12px; width: 88px; height: 88px;}
        .pad{width:40px;height:40px;border-radius:12px;background:#49454F;transition:all 0.15s ease;cursor:pointer;position:relative;border:1px solid transparent;}
        .pad:hover{background:#625B71;border-color:#79747E;transform:translateY(-1px);}
        .pad.active{background:%THEME_COLOR%;border-color:%THEME_COLOR%;box-shadow:0 2px 8px %THEME_COLOR_RGBA%;}
        .pad.selected{outline:2px solid %THEME_COLOR%;outline-offset:2px;background:%LIGHT_BACKGROUND%;}
        .pad.selected.active{background:%THEME_COLOR%;outline-color:%LIGHT_BACKGROUND%;}
        .pedal-container{margin-top:12px;padding:16px;background:#332D41;border-radius:8px;}
        .pedal-slider{width:100%;height:6px;background:#49454F;border-radius:3px;position:relative;cursor:pointer;}
        .pedal-thumb{position:absolute;top:-6px;width:18px;height:18px;background:%THEME_COLOR%;border-radius:50%;cursor:grab;transition:all 0.1s ease;}
        .pedal-thumb:hover{background:%LIGHTER_COLOR%;transform:scale(1.1);}
        .pedal-value{text-align:center;margin-top:8px;font-size:12px;color:#CAC4D0;font-family:monospace;}
        .pedal-label{font-size:12px;color:#E6E1E5;margin-bottom:8px;text-align:center;cursor:pointer;}
        .pedal-label:hover{color:%THEME_COLOR%;}

        .empty-slot{display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:200px;color:#79747E;text-align:center;}
        .empty-slot .material-icons{font-size:48px;margin-bottom:12px;opacity:0.6;}
        .empty-slot-text{font-size:14px;font-weight:500;}

        .drag-ghost{opacity:0.4;pointer-events:none;}
        .drop-indicator{position:absolute;top:0;left:0;right:0;bottom:0;border:2px solid %THEME_COLOR%;border-radius:16px;background:%THEME_COLOR_RGBA_LIGHT%;display:none;pointer-events:none;}
        .drop-indicator.show{display:block;}

        .wifi-form{display:flex;flex-direction:column;gap:16px;}.input-group{display:flex;flex-direction:column;gap:8px;}
        .input-label{font-size:12px;font-weight:500;color:#CAC4D0;text-transform:uppercase;letter-spacing:0.5px;}
        .text-field{background:#2B2930;border:1px solid #79747E;border-radius:4px;padding:12px 16px;font-size:16px;color:#E6E1E5;transition:all 0.15s ease;outline:none;}
        .text-field:focus{border-color:%THEME_COLOR%;box-shadow:0 0 0 1px %THEME_COLOR%;}
        .text-field::placeholder{color:#79747E;}
        .btn-primary{background:%THEME_COLOR%;color:#FFFFFF;border:none;border-radius:20px;padding:12px 24px;font-size:14px;font-weight:500;cursor:pointer;transition:all 0.15s ease;min-width:64px;}
        .btn-primary:hover:not(:disabled){background:%LIGHTER_COLOR%;box-shadow:0 2px 4px rgba(0,0,0,0.15);}
        .btn-primary:disabled{background:#49454F;color:#79747E;cursor:not-allowed;}
        .btn-secondary{background:transparent;color:%THEME_COLOR%;border:1px solid %THEME_COLOR%;border-radius:20px;padding:12px 24px;font-size:14px;font-weight:500;cursor:pointer;transition:all 0.15s ease;}
        .btn-secondary:hover{background:%THEME_COLOR_RGBA_LIGHT%;}.section-header{display:flex;justify-content:space-between;align-items:center;margin-bottom:20px;}

        #sidebar{position:fixed;right:0;top:0;width:320px;height:100vh;background:#211F26;border-left:1px solid #49454F;padding:0;z-index:1000;color:#E6E1E5;overflow-y:auto;display:none;flex-direction:column;}
        #sidebar.show{display:flex;}
        .sidebar-header{padding:24px 24px 0;border-bottom:1px solid #49454F;}.sidebar-title{font-size:20px;font-weight:500;color:#E6E1E5;margin:0 0 24px 0;}
        .sidebar-content{padding:24px;flex:1;}.info-card{background:#2B2930;border-radius:8px;padding:16px;margin-bottom:24px;}
        .device-info{font-size:14px;color:#E6E1E5;margin:0 0 8px 0;font-weight:500;}.button-info{font-size:12px;color:#CAC4D0;margin:0;} 
        .form-section{display:flex;flex-direction:column;gap:20px;}

        #leftSidebar{position:fixed;left:0;top:0;width:60px;height:100vh;background:#211F26;border-right:1px solid #49454F;z-index:1000;display:flex;flex-direction:column;justify-content:space-between;align-items:center;padding:16px 0;}
        .icon{width:40px;height:40px;margin:8px;background:#49454F;border-radius:12px;display:flex;align-items:center;justify-content:center;cursor:pointer;transition:all 0.15s ease;color:#CAC4D0;font-size:16px;font-weight:500;}
        .icon:hover{background:%THEME_COLOR%;color:#FFFFFF;}.icon.active{background:%THEME_COLOR%;color:#FFFFFF;box-shadow:0 2px 6px rgba(0,0,0,0.15);}
        .tab-content{display:none;}.tab-content.active{display:block;}.midi-channels{display:flex;flex-direction:column;gap:12px;}
        .map-table{width:100%;border-collapse:collapse;font-size:12px;}
        .map-table th,.map-table td{padding:6px 10px;border-bottom:1px solid #313244;text-align:left;white-space:nowrap;vertical-align:top;}
        .map-table th{color:#CAC4D0;font-weight:500;text-transform:uppercase;font-size:10px;letter-spacing:0.5px;}
        .map-table td{color:#E6E1E5;}
        .map-table .dup{color:#FF8A80;font-weight:600;}
        .map-table .dim{color:#79747E;}

        .toggle-label {
            display: flex !important;
            align-items: center;
            gap: 12px;
            cursor: pointer;
            font-size: 14px !important;
            color: #E6E1E5 !important;
            text-transform: none !important;
            letter-spacing: normal !important;
        }

        .toggle-slider {
            position: relative;
            width: 44px;
            height: 24px;
            background: #49454F;
            border-radius: 12px;
            transition: all 0.3s ease;
            display: flex;
            align-items: center;
            flex-shrink: 0;
        }

        .toggle-slider::before {
            content: '';
            position: absolute;
            width: 18px;
            height: 18px;
            background: #CAC4D0;
            border-radius: 50%;
            left: 3px;
            transition: all 0.3s ease;
        }

        input[type="checkbox"]:checked + .toggle-slider {
            background: %THEME_COLOR%;
        }

        input[type="checkbox"]:checked + .toggle-slider::before {
            transform: translateX(20px);
            background: #FFFFFF;
        }

        input[type="checkbox"] {
            display: none;
        }

        .advanced-section {
            margin-top: 24px;
            padding: 20px;
            background: #2B2930;
            border-radius: 12px;
            border: 1px solid #49454F;
        }

        .advanced-divider {
            height: 1px;
            background: #49454F;
            margin: 20px 0;
        }

        .advanced-title {
            font-size: 16px;
            font-weight: 500;
            color: %THEME_COLOR%;
            margin: 0 0 16px 0;
        }

        .layer-tabs {
            display: flex;
            gap: 0;
            border-radius: 8px;
            overflow: hidden;
            background: #49454F;
            margin-bottom: 20px;
        }

        .layer-tab {
    flex: 1;
    padding: 12px 8px;
    text-align: center;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    background: #49454F;
    color: #CAC4D0;
    border-right: 1px solid #79747E;
    transition: all 0.2s ease;
}

.layer-tab:last-child {
    border-right: none;
}

.layer-tab:hover {
    background: #625B71;
}

.layer-tab:nth-child(1).active {
    background: var(--layer1-color);
    color: #FFFFFF;
}

.layer-tab:nth-child(2).active {
    background: var(--layer2-color);
    color: #FFFFFF;
}

.layer-tab:nth-child(3).active {
    background: var(--layer3-color);
    color: #FFFFFF;
}

.layer-tab:nth-child(4).active {
    background: var(--layer4-color);
    color: #FFFFFF;
}

        .layer-tab.has-config {
            position: relative;
        }

        .layer-tab.has-config::after {
            content: '•';
            position: absolute;
            top: 2px;
            right: 4px;
            color: #f5b227;
            font-size: 16px;
        }

        .shift-indicator {
            display: inline-block;
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background: #79747E;
            margin-left: 8px;
            transition: all 0.2s ease;
        }

        .shift-indicator.active {
            background: #4CAF50;
            box-shadow: 0 0 4px #4CAF50;
        }

        .device-control-badge {
            display: inline-block;
            padding: 2px 6px;
            border-radius: 4px;
            font-size: 10px;
            font-weight: 500;
            margin-left: 8px;
        }

        .device-control-badge.shift {
            background: #4CAF50;
            color: #FFFFFF;
        }

        .device-control-badge.standard {
            background: #79747E;
            color: #CAC4D0;
        }

        /* Enhanced pad styles for shift indication */
        .pad.shift-button {
            border: 2px solid #4CAF50 !important;
            position: relative;
        }

        .pad.shift-button::before {
            content: 'SHIFT';
            position: absolute;
            bottom: -2px;
            left: 50%;
            transform: translateX(-50%);
            font-size: 8px;
            color: #4CAF50;
            font-weight: bold;
            background: #211F26;
            padding: 1px 3px;
            border-radius: 2px;
        }

        .pad.layer-active {
    box-shadow: 0 0 0 2px #f5b227;
}

.pad.layer-1 {
    box-shadow: 0 0 0 2px var(--layer1-color) !important;
    color: var(--layer1-color) !important;
    font-weight: bold;
}

.pad.layer-2 {
    box-shadow: 0 0 0 2px var(--layer2-color) !important;
    color: var(--layer2-color) !important;
    font-weight: bold;
}

.pad.layer-3 {
    box-shadow: 0 0 0 2px var(--layer3-color) !important;
    color: var(--layer3-color) !important;
    font-weight: bold;
}

.pad.layer-4 {
    box-shadow: 0 0 0 2px var(--layer4-color) !important;
    color: var(--layer4-color) !important;
    font-weight: bold;
}

.pads-vertical {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 12px;
    width: 44px;
    height: 180px;
}

.pad-vertical {
    width: 40px;
    height: 40px;
    border-radius: 12px;
    background: #49454F;
    transition: all 0.15s ease;
    cursor: pointer;
    position: relative;
    border: 1px solid transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    font-weight: bold;
}

.pad-vertical:hover {
    background: #625B71;
    border-color: #79747E;
    transform: translateY(-1px);
}

.pad-vertical.active {
    background: %THEME_COLOR%;
    border-color: %THEME_COLOR%;
    box-shadow: 0 2px 8px %THEME_COLOR_RGBA%;
}

.pad-vertical.selected {
    outline: 2px solid %THEME_COLOR%;
    outline-offset: 2px;
    background: %LIGHT_BACKGROUND%;
}

.pad-vertical.selected.active {
    background: %THEME_COLOR%;
    outline-color: %LIGHT_BACKGROUND%;
}

.pad-vertical.shift-button {
    border: 2px solid #4CAF50 !important;
    position: relative;
}

.pad-vertical.shift-button::before {
    content: 'SHIFT';
    position: absolute;
    bottom: -2px;
    left: 50%;
    transform: translateX(-50%);
    font-size: 6px;
    color: #4CAF50;
    font-weight: bold;
    background: #211F26;
    padding: 1px 2px;
    border-radius: 2px;
}

.pad-vertical.layer-1 {
    box-shadow: 0 0 0 2px var(--layer1-color) !important;
    color: var(--layer1-color) !important;
    font-weight: bold;
}

.pad-vertical.layer-2 {
    box-shadow: 0 0 0 2px var(--layer2-color) !important;
    color: var(--layer2-color) !important;
    font-weight: bold;
}

.pad-vertical.layer-3 {
    box-shadow: 0 0 0 2px var(--layer3-color) !important;
    color: var(--layer3-color) !important;
    font-weight: bold;
}

.pad-vertical.layer-4 {
    box-shadow: 0 0 0 2px var(--layer4-color) !important;
    color: var(--layer4-color) !important;
    font-weight: bold;
}

.lcd-mirror {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 200px;
}

.lcd-header {
    font-size: 12px;
    font-weight: 500;
    color: #CAC4D0;
    text-transform: uppercase;
    letter-spacing: 0.5px;
}

.lcd-screen {
    background: #0a0a0a;
    border: 2px solid #333;
    border-radius: 4px;
    padding: 12px;
    font-family: 'Courier New', monospace;
    font-size: 11px;
    line-height: 1.2;
    color: #00ff00;
    min-height: 80px;
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
    position: relative;
}

.lcd-screen::before {
    content: '';
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: repeating-linear-gradient(
        0deg,
        transparent,
        transparent 1px,
        rgba(0, 255, 0, 0.03) 1px,
        rgba(0, 255, 0, 0.03) 2px
    );
    pointer-events: none;
    border-radius: 2px;
}

.lcd-line {
    min-height: 14px;
    white-space: pre;
    overflow: hidden;
}

.lcd-line:nth-child(2) {
    font-size: 24px;
    font-weight: bold;
    text-align: center;
    line-height: 1;
    margin: 8px 0;
}

.config-section {
    margin-top: 16px;
    padding: 16px;
    background: #2B2930;
    border-radius: 8px;
    border-left: 4px solid %THEME_COLOR%;
}

.config-section .input-group {
    margin-bottom: 16px;
}

.config-section .input-group:last-child {
    margin-bottom: 0;
}


/* Dynamic layer color classes */
.layer-color-theme {
    --current-layer-color: var(--layer1-color);
}

.layer-color-theme.layer-0 {
    --current-layer-color: var(--layer1-color);
}

.layer-color-theme.layer-1 {
    --current-layer-color: var(--layer2-color);
}

.layer-color-theme.layer-2 {
    --current-layer-color: var(--layer3-color);
}

.layer-color-theme.layer-3 {
    --current-layer-color: var(--layer4-color);
}

/* Update existing elements to use the dynamic color */
.config-section {
    margin-top: 16px;
    padding: 16px;
    background: #2B2930;
    border-radius: 8px;
    border-left: 4px solid var(--current-layer-color);
    transition: border-color 0.3s ease;
}

.btn-primary {
    background: var(--current-layer-color);
    color: #FFFFFF;
    border: none;
    border-radius: 20px;
    padding: 12px 24px;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s ease;
    min-width: 64px;
}

.btn-primary:hover:not(:disabled) {
    background: var(--current-layer-color);
    filter: brightness(1.1);
    box-shadow: 0 2px 4px rgba(0,0,0,0.15);
}

/* Update text field focus color */
.text-field:focus {
    border-color: var(--current-layer-color);
    box-shadow: 0 0 0 1px var(--current-layer-color);
}

/* Update toggle slider color */
input[type="checkbox"]:checked + .toggle-slider {
    background: var(--current-layer-color);
}

.sidebar-header {
    padding: 24px 24px 0;
    border-bottom: 1px solid #49454F;
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.layer-indicator {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--current-layer-color);
    transition: background-color 0.3s ease;
}

.serial-monitor {
    background: #0a0a0a;
    border: 2px solid #333;
    border-radius: 8px;
    padding: 16px;
    font-family: 'Courier New', monospace;
    font-size: 12px;
    color: #00ff00;
    height: 400px;
    overflow-y: auto;
    position: relative;
}

.serial-monitor::before {
    content: '';
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: repeating-linear-gradient(
        0deg,
        transparent,
        transparent 1px,
        rgba(0, 255, 0, 0.03) 1px,
        rgba(0, 255, 0, 0.03) 2px
    );
    pointer-events: none;
    border-radius: 6px;
}

#serialOutput {
    margin: 0;
    padding: 0;
    white-space: pre-wrap;
    word-wrap: break-word;
    position: relative;
    z-index: 1;
}

.section-subtitle {
            font-size: 14px;
            font-weight: 500;
            color: #CAC4D0;
            margin: 0 0 16px 0;
            text-transform: uppercase;
            letter-spacing: 0.5px;
        }

        .theme-divider {
            height: 1px;
            background: #49454F;
            margin: 24px 0;
        }

        .color-selector {
            display: flex;
            align-items: center;
            gap: 12px;
        }

        .color-input {
            width: 40px;
            height: 40px;
            border: 2px solid #49454F;
            border-radius: 8px;
            cursor: pointer;
            background: none;
            padding: 0;
            transition: all 0.2s ease;
        }

        .color-input:hover {
            border-color: %THEME_COLOR%;
            transform: scale(1.1);
        }

        .color-input::-webkit-color-swatch-wrapper {
            padding: 0;
            border-radius: 6px;
            overflow: hidden;
        }

        .color-input::-webkit-color-swatch {
            border: none;
            border-radius: 6px;
        }

        .color-label {
            font-size: 14px;
            color: #E6E1E5;
            font-weight: 500;
        }

        .theme-actions {
            display: flex;
            gap: 12px;
            margin-top: 24px;
            flex-wrap: wrap;
        }

        .theme-actions .btn-secondary,
        .theme-actions .btn-primary {
            flex: 1;
            min-width: 120px;
        }

.encoder-container {
    margin-top: 12px;
    padding: 16px;
    background: #332D41;
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.2s ease;
}

.encoder-container:hover {
    background: #3D3349;
    border: 1px solid %THEME_COLOR%;
}

.encoder-visual {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
}

.encoder-knob {
    width: 40px;
    height: 40px;
    border-radius: 50%;
    background: linear-gradient(145deg, #49454F, #2B2930);
    border: 2px solid #79747E;
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
}

.encoder-indicator {
    width: 3px;
    height: 15px;
    background: %THEME_COLOR%;
    border-radius: 2px;
    position: absolute;
    top: 3px;
}

.encoder-button {
    width: 20px;
    height: 20px;
    border-radius: 4px;
    background: #49454F;
    border: 1px solid #79747E;
    transition: all 0.15s ease;
}

.encoder-button.pressed {
    background: %THEME_COLOR%;
    border-color: %THEME_COLOR%;
    transform: scale(0.9);
}

.encoder-label {
    font-size: 12px;
    color: #E6E1E5;
    text-align: center;
    line-height: 1.3;
}

.leds {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    margin-top: 8px;
}

.led-icon {
    width: 12px;
    height: 12px;
    font-size: 16px;
    color: #49454F;
    cursor: pointer;
    transition: all 0.2s ease;
    text-align: center;
    line-height: 12px;
}

.led-icon:hover {
    color: #CAC4D0;
    transform: scale(1.2);
}

.led-icon.led-on {
    color: %THEME_COLOR%;
    text-shadow: 0 0 8px %THEME_COLOR%;
}

.led-icon.selected {
    outline: 2px solid %THEME_COLOR%;
    outline-offset: 2px;
    border-radius: 50%;
}




    
/* ── Sticky save footer ──────────── */
.sidebar-save-footer{position:sticky;bottom:0;background:#211F26;padding:10px 20px;border-top:1px solid #313244;display:flex;align-items:center;gap:12px;flex-shrink:0;z-index:10;}
.save-feedback{color:#A5D6A7;font-size:12px;opacity:0;transition:opacity 0.3s ease;}
.save-feedback.visible{opacity:1;}
.unsaved-dot{width:7px;height:7px;background:%THEME_COLOR%;border-radius:50%;display:none;flex-shrink:0;box-shadow:0 0 4px %THEME_COLOR%;}
/* ── Config type pills ─────────────── */
.config-type-pills{display:flex;border-radius:20px;overflow:hidden;border:1px solid #49454F;margin-top:12px;}
.pill-btn{flex:1;border:none;background:transparent;color:#79747E;padding:7px 0;font-size:13px;cursor:pointer;transition:all 0.15s;}
.pill-btn.active{background:%THEME_COLOR%;color:#fff;font-weight:500;}
.pill-btn:hover:not(.active){background:#2B2930;}
/* ── Compact channel ───────────────── */
.midi-row{display:flex;gap:6px;align-items:center;}
.midi-row>.text-field{flex:1;min-width:0;}
.ch-label{font-size:11px;color:#79747E;white-space:nowrap;flex-shrink:0;}
.ch-select{width:86px!important;flex-shrink:0;font-size:12px!important;padding:8px 6px!important;}
/* ── LED ring on pad ────────────────── */
.pad,.pad-vertical{position:relative;transition:box-shadow 0.15s ease,background 0.15s ease;}
/* ── Layer count buttons ────────────── */
.layer-count-row{display:flex;gap:6px;}
.lc-btn{flex:1;border:1px solid #49454F;background:transparent;color:#79747E;border-radius:8px;padding:6px 0;font-size:13px;cursor:pointer;transition:all 0.15s;}
.lc-btn.active{background:%THEME_COLOR%;border-color:%THEME_COLOR%;color:#fff;font-weight:500;}
.lc-btn:hover:not(.active){background:#2B2930;}
/* ── Sidebar empty state ─────────────── */
.sidebar-empty{text-align:center;color:#CAC4D0;padding:40px 12px;font-size:14px;line-height:1.5;}
.sidebar-empty .se-icon{font-size:40px;color:#49454F;margin-bottom:8px;}
.sidebar-empty .dim{color:#79747E;font-size:12px;}
/* ── Segmented controls ───────────────── */
.seg{display:flex;border:1px solid #49454F;border-radius:10px;overflow:hidden;}
.seg button{flex:1;border:none;background:transparent;color:#CAC4D0;padding:9px 4px;font-size:12px;cursor:pointer;transition:all 0.15s;border-right:1px solid #49454F;}
.seg button:last-child{border-right:none;}
.seg button:hover:not(.on){background:#2B2930;}
.seg button.on{background:var(--current-layer-color,%THEME_COLOR%);color:#fff;font-weight:600;}
.lc-note{font-size:12px;color:#79747E;margin:0 0 14px 0;line-height:1.4;}
.lc-layers{display:flex;gap:8px;}
.lc-layers button{flex:1;height:44px;border-radius:10px;border:2px solid var(--c);background:transparent;color:var(--c);font-size:18px;font-weight:700;cursor:pointer;transition:all 0.15s;}
.lc-layers button.on{background:var(--c);color:#fff;box-shadow:0 0 12px var(--c);}
.lc-modes{display:flex;flex-direction:column;gap:6px;}
.lc-modes button{display:flex;flex-direction:column;align-items:flex-start;gap:2px;text-align:left;padding:9px 12px;border-radius:10px;border:1px solid #49454F;background:transparent;color:#E6E1E5;cursor:pointer;transition:all 0.15s;}
.lc-modes button span{font-size:11px;color:#79747E;}
.lc-modes button:hover:not(.on){background:#2B2930;}
.lc-modes button.on{border-color:%THEME_COLOR%;background:#2B2930;box-shadow:inset 3px 0 0 %THEME_COLOR%;}
.lc-summary{margin-top:14px;padding:10px 12px;border-radius:8px;background:#1C1B1F;font-size:13px;color:#E6E1E5;line-height:1.4;}
/* ── OSC tab ──────────────────────────── */
.osc-intro{font-size:13px;color:#CAC4D0;margin:-8px 0 20px 0;line-height:1.5;max-width:760px;}
.osc-grid{display:grid;grid-template-columns:minmax(260px,1fr) minmax(260px,1fr);gap:24px;}
@media (max-width:900px){.osc-grid{grid-template-columns:1fr;}}
.osc-col{display:flex;flex-direction:column;gap:16px;min-width:0;}
.osc-row2{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:12px;}
.osc-row2 > .input-group{min-width:0;}
.osc-row2 .text-field{width:100%;min-width:0;box-sizing:border-box;}
.osc-checks{gap:12px;}
.osc-pill{font-size:12px;padding:4px 10px;border-radius:12px;background:#49454F;color:#CAC4D0;}
.osc-pill.on{background:#1B5E20;color:#A5D6A7;}
.osc-msg{font-size:13px;min-height:16px;}
.osc-conn{font-size:13px;color:#CAC4D0;line-height:1.6;}
.osc-conn b{color:#E6E1E5;font-family:monospace;}
.osc-mon-head{display:flex;justify-content:space-between;align-items:center;}
.osc-counts{font-size:12px;color:#79747E;font-family:monospace;}
.osc-log{background:#0a0a0a;border:1px solid #333;border-radius:8px;padding:12px;margin:0;height:320px;overflow-y:auto;font-family:'Courier New',monospace;font-size:12px;color:#00ff00;white-space:pre-wrap;word-break:break-all;}
.osc-hint{font-size:11px;color:#79747E;margin:0;}
.osc-dir{font-size:10px;font-weight:700;padding:2px 6px;border-radius:4px;}
.osc-dir.out{background:#0D47A1;color:#BBDEFB;}
.osc-dir.in{background:#4A148C;color:#E1BEE7;}
#oscRefTable code{font-family:monospace;color:#E6E1E5;}
.osc-warn{font-size:12px;line-height:1.4;color:#FFCC80;background:#3E2723;border-radius:8px;padding:8px 10px;margin-bottom:14px;}
.osc-warn a{color:#FFE0B2;text-decoration:underline;cursor:pointer;}
.config-type-pills .pill-btn{font-size:12px;}
/* ── Live layer bar ───────────────────── */
.layer-bar{display:flex;align-items:center;gap:12px;flex-wrap:wrap;background:#2B2930;border-radius:12px;padding:10px 14px;margin:-4px 0 18px 0;}
.lb-label{font-size:12px;color:#CAC4D0;text-transform:uppercase;letter-spacing:0.5px;}
.lb-chips{display:flex;gap:6px;}
.lb-chips button{width:38px;height:32px;border-radius:8px;border:2px solid var(--c);background:transparent;color:var(--c);font-weight:700;font-size:14px;cursor:pointer;transition:all 0.15s;}
.lb-chips button.on{background:var(--c);color:#fff;box-shadow:0 0 10px var(--c);}
.lb-mode{font-size:12px;color:#79747E;}
/* ── Card layer chip + layer-button badges ── */
.dev-layer-chip{font-size:11px;font-weight:700;color:#fff;padding:2px 7px;border-radius:10px;margin-left:6px;flex-shrink:0;}
.pad,.pad-vertical{display:flex;align-items:center;justify-content:center;}
.pad-tag{font-size:10px;font-weight:700;color:#fff;background:rgba(0,0,0,0.45);padding:1px 4px;border-radius:4px;pointer-events:none;}
</style>
</head>
<body>
    <div id='leftSidebar'>
        <div id='controllerIcons'>
            <div class='icon active' id='devicesTab' onclick='showTab("devices")' title='Devices'>D</div>
            <div class='icon' id='midiTab' onclick='showTab("midi")' title='MIDI Channels'>M</div>
            <div class='icon' id='overviewTab' onclick='showTab("overview")' title='Mapping Overview'>O</div>
            <div class='icon' id='themeTab' onclick='showTab("theme")' title='Theme Settings'>T</div>
            <div class='icon' id='oscTab' onclick='showTab("osc")' title='OSC Control' style='font-size:11px;letter-spacing:0.5px;'>OSC</div>
            <div class='icon' id='preferencesTab' onclick='showTab("preferences")' title='Preferences'>P</div>
        </div>
        <div id='settingsIcons'>
            <div class='icon' id='wifiTab' onclick='showTab("wifi")' title='Wi-Fi Settings'>W</div>
            <div class='icon' id='aboutTab' onclick='showTab("about")' title='About'>?</div>
        </div>
    </div>

    <div class='status-card'>
        <div class='status-indicator'>
            <div class='status-dot'></div>
            <span class='status-text' id='wifi_status'>%STATUS% | IP: %IP_ADDRESS%</span>
        </div>
    </div>

    <div class='section-card tab-content active' id='devicesContent'>
        <div class='section-header'>
            <h4 class='section-title'>Connected Devices</h4>
            <div style='display:flex;align-items:center;gap:16px;'>
                <label class='input-label toggle-label' style='margin:0;'>
                    <input type='checkbox' id='selectByPressToggle' onchange='toggleSelectByPress()'>
                    <span class='toggle-slider'></span>
                    Select by press
                </label>
                <button type='button' class='btn-secondary' onclick='scanI2C()'>Scan I2C Bus</button>
            </div>
        </div>
        <div class='layer-bar' id='layerBar'>
            <span class='lb-label'>Active layer</span>
            <div class='lb-chips' id='lbChips'>
                <button type='button' data-v='0' style='--c:var(--layer1-color)' onclick='setGlobalLayer(0)'>1</button>
                <button type='button' data-v='1' style='--c:var(--layer2-color)' onclick='setGlobalLayer(1)'>2</button>
                <button type='button' data-v='2' style='--c:var(--layer3-color)' onclick='setGlobalLayer(2)'>3</button>
                <button type='button' data-v='3' style='--c:var(--layer4-color)' onclick='setGlobalLayer(3)'>4</button>
            </div>
            <span class='lb-mode' id='lbMode'>&mdash;</span>
        </div>
        <div class='device-grid'>
            <div id="deviceCardsContainer" style="display:flex; flex-wrap:wrap; gap:10px;"></div>
        </div>

<div class='status-card'>
    <div class='lcd-mirror'>
        <div class='lcd-header'>Hardware Display</div>
        <div class='lcd-screen' id='lcdMirror'>
            <div class='lcd-line' id='lcdLine1'>Initializing...</div>
            <div class='lcd-line' id='lcdLine2'></div>
            <div class='lcd-line' id='lcdLine3'></div>
            <div class='lcd-line' id='lcdLine4'></div>
        </div>
    </div>


<div id='serialMonitor' class='serial-monitor'>
        <pre id='serialOutput'></pre>
    </div>


</div>




    </div>

    <div class='section-card tab-content' id='wifiContent'>
        <h4 class='section-title'>Wi-Fi Configuration</h4>
        <div id='wifiSaveStatus' style='display:none;padding:8px 12px;border-radius:4px;margin-bottom:12px;font-size:14px;'></div>
        <div class='wifi-form'>
            <div class='input-group'>
                <label class='input-label'>Primary Network</label>
                <select id='ssid_primary' class='text-field'></select>
            </div>
            <div class='input-group'>
                <label class='input-label'>Password</label>
                <input type='password' id='pass_primary' value='%SAVED_PRIMARY_PASS%' class='text-field'>
            </div>
            <button type='button' class='btn-secondary' onclick='scanWiFi()'>Scan Networks</button>
            <div class='input-group'>
                <label class='input-label'>Fallback SSID</label>
                <input type='text' id='ssid_fallback' value='%WIFI_FALLBACK_SSID%' class='text-field' placeholder='Optional fallback'>
            </div>
            <div class='input-group'>
                <label class='input-label'>Fallback Password</label>
                <input type='password' id='pass_fallback' value='%WIFI_FALLBACK_PASS%' class='text-field'>
            </div>
            <div class='input-group'>
                <label class='input-label'>Hostname</label>
                <input type='text' id='wifi_hostname' value='%WIFI_HOSTNAME%' class='text-field' placeholder='ESP32_MIDI'>
            </div>
            <button type='button' class='btn-primary' onclick='saveWifi()'>Save Configuration</button>
        </div>
    </div>

    <div class='section-card tab-content' id='midiContent'>
        <h4 class='section-title'>MIDI Channel Configuration</h4>
        <div class='midi-channels'>
            %MIDI_CHANNELS%
        </div>
    </div>

<div class='section-card tab-content' id='themeContent'>
        <h4 class='section-title'>Theme Customization</h4>
        <div class='form-section'>
            <div class='input-group'>
                <label class='input-label'>Main Theme Color</label>
                <select id='mainThemeSelect' class='text-field' onchange='changeMainTheme()'>
                    <option value='#4CAF50'%GREEN_SELECTED%>Green</option>
                    <option value='#2196F3'%BLUE_SELECTED%>Blue</option>
                    <option value='#FF5722'%ORANGE_SELECTED%>Orange</option>
                    <option value='#F44336'%RED_SELECTED%>Red</option>
                    <option value='#9C27B0'%PURPLE_SELECTED%>Purple</option>
                    <option value='#607D8B'%BLUE_GREY_SELECTED%>Blue Grey</option>
                    <option value='#f5b227'%AMBER_SELECTED%>Amber</option>
                </select>
            </div>
            
            <div class='theme-divider'></div>
            
            <h5 class='section-subtitle'>Layer Colors</h5>
            
            <div class='input-group'>
                <label class='input-label'>Layer 1 Color</label>
                <div class='color-selector'>
                    <input type='color' id='layer1Color' class='color-input' value='#F44336' onchange='changeLayerColor(0, this.value)'>
                    <span class='color-label'>Red</span>
                </div>
            </div>
            
            <div class='input-group'>
                <label class='input-label'>Layer 2 Color</label>
                <div class='color-selector'>
                    <input type='color' id='layer2Color' class='color-input' value='#9C27B0' onchange='changeLayerColor(1, this.value)'>
                    <span class='color-label'>Purple</span>
                </div>
            </div>
            
            <div class='input-group'>
                <label class='input-label'>Layer 3 Color</label>
                <div class='color-selector'>
                    <input type='color' id='layer3Color' class='color-input' value='#2196F3' onchange='changeLayerColor(2, this.value)'>
                    <span class='color-label'>Blue</span>
                </div>
            </div>
            
            <div class='input-group'>
                <label class='input-label'>Layer 4 Color</label>
                <div class='color-selector'>
                    <input type='color' id='layer4Color' class='color-input' value='#4CAF50' onchange='changeLayerColor(3, this.value)'>
                    <span class='color-label'>Green</span>
                </div>
            </div>
            
            <div class='theme-actions'>
                <button class='btn-secondary' onclick='resetThemeDefaults()'>Reset to Defaults</button>
                <button class='btn-primary' onclick='saveTheme()'>Save Theme</button>
            </div>
        </div>
    </div>

    <div class='section-card tab-content' id='oscContent'>
        <div class='section-header'>
            <h4 class='section-title'>OSC Control</h4>
            <span class='osc-pill' id='oscStatus'>Off</span>
        </div>
        <p class='osc-intro'>Network settings for OSC. To make a button send OSC, select it on the Devices tab and choose <b>Send OSC</b>, just like Send MIDI or Switch Layers. Pedals and encoders have an OSC option too, and LEDs can follow incoming OSC. <b>Set a Send-to IP</b> so the controller knows where to send.</p>
        <div class='osc-grid'>
            <div class='osc-col'>
                <div class='input-group'>
                    <label class='input-label toggle-label'>
                        <input type='checkbox' id='oscEnabled'>
                        <span class='toggle-slider'></span>
                        Enable OSC
                    </label>
                </div>
                <div class='osc-row2'>
                    <div class='input-group'>
                        <label class='input-label' for='oscInPort'>Listen port</label>
                        <input type='number' id='oscInPort' class='text-field' min='1' max='65535' value='8000'>
                    </div>
                    <div class='input-group'>
                        <label class='input-label' for='oscOutPort'>Send port</label>
                        <input type='number' id='oscOutPort' class='text-field' min='1' max='65535' value='9000'>
                    </div>
                </div>
                <div class='input-group'>
                    <label class='input-label' for='oscTargetIP'>Send to IP</label>
                    <input type='text' id='oscTargetIP' class='text-field' placeholder='Blank = reply to whoever last sent OSC'>
                </div>
                <div class='input-group'>
                    <label class='input-label' for='oscPrefix'>Address prefix</label>
                    <input type='text' id='oscPrefix' class='text-field' value='/fc' placeholder='/fc' oninput='renderOscRef()'>
                </div>
                <div class='input-group osc-checks'>
                    <label class='input-label toggle-label'><input type='checkbox' id='oscSendControls'><span class='toggle-slider'></span>Also broadcast every press / pedal / layer change (for remote apps)</label>
                    <label class='input-label toggle-label'><input type='checkbox' id='oscMirrorOut'><span class='toggle-slider'></span>Mirror MIDI output as OSC</label>
                    <label class='input-label toggle-label'><input type='checkbox' id='oscMirrorIn'><span class='toggle-slider'></span>Mirror MIDI input as OSC</label>
                </div>
                <div style='display:flex;gap:10px;flex-wrap:wrap;'>
                    <button type='button' class='btn-primary' onclick='saveOsc()'>Save</button>
                    <button type='button' class='btn-secondary' onclick='oscTest()'>Send test + sync</button>
                </div>
                <div class='osc-msg' id='oscMsg'></div>
                <div class='osc-conn' id='oscConn'></div>
            </div>
            <div class='osc-col'>
                <div class='osc-mon-head'>
                    <span class='input-label'>Live monitor</span>
                    <span class='osc-counts' id='oscCounts'></span>
                </div>
                <pre class='osc-log' id='oscLog'>Waiting for messages...</pre>
                <p class='osc-hint'>&lt; received &nbsp; &gt; sent &nbsp; ? not understood</p>
            </div>
        </div>
        <h5 class='section-subtitle' style='margin-top:24px;'>Remote-control addresses (optional)</h5>
        <p class='osc-hint' style='margin-top:-8px;'>Slot, button and layer numbers are 1-based. Values can be int or float (TouchOSC-style 0.0/1.0 works).</p>
        <div style='overflow-x:auto;'><table class='map-table' id='oscRefTable'></table></div>
    </div>

    <div class='section-card tab-content' id='preferencesContent'>
        <h4 class='section-title'>System Preferences</h4>
        
            
            <!-- Advanced Options Toggle -->
            <div class='input-group'>
                <label class='input-label toggle-label'>
                    <input type='checkbox' id='advancedToggle' onchange='toggleAdvancedOptions()' %ADVANCED_CHECKED%>
                    <span class='toggle-slider'></span>
                    Advanced Options
                </label>
            </div>
            
            <button class='btn-secondary' onclick='sendMidiPanic()'>Send All Notes Off</button>

            <div class='input-group' style='margin-top:16px;'>
                <label class='input-label'>LED Brightness <span id='ledBrightLabel' style='color:%THEME_COLOR%;margin-left:6px;'>100%</span></label>
                <input type='range' id='ledBrightness' min='5' max='255' value='255' style='width:100%;accent-color:%THEME_COLOR%;cursor:pointer;'
                       oninput='onLedBrightnessInput(this.value)' onchange='saveLedBrightness(this.value)'>
            </div>
            
            <!-- Advanced Options Section (Initially Hidden) -->
            <div id='advancedOptions' class='advanced-section' style='display: %ADVANCED_DISPLAY%;'>
                <h5 class='advanced-title'>Advanced Settings</h5>
                
                <div class='input-group'>
                    <label class='input-label'>MIDI Polling Rate</label>
                    <select class='text-field' onchange='saveMidiPollingRate(this.value)'>
                        <option value='10'>10ms (Default)</option>
                        <option value='5'>5ms (High Performance)</option>
                        <option value='20'>20ms (Power Saving)</option>
                    </select>
                </div>
                
                <div class='input-group'>
                    <label class='input-label'>Debug Mode</label>
                    <label class='input-label toggle-label'>
                        <input type='checkbox' id='debugMode' onchange='toggleDebugMode()'>
                        <span class='toggle-slider'></span>
                        Enable Serial Debug Output
                    </label>
                </div>
<label class='input-label toggle-label'>
                <input type='checkbox' id='webSerialToggle' onchange='toggleWebSerial()'>
                <span class='toggle-slider'></span>
                Enable Web Serial
            </label>


                <div class='input-group'>
                    <label class='input-label'>MIDI Output</label>
                    <select id='midiOutputSelect' class='text-field' onchange='saveMidiOutput()'>
                        <option value='0'>UART Only (DIN)</option>
                        <option value='1'>USB Only</option>
                        <option value='2'>Both UART + USB</option>
                    </select>
                </div>

                <div class='input-group'>
                    <label class='input-label'>MIDI 2.0 — Network UMP (UDP)</label>
                    <label class='input-label toggle-label'>
                        <input type='checkbox' id='midi2Toggle' onchange='saveMidi2()'>
                        <span class='toggle-slider'></span>
                        Enable MIDI 2.0 Network (UDP 5004/5005, mDNS <em>_midi2._udp</em>)
                    </label>
                    <p style='font-size:11px;color:#79747E;margin-top:4px;line-height:1.4;'>
                        Streams Universal MIDI Packets (UMP) over UDP with 32-bit resolution.<br>
                        Works with hosts that implement the MIDI 2.0 network transport.<br>
                        Device advertises as <strong>i2c-midi2.local:5004</strong>
                    </p>
                </div>
                
                <div class='input-group'>
                    <label class='input-label'>Number of Layers <span id='layerCountLabel' style='color:%THEME_COLOR%;margin-left:6px;font-size:11px;'>(4 active)</span></label>
                    <div class='layer-count-row'>
                        <button class='lc-btn' id='lc1' onclick='setLayerCount(1)'>1</button>
                        <button class='lc-btn' id='lc2' onclick='setLayerCount(2)'>2</button>
                        <button class='lc-btn' id='lc3' onclick='setLayerCount(3)'>3</button>
                        <button class='lc-btn active' id='lc4' onclick='setLayerCount(4)'>4</button>
                    </div>
                </div>
                <button class='btn-secondary' onclick='exportSettings()'>Export Configuration</button>
                <button class='btn-secondary' onclick='importSettings()'>Import Configuration</button>
            </div>
        </div>
    </div>

    <div class='section-card tab-content' id='overviewContent'>
        <div class='section-header'>
            <h4 class='section-title'>Mapping Overview</h4>
            <button type='button' class='btn-secondary' onclick='loadMappingOverview()'>Refresh</button>
        </div>
        <p style='font-size:12px;color:#79747E;margin-top:-8px;margin-bottom:16px;'>Every configured assignment across all slots and layers. Red entries send the same MIDI message as another control. &#9095; = latching.</p>
        <div id='overviewTableWrap' style='overflow-x:auto;'></div>
    </div>

    <div class='section-card tab-content' id='aboutContent'>
        <h4 class='section-title'>About ESP32 MIDI Controller</h4>
        <div class='info-card'>
            <p class='device-info'>ESP32 I2C to MIDI Bridge</p>
            <p class='button-info'>Firmware Version: 1.3.1 (Fixed)</p>
            <p class='button-info'>Build Date: %BUILD_DATE%</p>
            <button class='btn-primary' onclick='factoryReset()' style='background:#F44336;'>Factory Reset</button>
        </div>
    </div>

    <div id='sidebar'>
        <div class='sidebar-header' style='display:flex;align-items:center;justify-content:space-between;padding-right:8px;'>
    <h3 class='sidebar-title'>Device Settings</h3>
    <div style='display:flex;align-items:center;gap:8px;'><div class='unsaved-dot' id='unsavedDot'></div><div class='layer-indicator' id='layerIndicator'></div></div>
</div>
        <div class='sidebar-content'>
    <div class='sidebar-empty' id='sidebarEmpty'>
        <div class='se-icon'>&#9673;</div>
        <p>Click a button, pedal or encoder on a device card to configure it.</p>
        <p class='dim'>Tip: turn on <b>Select by press</b> and just stomp the switch.</p>
    </div>
    <div class='info-card' id='selInfo' style='display:none;'>
        <p class='device-info' id='deviceIDLabel'>Select a button or pedal to configure</p>
        <p class='button-info'>Element: <span id='buttonLabel'>-</span></p>
    </div>

    <!-- Layer Tabs -->
    <div class='layer-tabs' id='layerTabs'>
        <div class='layer-tab active' onclick='selectLayer(0)'>Layer 1</div>
        <div class='layer-tab' onclick='selectLayer(1)'>Layer 2</div>
        <div class='layer-tab' onclick='selectLayer(2)'>Layer 3</div>
        <div class='layer-tab' onclick='selectLayer(3)'>Layer 4</div>
    </div>

    <input type='hidden' id='configType' value='midi'>
    <div class='config-type-pills' style='margin-top:12px;'>
        <button class='pill-btn active' id='pillMidi'   onclick='onPillClick("midi")'>Send MIDI</button>
        <button class='pill-btn'        id='pillOsc'    onclick='onPillClick("osc")'>Send OSC</button>
        <button class='pill-btn'        id='pillDevice' onclick='onPillClick("device")'>Switch Layers</button>
    </div>

    <!-- MIDI Configuration Section -->
    <div id='midiConfigSection' class='config-section'>
        <div class='input-group'>
            <label class='input-label' for='midiType'>MIDI Type</label>
            <select id='midiType' class='text-field' onchange='onMidiSettingsChange()'>
                <option value='note'>Note</option>
                <option value='cc'>Control Change</option>
            </select>
        </div>
        
        <div class='input-group'>
            <label class='input-label' for='midiNumber'>Number</label>
            <div class='midi-row'>
                <input type='number' id='midiNumber' class='text-field' min='0' max='127' placeholder='0-127' onchange='onMidiSettingsChange()' onwheel='handleMidiWheel(event,this)'>
                <span class='ch-label'>Ch</span>
                <select id='midiChannel' class='ch-select' onchange='onMidiSettingsChange()'>%MIDI_CHANNEL_OPTIONS%</select>
            </div>
        </div>
        
        <div class='input-group'>
            <label class='input-label' for='midiBehavior'>Button Behavior</label>
            <select id='midiBehavior' class='text-field' onchange='onMidiSettingsChange()'>
                <option value='momentary'>Momentary (Note On/Off)</option>
                <option value='latching'>Latching (Toggle)</option>
            </select>
        </div>
    </div>

    <!-- OSC action section -->
    <div id='oscActionSection' class='config-section' style='display:none;'>
        <div class='osc-warn' id='oscWarn' style='display:none;'></div>
        <div class='input-group'>
            <label class='input-label' for='btnOscAddr'>OSC Address</label>
            <input type='text' id='btnOscAddr' class='text-field' placeholder='/looper/record' onchange='onOscSettingsChange()'>
        </div>
        <div class='input-group'>
            <label class='input-label' for='btnOscArg'>Value Type</label>
            <select id='btnOscArg' class='text-field' onchange='onOscArgChange()'>
                <option value='f'>Float</option>
                <option value='i'>Integer</option>
                <option value='none'>No value (address only)</option>
            </select>
        </div>
        <div class='osc-row2' id='btnOscVals'>
            <div class='input-group'>
                <label class='input-label' for='btnOscOn' id='btnOscOnLabel'>Press value</label>
                <input type='number' step='any' id='btnOscOn' class='text-field' value='1' onchange='onOscSettingsChange()'>
            </div>
            <div class='input-group' id='btnOscOffGroup'>
                <label class='input-label' for='btnOscOff' id='btnOscOffLabel'>Release value</label>
                <input type='number' step='any' id='btnOscOff' class='text-field' value='0' onchange='onOscSettingsChange()'>
            </div>
        </div>
        <div class='input-group'>
            <label class='input-label' for='btnOscBehavior'>Button Behavior</label>
            <select id='btnOscBehavior' class='text-field' onchange='onOscArgChange()'>
                <option value='momentary'>Momentary (press / release)</option>
                <option value='latching'>Latching (toggle on / off)</option>
                <option value='trigger'>Trigger (press only)</option>
            </select>
        </div>
        <button type='button' class='btn-secondary' style='padding:8px 14px;font-size:12px;' onclick='oscSendNow("btnOscAddr","btnOscArg","btnOscOn")'>Send test message</button>
    </div>

    <!-- Layer Control (Device Control) Section -->
    <div id='deviceConfigSection' class='config-section' style='display:none;'>
        <p class='lc-note'>This button switches layers instead of sending MIDI. It does the same thing on every layer.</p>

        <div class='input-group'>
            <label class='input-label'>Action</label>
            <div class='seg' id='lcActionSeg'>
                <button type='button' data-v='jump' onclick='lcSet("action","jump")'>Go to layer</button>
                <button type='button' data-v='up'   onclick='lcSet("action","up")'>Next &#9650;</button>
                <button type='button' data-v='down' onclick='lcSet("action","down")'>Prev &#9660;</button>
            </div>
        </div>

        <div class='input-group' id='lcTargetGroup'>
            <label class='input-label'>Target layer</label>
            <div class='lc-layers' id='lcLayerBtns'>
                <button type='button' data-v='0' style='--c:var(--layer1-color)' onclick='lcSet("target",0)'>1</button>
                <button type='button' data-v='1' style='--c:var(--layer2-color)' onclick='lcSet("target",1)'>2</button>
                <button type='button' data-v='2' style='--c:var(--layer3-color)' onclick='lcSet("target",2)'>3</button>
                <button type='button' data-v='3' style='--c:var(--layer4-color)' onclick='lcSet("target",3)'>4</button>
            </div>
        </div>

        <div class='input-group' id='lcModeGroup'>
            <label class='input-label'>When pressed</label>
            <div class='lc-modes' id='lcModeList'>
                <button type='button' data-v='jump'      onclick='lcSet("mode","jump")'><b>Stay</b><span>Switch to the layer and stay there</span></button>
                <button type='button' data-v='latching'  onclick='lcSet("mode","latching")'><b>Toggle</b><span>Press again to go back to Layer 1</span></button>
                <button type='button' data-v='momentary' onclick='lcSet("mode","momentary")'><b>Hold</b><span>Only while held, back to Layer 1 on release</span></button>
            </div>
        </div>

        <div class='input-group'>
            <label class='input-label'>Affects</label>
            <div class='seg' id='lcScopeSeg'>
                <button type='button' data-v='global' onclick='lcSet("scope","global")'>All devices</button>
                <button type='button' data-v='device' onclick='lcSet("scope","device")'>This device only</button>
            </div>
        </div>

        <div class='lc-summary' id='lcSummary'></div>

        <!-- Underlying values sent to the firmware (driven by the controls above) -->
        <div style='display:none;'>
            <select id='deviceControlScope'>
                <option value='layer_global'></option><option value='layer_device'></option>
                <option value='layer_up_global'></option><option value='layer_down_global'></option>
                <option value='layer_up_device'></option><option value='layer_down_device'></option>
            </select>
            <select id='targetLayer'>
                <option value='0'>Layer 1</option><option value='1'>Layer 2</option>
                <option value='2'>Layer 3</option><option value='3'>Layer 4</option>
            </select>
            <select id='controlMode'>
                <option value='jump'></option><option value='momentary'></option><option value='latching'></option>
            </select>
        </div>
    </div>

    <div id='encoderConfigSection' class='config-section' style='display:none;'>
    <h5 class='section-subtitle'>Push Button</h5>
    <div class='input-group'>
        <label class='input-label' for='encoderButtonMode'>Button Action</label>
        <select id='encoderButtonMode' class='text-field' onchange='onEncoderButtonModeChange()'>
            <option value='layer_toggle'>Toggle global / per-device layers</option>
            <option value='midi_momentary'>MIDI Note (momentary)</option>
            <option value='midi_latching'>MIDI Note (latching)</option>
            <option value='osc'>OSC (int 1 on press, 0 on release)</option>
            <option value='pequod'>PEQUOD (set in PEQUOD Host)</option>
            <option value='none'>Nothing</option>
        </select>
    </div>
    <div id='encoderButtonOscOptions' style='display:none;'>
        <div class='input-group'>
            <label class='input-label' for='encoderButtonOscAddr'>OSC Address</label>
            <input type='text' id='encoderButtonOscAddr' class='text-field' placeholder='/transport/play' onchange='safeDisableButton("saveBtn", false)'>
        </div>
    </div>
    <div id='encoderButtonMidiOptions' style='display:none;'>
        <div class='input-group'>
            <label class='input-label' for='encoderButtonNote'>Note</label>
            <div class='midi-row'>
                <input type='number' id='encoderButtonNote' class='text-field' min='0' max='127' value='60' onchange='safeDisableButton("saveBtn", false)'>
                <span class='ch-label'>Ch</span>
                <select id='encoderButtonChannel' class='ch-select' onchange='safeDisableButton("saveBtn", false)'>%MIDI_CHANNEL_OPTIONS%</select>
            </div>
        </div>
    </div>

    <!-- Encoder Settings Section (below button settings) -->
    <div class='advanced-divider'></div>
    <h5 class='section-subtitle'>Encoder Settings</h5>
    
    <div class='input-group'>
        <label class='input-label' for='encoderRotationMode'>Rotation Mode</label>
        <select id='encoderRotationMode' class='text-field' onchange='onEncoderRotationModeChange()'>
            <option value='layer_global'>Layer Scroll (Global)</option>
            <option value='layer_device'>Layer Scroll (Device)</option>
            <option value='midi_cc'>MIDI CC</option>
            <option value='midi_note'>MIDI Note (Per Click)</option>
            <option value='osc'>OSC</option>
            <option value='pequod'>PEQUOD (set in PEQUOD Host)</option>
        </select>
    </div>
    
    <div id='encoderMidiOptions' style='display:none;'>
        <div class='input-group'>
            <label class='input-label' for='encoderMidiCC'>MIDI CC Number</label>
            <input type='number' id='encoderMidiCC' class='text-field' min='0' max='127' value='1' onchange='safeDisableButton("saveBtn", false)'>
        </div>
        
        <div class='input-group'>
            <label class='input-label' for='encoderMidiChannel'>MIDI Channel</label>
            <select id='encoderMidiChannel' class='text-field' onchange='safeDisableButton("saveBtn", false)'>
                %MIDI_CHANNEL_OPTIONS%
            </select>
        </div>
        
        <div class='input-group'>
            <label class='input-label' for='encoderMinValue'>Minimum Value</label>
            <input type='number' id='encoderMinValue' class='text-field' min='0' max='127' value='0' onchange='safeDisableButton("saveBtn", false)'>
        </div>
        
        <div class='input-group'>
            <label class='input-label' for='encoderMaxValue'>Maximum Value</label>
            <input type='number' id='encoderMaxValue' class='text-field' min='0' max='127' value='127' onchange='safeDisableButton("saveBtn", false)'>
        </div>
    </div>
    
    <div id='encoderOscOptions' style='display:none;'>
        <div class='input-group'>
            <label class='input-label' for='encoderOscAddr'>OSC Address</label>
            <input type='text' id='encoderOscAddr' class='text-field' placeholder='/mixer/pan' onchange='safeDisableButton("saveBtn", false)'>
        </div>
        <div class='input-group'>
            <label class='input-label' for='encoderOscMode'>Sends</label>
            <select id='encoderOscMode' class='text-field' onchange='safeDisableButton("saveBtn", false)'>
                <option value='relative'>Steps (int +1 / -1 per click)</option>
                <option value='absolute'>Position (float 0.0 - 1.0)</option>
            </select>
        </div>
    </div>

    <div id='encoderNoteOptions' style='display:none;'>
        <div class='input-group'>
            <label class='input-label' for='encoderNoteNumber'>MIDI Note</label>
            <input type='number' id='encoderNoteNumber' class='text-field' min='0' max='127' value='60' onchange='safeDisableButton("saveBtn", false)'>
        </div>
        
        <div class='input-group'>
            <label class='input-label' for='encoderNoteChannel'>MIDI Channel</label>
            <select id='encoderNoteChannel' class='text-field' onchange='safeDisableButton("saveBtn", false)'>
                %MIDI_CHANNEL_OPTIONS%
            </select>
        </div>
    </div>
    
    <div class='input-group'>
        <label class='input-label toggle-label'>
            <input type='checkbox' id='encoderInvertDirection' onchange='safeDisableButton("saveBtn", false)'>
            <span class='toggle-slider'></span>
            Invert Rotation Direction
        </label>
    </div>
    
    <div class='input-group'>
        <label class='input-label' for='encoderStepsPerDetent'>Steps Per Detent</label>
        <select id='encoderStepsPerDetent' class='text-field' onchange='safeDisableButton("saveBtn", false)'>
            <option value='1'>1 (High Resolution)</option>
            <option value='2'>2</option>
            <option value='4' selected>4 (Standard)</option>
        </select>
    </div>
</div>

<div id='ledConfigSection' class='config-section' style='display:none;'>
    <h5 class='section-subtitle'>LED Settings</h5>
    
    <div class='input-group'>
        <label class='input-label' for='ledMode'>LED Mode</label>
        <select id='ledMode' class='text-field' onchange='onLEDModeChange()'>
            <option value='button_state'>Follow Button State</option>
            <option value='midi_out'>Follow MIDI Output</option>
            <option value='midi_out_triggered'>MIDI Output Triggered</option>
            <option value='layer'>Always Show Layer</option>
            <option value='midi_in'>MIDI Input Triggered</option>
            <option value='osc_in'>OSC Input Triggered</option>
            <option value='off'>Off</option>
        </select>
    </div>
    
    <div id='ledColorOptions'>
        <div class='input-group'>
            <label class='input-label toggle-label'>
                <input type='checkbox' id='ledFollowLayer' onchange='onLEDFollowLayerChange()' checked>
                <span class='toggle-slider'></span>
                Use Layer Colors
            </label>
        </div>
        
        <div id='ledCustomColorOption' style='display:none;'>
            <div class='input-group'>
                <label class='input-label'>Custom LED Color</label>
                <div class='color-selector'>
                    <input type='color' id='ledCustomColor' class='color-input' value='#FFFFFF' onchange='safeDisableButton("saveBtn", false)'>
                    <span class='color-label' id='ledColorLabel'>White</span>
                </div>
            </div>
        </div>
    </div>
    
    <div id='ledOscOptions' style='display:none;'>
        <div class='input-group'>
            <label class='input-label' for='ledOscAddr'>Listen for OSC Address</label>
            <input type='text' id='ledOscAddr' class='text-field' placeholder='/looper/recording' onchange='safeDisableButton("saveBtn", false)'>
            <p class='osc-hint' style='margin-top:4px;'>Value &ge; 0.5 (or "on") lights it, below 0.5 turns it off, no value toggles.</p>
        </div>
    </div>
    <div id='ledMidiOptions' style='display:none;'>
        <div class='input-group'>
            <label class='input-label' for='ledMidiNote'>MIDI Note</label>
            <input type='number' id='ledMidiNote' class='text-field' min='0' max='127' value='60' onchange='safeDisableButton("saveBtn", false)'>
        </div>
        
        <div class='input-group'>
            <label class='input-label' for='ledMidiChannel'>MIDI Channel</label>
            <select id='ledMidiChannel' class='text-field' onchange='safeDisableButton("saveBtn", false)'>
                %MIDI_CHANNEL_OPTIONS%
            </select>
        </div>
    </div>
</div>

<!-- Pedal MIDI config — shown instead of midiConfigSection when a pedal is selected -->
<div id='pedalMidiSection' class='config-section' style='display:none;'>
    <input type='hidden' id='pedalOutType' value='cc'>
    <div class='seg' style='margin-bottom:14px;'>
        <button type='button' id='pedalOutCC'  class='on' onclick='setPedalOut("cc", true)'>MIDI CC</button>
        <button type='button' id='pedalOutOsc'            onclick='setPedalOut("osc", true)'>OSC</button>
    </div>
    <div id='pedalOscFields' style='display:none;'>
        <div class='osc-warn' id='pedalOscWarn' style='display:none;'></div>
        <div class='input-group'>
            <label class='input-label' for='pedalOscAddr'>OSC Address</label>
            <input type='text' id='pedalOscAddr' class='text-field' placeholder='/mixer/volume' onchange='safeDisableButton("saveBtn", false)'>
        </div>
        <div class='osc-row2'>
            <div class='input-group'>
                <label class='input-label' for='pedalOscMin'>Heel (min)</label>
                <input type='number' step='any' id='pedalOscMin' class='text-field' value='0' onchange='safeDisableButton("saveBtn", false)'>
            </div>
            <div class='input-group'>
                <label class='input-label' for='pedalOscMax'>Toe (max)</label>
                <input type='number' step='any' id='pedalOscMax' class='text-field' value='1' onchange='safeDisableButton("saveBtn", false)'>
            </div>
        </div>
        <div class='input-group'>
            <label class='input-label' for='pedalOscArg'>Value Type</label>
            <select id='pedalOscArg' class='text-field' onchange='safeDisableButton("saveBtn", false)'>
                <option value='f'>Float</option>
                <option value='i'>Integer</option>
            </select>
        </div>
    </div>
    <div class='input-group' id='pedalCCFields'>
        <label class='input-label'>MIDI CC</label>
        <div class='midi-row'>
            <input type='number' id='pedalCCNumber' class='text-field' min='0' max='127'
                   placeholder='0–127' onchange='safeDisableButton("saveBtn", false)'
                   onwheel='handleMidiWheel(event,this)'>
            <span class='ch-label'>Ch</span>
            <select id='pedalCCChannel' class='ch-select' onchange='safeDisableButton("saveBtn", false)'>
                %MIDI_CHANNEL_OPTIONS%
            </select>
        </div>
    </div>
</div>





            
                
                

            <div id='calSection' style='display:none;'>
                <div style='height:12px;'></div>
                <div class='info-card' style='margin-bottom:16px;'>
                    <p class='device-info'>Live Raw Values</p>
                    <div style='display:flex;flex-direction:column;gap:8px;font-family:monospace;font-size:12px;'>
                        <div style='display:flex;justify-content:space-between;'>
                            <span>Raw ADC:</span>
                            <span id='rawValue' style='color:%THEME_COLOR%;font-weight:bold;'>----</span>
                        </div>
                        <div style='display:flex;justify-content:space-between;'>
                            <span>Calibrated:</span>
                            <span id='calValue' style='color:%THEME_COLOR%;font-weight:bold;'>----</span>
                        </div>
                        <div style='display:flex;justify-content:space-between;'>
                            <span>MIDI Out:</span>
                            <span id='midiOut' style='color:%THEME_COLOR%;font-weight:bold;'>----</span>
                        </div>
                    </div>
                </div>
                <div class='info-card'>
                    <p class='device-info'>Pedal Calibration</p>
                    <div class='input-group'>
                        <label class='input-label' for='calMin'>Calibration Min (0-4095 raw ADC)</label>
                        <input type='number' id='calMin' class='text-field' min='0' max='4095' value='0'>
                        <button type='button' class='btn-secondary' onclick='setCurrentRawAsMin()' style='margin-top:4px;font-size:12px;padding:8px 12px;'>Set Current as Min</button>
                    </div>
                    <div class='input-group'>
                        <label class='input-label' for='calMax'>Calibration Max (0-4095 raw ADC)</label>
                        <input type='number' id='calMax' class='text-field' min='0' max='4095' value='4095'>
                        <button type='button' class='btn-secondary' onclick='setCurrentRawAsMax()' style='margin-top:4px;font-size:12px;padding:8px 12px;'>Set Current as Max</button>
                    </div>
                    <div class='input-group'>
                        <label class='input-label' style='display:flex;align-items:center;gap:12px;'>
                            <input type='checkbox' id='calInvert'> Invert
                        </label>
                    </div>
                    <button class='btn-primary' onclick='savePedalCal()' disabled id='saveCalBtn'>Save Calibration</button>
                </div>
            </div>
        </div><!-- /sidebar-content -->
        <div class='sidebar-save-footer'>
            <button class='btn-primary' onclick='saveSettings()' disabled id='saveBtn' style='display:none;'>Save</button>
            <span style='font-size:12px;color:#79747E;'>Changes save automatically</span>
            <span class='save-feedback' id='saveFeedback'>Saved ✓</span>
        </div>
    </div>
</body>
</html>
)rawliteral";


void handleGetNumLayers() {
  prefs.begin("ui", true); int n = prefs.getInt("numLayers", 4); prefs.end();
  configNumLayers = n;  // keep runtime variable in sync
  server.send(200, "text/plain", String(n));
}
void handleSaveNumLayers() {
  int n = server.arg("n").toInt();
  if (n >= 1 && n <= 4) {
    configNumLayers = n;  // update runtime immediately
    prefs.begin("ui", false); prefs.putInt("numLayers", n); prefs.end();
  }
  server.send(200, "text/plain", "OK");
}
void handleGetLayerMidi();
void handleSaveLayerMidi();
void handleSaveShiftConfig();
void handleGetDeviceShiftConfig();

void initStackCanary() {
  for (int i = 0; i < 8; i++) {
    stackCanary[i] = 0xDEADBEEF + i;
  }
}

bool isStackCanaryIntact() {
  for (int i = 0; i < 8; i++) {
    if (stackCanary[i] != (0xDEADBEEF + i)) {
      Serial.println("CRITICAL: Stack overflow detected!");
      return false;
    }
  }
  return true;
}

bool checkMemoryHealth() {
  size_t freeHeap = ESP.getFreeHeap();
  size_t minFreeHeap = ESP.getMinFreeHeap();
  size_t maxAllocHeap = ESP.getMaxAllocHeap();

  memStats.freeHeap = freeHeap;
  memStats.minFreeHeap = minFreeHeap;
  memStats.maxAllocHeap = maxAllocHeap;
  memStats.heapFragmentation = (freeHeap > 0) ? 100 - (maxAllocHeap * 100) / freeHeap : 100;
  memStats.lastCheck = millis();

  if (freeHeap < MIN_FREE_HEAP) {
    Serial.printf("CRITICAL: Low memory! Free: %d bytes\n", freeHeap);
    return false;
  }

  if (memStats.heapFragmentation > 50) {
    Serial.printf("WARNING: High fragmentation: %d%%\n", memStats.heapFragmentation);
  }

  return true;
}

void printMemoryStats() {
  Serial.println("=== Memory Statistics ===");
  Serial.printf("Free Heap: %d bytes\n", memStats.freeHeap);
  Serial.printf("Min Free Heap: %d bytes\n", memStats.minFreeHeap);
  Serial.printf("Max Alloc Heap: %d bytes\n", memStats.maxAllocHeap);
  Serial.printf("Fragmentation: %d%%\n", memStats.heapFragmentation);
  Serial.printf("Stack Canary: %s\n", isStackCanaryIntact() ? "OK" : "CORRUPTED");

  size_t largestFree = heap_caps_get_largest_free_block(MALLOC_CAP_DEFAULT);
  Serial.printf("Largest Free Block: %d bytes\n", largestFree);
}

bool safeStringCopy(char* dest, const char* src, size_t destSize) {
  if (!dest || !src || destSize == 0) return false;
  strncpy(dest, src, destSize - 1);
  dest[destSize - 1] = '\0';
  return true;
}

void initializeEncoderConfig(DeviceConfig& config) {
  config.encoderRotationMode = "layer_global";
  config.encoderMidiCC = 1;
  config.encoderMidiChannel = 1;
  config.encoderNoteNumber = 60;        // NEW
  config.encoderNoteChannel = 1;        // NEW
  config.encoderMinValue = 0;
  config.encoderMaxValue = 127;
  config.encoderInvertDirection = false;
  config.encoderButtonMode = "layer_toggle";
  config.encoderButtonMidiNote = 60;
  config.encoderButtonMidiChannel = 1;
  config.encoderStepsPerDetent = 4;
  config.encoderOscAddr = "/encoder";
  config.encoderOscMode = "relative";
  config.encoderButtonOscAddr = "/encoder/button";
}


// MIDI Functions
void sendMidiNote(uint8_t note, bool on, uint8_t channel = 1, uint8_t velocity = 127) {
  if (midiOutputMode == MIDI_UART_ONLY || midiOutputMode == MIDI_BOTH) {
    uint8_t status = on ? 0x90 : 0x80;
    status |= (channel - 1) & 0x0F;
    Serial1.write(status);
    Serial1.write(note & 0x7F);
    Serial1.write(velocity & 0x7F);
  }
  if (midiOutputMode == MIDI_USB_ONLY || midiOutputMode == MIDI_BOTH) {
    CS::MIDIAddress address {note, CS::Channel(channel)};
    if (on) usbmidi.sendNoteOn(address, velocity);
    else    usbmidi.sendNoteOff(address, velocity);
  }
  // MIDI 2.0 Network
  if (midi2Enabled) {
    if (on) midi2NetNoteOn(channel, note, velocity);
    else    midi2NetNoteOff(channel, note, velocity);
  }
  oscMirrorMidiOut(true, channel, note, on ? velocity : 0);
}

void sendMidiCC(uint8_t ccNumber, uint8_t ccValue, uint8_t channel = 1) {
  if (midiOutputMode == MIDI_UART_ONLY || midiOutputMode == MIDI_BOTH) {
    uint8_t status = 0xB0 | ((channel - 1) & 0x0F);
    Serial1.write(status);
    Serial1.write(ccNumber & 0x7F);
    Serial1.write(ccValue & 0x7F);
  }
  if (midiOutputMode == MIDI_USB_ONLY || midiOutputMode == MIDI_BOTH) {
    CS::MIDIAddress address {ccNumber, CS::Channel(channel)};
    usbmidi.sendControlChange(address, ccValue);
  }
  // MIDI 2.0 Network
  if (midi2Enabled) {
    midi2NetCC(channel, ccNumber, ccValue);
  }
  oscMirrorMidiOut(false, channel, ccNumber, ccValue);
}              


void handleSaveMidiOutput() {
  if (server.hasArg("mode")) {
    int mode = server.arg("mode").toInt();
    if (mode >= 0 && mode <= 2) {
      midiOutputMode = (MidiOutputMode)mode;
      spiffsConfig.saveUIConfig(THEME_COLOR, advancedOptionsEnabled, midiOutputMode);
      Serial.printf("MIDI output mode changed to: %d\n", mode);
    }
  }
  server.send(200, "text/plain", "OK");
}

void handleGetMidiOutput() {
  String json = "{\"mode\":" + String((int)midiOutputMode) + "}";
  server.send(200, "application/json", json);
}

// Add this function to get the current active layer
int getCurrentActiveLayer() {
  if (globalLayerControl.enabled) {
    return globalLayerControl.currentGlobalLayer;
  }

  // Check for any device-specific layer activity
  for (int slot = 0; slot < 8; slot++) {
    if (shiftStates[slot].currentLayer > 0) {
      return shiftStates[slot].currentLayer;
    }
  }

  return 0;  // Default to layer 0
}

// Layer used for a given slot's MIDI output: a per-device layer (when not
// Layer 1) overrides the global layer; otherwise the global layer applies.
int getSlotActiveLayer(int slot) {
  int layer = 0;
  if (slot >= 0 && slot < 8 && shiftStates[slot].currentLayer > 0) {
    layer = shiftStates[slot].currentLayer;
  } else if (globalLayerControl.enabled) {
    layer = globalLayerControl.currentGlobalLayer;
  }
  return constrain(layer, 0, configNumLayers - 1);
}

// Live layer state for the web UI
String layerStateJson() {
  String json = "{\"globalEnabled\":" + String(globalLayerControl.enabled ? "true" : "false");
  json += ",\"globalLayer\":" + String(globalLayerControl.currentGlobalLayer);
  json += ",\"active\":" + String(getCurrentActiveLayer());
  json += ",\"numLayers\":" + String(configNumLayers) + "}";
  return json;
}

// Initialize device config with layers
void initializeDeviceConfigWithLayers(DeviceConfig& config, uint8_t deviceType) {
  // Initialize base layer (layer 0)
  if (deviceType >= TYPE_PEDAL_MIN && deviceType <= TYPE_PEDAL_MAX) {
    config.midiMapping[0] = 7;
    config.midiType[0] = "cc";
    config.midiChannel[0] = 1;
    for (int i = 1; i < 4; i++) {
      config.midiMapping[i] = 0;
      config.midiType[i] = "cc";
      config.midiChannel[i] = 1;
    }
  } else {
    for (int i = 0; i < 4; i++) {
      config.midiMapping[i] = 60 + i;
      config.midiType[i] = "note";
      config.midiChannel[i] = 1;
    }
  }

  // Initialize all 4 layers with defaults
  for (int layer = 0; layer < 4; layer++) {
    for (int btn = 0; btn < 4; btn++) {
      if (deviceType >= TYPE_PEDAL_MIN && deviceType <= TYPE_PEDAL_MAX) {
        config.layerMidiMapping[layer][btn] = (layer == 0 && btn == 0) ? 7 : 0;
        config.layerMidiType[layer][btn] = "cc";
        config.layerMidiChannel[layer][btn] = 1;
        config.layerMidiBehavior[layer][btn] = "momentary";
        config.layerOscAddr[layer][btn] = (btn == 0) ? "/pedal" : "/pedal/button";
      } else {
        config.layerMidiMapping[layer][btn] = 60 + (layer * 16) + btn;
        config.layerMidiType[layer][btn] = "note";
        config.layerMidiChannel[layer][btn] = 1;
        config.layerMidiBehavior[layer][btn] = "momentary";
        config.layerOscAddr[layer][btn] = "/button/" + String(btn + 1);
      }
      config.layerOscOn[layer][btn]  = 1.0f;
      config.layerOscOff[layer][btn] = 0.0f;
      config.layerOscArg[layer][btn] = "f";
    }
  }

  // Initialize device control settings
  for (int btn = 0; btn < 4; btn++) {
    config.deviceControlType[btn] = "none";
    config.deviceControlTarget[btn] = 0;
    config.deviceControlMode[btn] = "momentary";
  }

  // Initialize LED settings
  for (int btn = 0; btn < 4; btn++) {
    config.ledMode[btn] = "button_state";  // Default: follow button
    config.ledColor[btn] = "#FFFFFF";      // Default: white
    config.ledMidiNote[btn] = 60 + btn;
    config.ledMidiChannel[btn] = 1;
    config.ledFollowLayer[btn] = true;     // Default: follow layer colors
    config.ledOscAddr[btn] = "/led/" + String(btn + 1);
  }

  // Initialize shift settings
  config.shiftButton = -1;
  config.shiftMode = true;
  config.shiftEnabled = false;

  // Pedal settings
  config.pedalCalMin = 0;
  config.pedalCalMax = 4095;
  config.pedalInvert = false;
  config.preferredSlot = -1;
  
  // NEW: Initialize encoder settings if this is an encoder device
  if (deviceType >= TYPE_ENCODER_MIN && deviceType <= TYPE_ENCODER_MAX) {
    initializeEncoderConfig(config);
  }
}


String typeToName(uint8_t type) {
  if (type >= TYPE_MASTER_MIN && type <= TYPE_MASTER_MAX) return "Master";
  if (type >= TYPE_FOOTSWITCH_MIN && type <= TYPE_FOOTSWITCH_MAX) {
    if (type == 21) return "Vertical";  // Special case for vertical layout
    return "Footswitch";
  }
  if (type >= TYPE_PEDAL_MIN && type <= TYPE_PEDAL_MAX) return "Pedal";
  if (type >= TYPE_BUTTON_MIN && type <= TYPE_BUTTON_MAX) return "Button";
  if (type >= TYPE_KNOB_MIN && type <= TYPE_KNOB_MAX) return "Knob";
  if (type >= TYPE_ENCODER_MIN && type <= TYPE_ENCODER_MAX) return "Encoder";
  return "Reserved";
}

String getLighterColor(String baseColor) {
  if (baseColor == "#FF5722") return "#FF6F42";
  if (baseColor == "#2196F3") return "#42A5F5";
  if (baseColor == "#4CAF50") return "#66BB6A";
  if (baseColor == "#F44336") return "#EF5350";
  if (baseColor == "#9C27B0") return "#AB47BC";
  if (baseColor == "#607D8B") return "#78909C";
  if (baseColor == "#f5b227") return "#f7c455";  // NEW: Amber lighter variant
  return baseColor + "33";
}

// Update the getLightBackground function:
String getLightBackground(String baseColor) {
  if (baseColor == "#FF5722") return "#FFE0DB";
  if (baseColor == "#2196F3") return "#E3F2FD";
  if (baseColor == "#4CAF50") return "#E8F5E8";
  if (baseColor == "#F44336") return "#FFEBEE";
  if (baseColor == "#9C27B0") return "#F3E5F5";
  if (baseColor == "#607D8B") return "#ECEFF1";
  if (baseColor == "#f5b227") return "#fef9e7";  // NEW: Amber light background
  return "#E8DEF8";
}

DeviceConfig* getDeviceConfigForSlot(int slot) {
  if (slot < 0 || slot >= 8) return nullptr;
  if (deviceMapping[slot] < 0 || deviceMapping[slot] >= numDetected) return nullptr;
  return &detectedSlaves[deviceMapping[slot]].config;
}

SlaveInfo* getSlaveInfoForSlot(int slot) {
  if (slot < 0 || slot >= 8) return nullptr;
  if (deviceMapping[slot] < 0 || deviceMapping[slot] >= numDetected) return nullptr;
  return &detectedSlaves[deviceMapping[slot]];
}




// Load device config with layer support
// FIXED: Load device config with behavior defaults
void loadDeviceConfigWithLayers(int slaveIndex) {
  if (slaveIndex < 0 || slaveIndex >= numDetected) return;

  SlaveInfo& slave = detectedSlaves[slaveIndex];
  // Start from defaults, then overlay whatever the saved file contains, so
  // fields added in newer firmware (e.g. OSC) are valid for older config files.
  initializeDeviceConfigWithLayers(slave.config, slave.type);
  initializeEncoderConfig(slave.config);
  spiffsConfig.loadDeviceConfig(slaveIndex, slave);
  // Shift state initialized by initializeShiftStates() after autoMapDevices()
}

void cleanupOrphanedDeviceControlKeys(String deviceKey) {
  Serial.printf("[CLEANUP] Checking for orphaned keys for device %s\n", deviceKey.c_str());

  prefs.begin("devices", false);

  // Remove old long-format keys if they exist
  for (int btn = 0; btn < 4; btn++) {
    String oldTypeKey = deviceKey + "_dc_type_" + String(btn);
    String oldTargetKey = deviceKey + "_dc_target_" + String(btn);
    String oldMomKey = deviceKey + "_dc_momentary_" + String(btn);

    if (prefs.isKey(oldTypeKey.c_str())) {
      Serial.printf("[CLEANUP] Removing orphaned key: %s\n", oldTypeKey.c_str());
      prefs.remove(oldTypeKey.c_str());
      prefs.remove(oldTargetKey.c_str());
      prefs.remove(oldMomKey.c_str());
    }
  }

  prefs.end();
}


void dumpPreferences() {
  Serial.println("=== PREFERENCES DUMP ===");
  prefs.begin("devices", true);

  // Get all keys
  Serial.println("Available keys in 'devices' namespace:");

  // Since we can't easily iterate all keys, let's check for our known devices
  for (int d = 0; d < numDetected; d++) {
    String deviceKey = idToShortKey(detectedSlaves[d].uniqueID);
    Serial.printf("Checking device %s:\n", deviceKey.c_str());

    // Check device control keys
    for (int btn = 0; btn < 4; btn++) {
      String dcTypeKey = deviceKey + "_dt" + String(btn);    // 13 chars max
      String dcTargetKey = deviceKey + "_dr" + String(btn);  // 13 chars max
      String dcMomKey = deviceKey + "_dm" + String(btn);     // 13 chars max

      if (prefs.isKey(dcTypeKey.c_str())) {
        String type = prefs.getString(dcTypeKey.c_str(), "none");
        int target = prefs.getInt(dcTargetKey.c_str(), 0);
        bool momentary = prefs.getBool(dcMomKey.c_str(), true);
        Serial.printf("  %s = %s, target=%d, momentary=%s\n",
                      dcTypeKey.c_str(), type.c_str(), target, momentary ? "Y" : "N");
      }
    }

    // Check some layer keys too
    String layerKey = deviceKey + "_L0_0_val";
    if (prefs.isKey(layerKey.c_str())) {
      int val = prefs.getInt(layerKey.c_str(), -1);
      Serial.printf("  %s = %d\n", layerKey.c_str(), val);
    }
  }

  prefs.end();
  Serial.println("=== END PREFERENCES DUMP ===");
}




void testDeviceControlSave() {
  Serial.println("=== TESTING DEVICE CONTROL SAVE ===");

  // Test saving some device control settings
  if (numDetected > 0) {
    SlaveInfo& slave = detectedSlaves[0];
    String deviceKey = idToShortKey(slave.uniqueID);

    // Manually set some device control config
    slave.config.deviceControlType[0] = "layer_device";
    slave.config.deviceControlTarget[0] = 2;
    slave.config.deviceControlMode[0] = "latching";  // CHANGED

    Serial.printf("Test: Setting device %s button 0 to layer_device, target=2, mode=latching\n", deviceKey.c_str());

    // Save it
    saveDeviceConfigWithLayers(0);

    // Clear the config
    slave.config.deviceControlType[0] = "none";
    slave.config.deviceControlTarget[0] = 0;
    slave.config.deviceControlMode[0] = "momentary";  // CHANGED

    Serial.println("Test: Cleared config, now reloading...");

    // Reload it
    loadDeviceConfigWithLayers(0);

    // Check if it loaded correctly
    Serial.printf("Test result: Type=%s, Target=%d, Mode=%s\n",  // CHANGED
                  slave.config.deviceControlType[0].c_str(),
                  slave.config.deviceControlTarget[0],
                  slave.config.deviceControlMode[0].c_str());  // CHANGED
  }

  Serial.println("=== END TEST ===");
}







// Save device config with layer support
void saveDeviceConfigWithLayers(int slaveIndex) {
  if (slaveIndex < 0 || slaveIndex >= numDetected) return;

  SlaveInfo& slave = detectedSlaves[slaveIndex];
  bool success = spiffsConfig.saveDeviceConfig(slaveIndex, slave);

  if (success) {
    Serial.printf("[CONFIG SAVE] Successfully saved device %s to SPIFFS\n",
                  idToShortKey(slave.uniqueID).c_str());
  } else {
    Serial.printf("[CONFIG SAVE] Failed to save device %s to SPIFFS\n",
                  idToShortKey(slave.uniqueID).c_str());
  }
}



void checkNVSStatus() {
  Serial.println("=== NVS DIAGNOSTICS ===");

  // Test basic preferences operations
  Preferences testPrefs;
  if (testPrefs.begin("nvs_test", false)) {
    Serial.println("Test preferences opened successfully");

    size_t written = testPrefs.putString("test_key", "test_value");
    Serial.printf("Test write: %d bytes\n", written);

    String readBack = testPrefs.getString("test_key", "MISSING");
    Serial.printf("Test read: '%s'\n", readBack.c_str());

    testPrefs.remove("test_key");
    testPrefs.end();

    if (written > 0 && readBack == "test_value") {
      Serial.println("Basic NVS operations: WORKING");
    } else {
      Serial.println("Basic NVS operations: FAILED");
    }
  } else {
    Serial.println("ERROR: Failed to open test preferences");
  }

  Serial.println("=== END NVS DIAGNOSTICS ===");
}














// Enhanced button polling with shift support
// Forward declaration for setDeviceLED (defined after the polling functions)
void setDeviceLED(uint8_t i2cAddress, uint8_t ledIndex, CRGB color);
// Forward declarations for MIDI input handlers (defined after the MIDI 2.0 section)
void handleMidiInputMessage(uint8_t type, uint8_t channel, uint8_t data1, uint8_t data2);
void processMidiInput();


// ═══════════════════════════════════════════════════════════════════════════
//  Button processing (shared by physical presses and OSC "virtual" presses)
// ═══════════════════════════════════════════════════════════════════════════
bool ccLatchState[8][4]   = {{false}};
bool noteLatchState[8][4] = {{false}};
bool ledOutputState[8][4] = {{false}};
bool lastLEDState[8][4]   = {{false}};
unsigned long lastLEDUpdate[8][4] = {{0}};
const unsigned long LED_UPDATE_DEBOUNCE = 50;

// LED overrides set over OSC (/slot/S/led/B). While active, the override colour
// wins over the normal LED logic until /led/B/auto (or /led/auto) is received.
bool ledOverrideOn[8][4] = {{false}};
CRGB ledOverrideColor[8][4];

// Button configured to send OSC: momentary (on/off), latching (toggle) or trigger (press only)
void processOscButton(SlaveInfo* slave, int slot, int btn, int ledBtn, bool pressed, int layer) {
  DeviceConfig& config = slave->config;
  String beh  = config.layerMidiBehavior[layer][btn];
  String addr = config.layerOscAddr[layer][btn];
  bool  send = false;
  float val  = 0;
  if (beh == "latching") {
    if (pressed) { noteLatchState[slot][btn] = !noteLatchState[slot][btn]; send = true;
                   val = noteLatchState[slot][btn] ? config.layerOscOn[layer][btn] : config.layerOscOff[layer][btn]; }
  } else if (beh == "trigger") {
    if (pressed) { send = true; val = config.layerOscOn[layer][btn]; }
  } else {
    send = true;
    val = pressed ? config.layerOscOn[layer][btn] : config.layerOscOff[layer][btn];
  }
  if (send) oscSendAction(addr, config.layerOscArg[layer][btn], val);

  String lm = config.ledMode[ledBtn];
  if (lm == "midi_out_triggered") {
    if (pressed && send) { midiOutTriggerTime[slot][ledBtn] = millis(); updateDeviceLED(slot, ledBtn, true); lastLEDState[slot][ledBtn] = true; }
    return;
  }
  bool led = (lm == "midi_out" && beh == "latching") ? noteLatchState[slot][btn] : pressed;
  if (led != lastLEDState[slot][ledBtn]) {
    updateDeviceLED(slot, ledBtn, led);
    lastLEDState[slot][ledBtn] = led;
    lastLEDUpdate[slot][ledBtn] = millis();
  }
}

void processButtonChange(SlaveInfo* slave, int slot, int btn, bool pressed, bool prev) {
  DeviceConfig& config = slave->config;
    if (pressed) anyButtonPressed = true;

    Serial.printf("[BUTTON] Slot %d, Btn %d: %s->%s, DC Type='%s'\n",
                  slot, btn, prev ? "ON" : "OFF", pressed ? "ON" : "OFF",
                  config.deviceControlType[btn].c_str());

    // Skip shift button MIDI output if it's configured as shift
    if (config.shiftEnabled && btn == config.shiftButton) {
      Serial.printf("[BUTTON] Skipping btn %d (shift button)\n", btn);
      ledOutputState[slot][btn] = pressed;
      updateDeviceLED(slot, btn, ledOutputState[slot][btn]);
      return;
    }

    // Check if this button is configured for device control
    if (config.deviceControlType[btn] != "none" && config.deviceControlType[btn] != "") {
      Serial.printf("[BUTTON] Processing as device control\n");
      
      String controlMode = config.deviceControlMode[btn];
      bool newLEDState = pressed;
      
      if (controlMode == "latching") {
        if (config.deviceControlType[btn] == "layer_global") {
          newLEDState = (globalLayerControl.currentGlobalLayer == config.deviceControlTarget[btn]);
        } else if (config.deviceControlType[btn] == "layer_device") {
          newLEDState = (shiftStates[slot].currentLayer == config.deviceControlTarget[btn]);
        }
      }
      
      handleDeviceControl(slot, btn, pressed, config);
      
      if (newLEDState != lastLEDState[slot][btn] && 
          (millis() - lastLEDUpdate[slot][btn] > LED_UPDATE_DEBOUNCE)) {
        updateDeviceLED(slot, btn, newLEDState);
        lastLEDState[slot][btn] = newLEDState;
        lastLEDUpdate[slot][btn] = millis();
      }
      return;
    }

    Serial.printf("[BUTTON] Processing as MIDI\n");

    // Get MIDI settings for current layer
    int activeLayer = getSlotActiveLayer(slot);

    // Owned by PEQUOD: it already got the press over the link and drives the LED.
    if (config.layerMidiType[activeLayer][btn] == "pequod") {
      if (!pqHostConnected()) updateDeviceLED(slot, btn, pressed);   // simple feedback while PEQUOD is closed
      return;
    }

    if (config.layerMidiType[activeLayer][btn] == "osc") {
      processOscButton(slave, slot, btn, btn, pressed, activeLayer);
      return;
    }

    String midiType = config.layerMidiType[activeLayer][btn];
    String midiBehavior = config.layerMidiBehavior[activeLayer][btn];
    int midiValue = config.layerMidiMapping[activeLayer][btn];
    int midiChannel = config.layerMidiChannel[activeLayer][btn];

    Serial.printf("[MIDI] Slot %d, Btn %d, Layer %d: %s %d Ch%d, Behavior=%s, %s\n",
                  slot, btn, activeLayer, midiType.c_str(), midiValue, midiChannel,
                  midiBehavior.c_str(), pressed ? "ON" : "OFF");

    if (midiValue >= 0) {
      if (midiType == "cc") {
        if (midiBehavior == "latching") {
          if (pressed) {
            ccLatchState[slot][btn] = !ccLatchState[slot][btn];
            sendMidiCC(midiValue, ccLatchState[slot][btn] ? 127 : 0, midiChannel);
            if (config.ledMode[btn] == "midi_out_triggered") {
              midiOutTriggerTime[slot][btn] = millis();
              updateDeviceLED(slot, btn, true);
              lastLEDState[slot][btn] = true;
            }
          }
          {
            bool newLEDState = (config.ledMode[btn] == "midi_out")
              ? ccLatchState[slot][btn] : pressed;
            if (newLEDState != lastLEDState[slot][btn]) {
              updateDeviceLED(slot, btn, newLEDState);
              lastLEDState[slot][btn] = newLEDState;
              lastLEDUpdate[slot][btn] = millis();
            }
          }
        } else {
          // Momentary CC
          sendMidiCC(midiValue, pressed ? 127 : 0, midiChannel);
          if (pressed && config.ledMode[btn] == "midi_out_triggered") {
            midiOutTriggerTime[slot][btn] = millis();
            updateDeviceLED(slot, btn, true);
            lastLEDState[slot][btn] = true;
          } else if (pressed != lastLEDState[slot][btn] &&
              (millis() - lastLEDUpdate[slot][btn] > LED_UPDATE_DEBOUNCE)) {
            updateDeviceLED(slot, btn, pressed);
            lastLEDState[slot][btn] = pressed;
            lastLEDUpdate[slot][btn] = millis();
          }
        }
      } else {
        // Note handling
        if (midiBehavior == "latching") {
          if (pressed) {
            noteLatchState[slot][btn] = !noteLatchState[slot][btn];
            sendMidiNote(midiValue, noteLatchState[slot][btn], midiChannel);
            if (config.ledMode[btn] == "midi_out_triggered") {
              midiOutTriggerTime[slot][btn] = millis();
              updateDeviceLED(slot, btn, true);
              lastLEDState[slot][btn] = true;
            }
          }
          if (config.ledMode[btn] != "midi_out_triggered") {
            bool newLEDState = (config.ledMode[btn] == "midi_out")
              ? noteLatchState[slot][btn] : pressed;
            if (newLEDState != lastLEDState[slot][btn]) {
              updateDeviceLED(slot, btn, newLEDState);
              lastLEDState[slot][btn] = newLEDState;
              lastLEDUpdate[slot][btn] = millis();
            }
          }
        } else {
          // Momentary Note
          sendMidiNote(midiValue, pressed, midiChannel);
          if (pressed && config.ledMode[btn] == "midi_out_triggered") {
            midiOutTriggerTime[slot][btn] = millis();
            updateDeviceLED(slot, btn, true);
            lastLEDState[slot][btn] = true;
          } else if (pressed != lastLEDState[slot][btn] &&
              (millis() - lastLEDUpdate[slot][btn] > LED_UPDATE_DEBOUNCE)) {
            updateDeviceLED(slot, btn, pressed);
            lastLEDState[slot][btn] = pressed;
            lastLEDUpdate[slot][btn] = millis();
          }
        }
      }
    }
}

void processPedalButtonChange(SlaveInfo* slave, int slot, bool pressed) {
  DeviceConfig& config = slave->config;
  if (config.layerMidiType[getCurrentActiveLayer()][1] == "pequod") {   // owned by PEQUOD
    anyButtonPressed = true;
    if (!pqHostConnected()) updateDeviceLED(slot, 1, pressed);
    return;
  }
  if (config.layerMidiType[getCurrentActiveLayer()][1] == "osc") {
    anyButtonPressed = true;
    processOscButton(slave, slot, 1, 1, pressed, getCurrentActiveLayer());
    return;
  }
    anyButtonPressed = true;
    int  activeLayer  = getCurrentActiveLayer();
    String midiType   = config.layerMidiType[activeLayer][1];
    int  midiNum      = config.layerMidiMapping[activeLayer][1];
    int  midiCh       = config.layerMidiChannel[activeLayer][1];
    String behavior   = config.layerMidiBehavior[activeLayer][1];

    if (behavior == "latching") {
      if (pressed) {
        ledOutputState[slot][1] = !ledOutputState[slot][1];
        if (midiType == "note")
          sendMidiNote(midiNum, ledOutputState[slot][1], midiCh);
        else if (midiType == "cc")
          sendMidiCC(midiNum, ledOutputState[slot][1] ? 127 : 0, midiCh);
      }
    } else {
      ledOutputState[slot][1] = pressed;
      if (midiType == "note")
        sendMidiNote(midiNum, pressed, midiCh);
      else if (midiType == "cc")
        sendMidiCC(midiNum, pressed ? 127 : 0, midiCh);
    }
    updateDeviceLED(slot, 1, ledOutputState[slot][1]);
}

// ═══════════════════════════════════════════════════════════════════════════
//  OSC (Open Sound Control) over UDP
//  All slot / button / layer numbers in addresses are 1-based.
//  OUT  <p>/slot/S/button/B i 1|0      <p>/slot/S/pedal f 0..1, i 0..127
//       <p>/slot/S/encoder i ±1        <p>/slot/S/encoder/button i 1|0
//       <p>/layer i L                  <p>/slot/S/layer i L
//       <p>/midi/out/note|cc  ch n v   <p>/midi/in/note|cc ch n v  (optional)
//  IN   same /button, /pedal, /encoder, /layer addresses act like the hardware;
//       <p>/slot/S/led/B  i 0|1 | r g b | s "#RRGGBB"   <p>/slot/S/led/B/auto
//       <p>/led/auto   <p>/layer/next   <p>/layer/prev
//       <p>/midi/note ch n v   <p>/midi/cc ch n v   <p>/midi/panic
//       <p>/sync   <p>/ping
// ═══════════════════════════════════════════════════════════════════════════
struct OscConfig {
  bool     enabled       = false;
  uint16_t inPort        = 8000;
  uint16_t outPort       = 9000;
  String   targetIP      = "";      // blank = reply to whoever last sent us OSC
  String   prefix        = "/fc";
  bool     sendControls  = false;
  bool     mirrorMidiOut = false;
  bool     mirrorMidiIn  = false;
} oscCfg;

WiFiUDP   oscUdp;
bool      oscRunning    = false;
bool      oscInjecting  = false;   // true while running an OSC virtual press
IPAddress oscTarget;
bool      oscHaveTarget = false;
IPAddress oscLastSender;
bool      oscHaveSender = false;
uint32_t  oscRxCount = 0, oscTxCount = 0;
bool      oscVirtualBtn[8][4] = {{false}};

const int OSC_LOG_SIZE = 30;
String oscLogBuf[OSC_LOG_SIZE];
int    oscLogHead = 0, oscLogCount = 0;
void oscLog(const String& line) {
  oscLogBuf[oscLogHead] = String(millis() / 1000.0, 1) + "s " + line;
  oscLogHead = (oscLogHead + 1) % OSC_LOG_SIZE;
  if (oscLogCount < OSC_LOG_SIZE) oscLogCount++;
}

String oscNormalizePrefix(String pfx) {
  pfx.trim();
  if (pfx.length() && pfx[0] != '/') pfx = "/" + pfx;
  while (pfx.endsWith("/")) pfx.remove(pfx.length() - 1);
  return pfx;   // "" means no prefix
}

void ensureMDNS() {
  static bool started = false;
  if (!started) started = MDNS.begin("i2c-midi2");
}

void oscSend(OscMsg& m) {
  if (!oscRunning) return;
  IPAddress ip;
  if (oscHaveTarget) ip = oscTarget;
  else if (oscHaveSender) ip = oscLastSender;
  else return;   // nobody to talk to yet
  uint8_t buf[256];
  int n = m.build(buf, sizeof(buf));
  if (n <= 0) return;
  oscUdp.beginPacket(ip, oscCfg.outPort);
  oscUdp.write(buf, n);
  oscUdp.endPacket();
  oscTxCount++;
  oscLog("> " + m.txt);
}

// Send a user-configured OSC action (full address, no prefix). Returns false
// if OSC is off, the address is invalid, or there's nobody to send to yet.
bool oscSendAction(const String& addr, const String& argType, float val) {
  if (!oscRunning || addr.length() == 0 || addr[0] != '/') return false;
  if (!oscHaveTarget && !oscHaveSender) return false;
  OscMsg m(addr);
  if (argType == "i") m.i((int32_t)lroundf(val));
  else if (argType != "none") m.f(val);
  oscSend(m);
  return true;
}

// LEDs in "osc_in" mode follow an incoming OSC address (value >= 0.5 = on;
// no value toggles). Returns true if any LED listened to this address.
bool oscDriveLeds(const String& addr, OscArg* a, int na) {
  bool hit = false;
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* sl = getSlaveInfoForSlot(slot);
    if (!sl) continue;
    for (int b = 0; b < 4; b++) {
      if (sl->config.ledMode[b] != "osc_in" || sl->config.ledOscAddr[b] != addr) continue;
      bool on;
      if (na == 0) on = !midiInLEDState[slot][b];
      else if (a[0].t == 's') { String v = a[0].s; v.toLowerCase(); on = (v == "on" || v == "true" || v == "1"); }
      else on = (a[0].t == 'f' ? a[0].f : (float)a[0].i) >= 0.5f;
      midiInLEDState[slot][b] = on;
      updateDeviceLED(slot, b, on);
      hit = true;
    }
  }
  return hit;
}

String oscA(const String& tail) { return oscCfg.prefix + tail; }
String oscSlotA(int slot, const String& tail) { return oscCfg.prefix + "/slot/" + String(slot + 1) + tail; }

// ── outgoing events ─────────────────────────────────────────────────────────
void oscOnButton(int slot, int humanBtn, bool pressed) {
  if (!oscRunning || !oscCfg.sendControls || oscInjecting) return;
  OscMsg m(oscSlotA(slot, "/button/" + String(humanBtn))); m.i(pressed ? 1 : 0); oscSend(m);
}
void oscOnPedal(int slot, int v127) {
  if (!oscRunning || !oscCfg.sendControls || oscInjecting) return;
  OscMsg m(oscSlotA(slot, "/pedal")); m.f(v127 / 127.0f).i(v127); oscSend(m);
}
void oscOnEncoder(int slot, int delta) {
  if (!oscRunning || !oscCfg.sendControls || oscInjecting) return;
  OscMsg m(oscSlotA(slot, "/encoder")); m.i(delta); oscSend(m);
}
void oscOnEncoderButton(int slot, bool pressed) {
  if (!oscRunning || !oscCfg.sendControls || oscInjecting) return;
  OscMsg m(oscSlotA(slot, "/encoder/button")); m.i(pressed ? 1 : 0); oscSend(m);
}
void oscSendLayer(int layer0) {
  if (!oscRunning || !oscCfg.sendControls) return;
  OscMsg m(oscA("/layer")); m.i(layer0 + 1); oscSend(m);
}
void oscCheckSlotLayers() {
  static int prev[8] = {0};
  for (int slot = 0; slot < 8; slot++) {
    int L = shiftStates[slot].currentLayer;
    if (L != prev[slot]) {
      prev[slot] = L;
      if (oscRunning && oscCfg.sendControls && getSlaveInfoForSlot(slot)) {
        OscMsg m(oscSlotA(slot, "/layer")); m.i(L + 1); oscSend(m);
      }
    }
  }
}
void oscMirrorMidiOut(bool isNote, uint8_t ch, uint8_t d1, uint8_t d2) {
  if (!oscRunning || !oscCfg.mirrorMidiOut) return;
  OscMsg m(oscA(isNote ? "/midi/out/note" : "/midi/out/cc")); m.i(ch).i(d1).i(d2); oscSend(m);
}
void oscOnMidiIn(uint8_t type, uint8_t ch, uint8_t d1, uint8_t d2) {
  if (!oscRunning || !oscCfg.mirrorMidiIn) return;
  const char* kind = (type == 0x90 || type == 0x80) ? "/midi/in/note" : (type == 0xB0) ? "/midi/in/cc" : nullptr;
  if (!kind) return;
  OscMsg m(oscA(kind)); m.i(ch).i(d1).i(type == 0x80 ? 0 : d2); oscSend(m);
}
void oscSendSync() {
  OscMsg L(oscA("/layer")); L.i(getCurrentActiveLayer() + 1); oscSend(L);
  OscMsg N(oscA("/layers")); N.i(configNumLayers); oscSend(N);
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* sl = getSlaveInfoForSlot(slot);
    if (!sl) continue;
    OscMsg t(oscSlotA(slot, "/type")); t.str(typeToName(sl->type)); oscSend(t);
    OscMsg l(oscSlotA(slot, "/layer")); l.i(shiftStates[slot].currentLayer + 1); oscSend(l);
  }
}

// ── layer helpers (also used by the web UI) ─────────────────────────────────
void applyGlobalLayer(int layer) {
  layer = constrain(layer, 0, configNumLayers - 1);
  globalLayerControl.enabled = true;
  globalLayerControl.currentGlobalLayer = layer;
  for (int i = 0; i < 8; i++) shiftStates[i].currentLayer = 0;
  updateLayerLED(layer);
  spiffsConfig.saveGlobalConfig(true, layer);
}

// ── incoming ────────────────────────────────────────────────────────────────

float oscArgF(const OscArg& a) {
  if (a.t == 'f') return a.f;
  if (a.t == 'i') return (float)a.i;
  if (a.t == 's') return a.s.toFloat();
  return 0;
}
int oscArgI(const OscArg& a) {
  if (a.t == 'f') return (int)lroundf(a.f);
  if (a.t == 'i') return a.i;
  if (a.t == 's') return a.s.toInt();
  return 0;
}

// Physical button index for a 1-based OSC button number, or -1.
int oscButtonIndex(SlaveInfo* sl, int humanBtn) {
  if (sl->type == 41) return humanBtn == 1 ? 1 : -1;           // pedal + button
  if ((sl->type >= TYPE_FOOTSWITCH_MIN && sl->type <= TYPE_FOOTSWITCH_MAX) ||
      (sl->type >= TYPE_BUTTON_MIN && sl->type <= TYPE_BUTTON_MAX))
    return (humanBtn >= 1 && humanBtn <= 4) ? humanBtn - 1 : -1;
  return -1;
}

void oscVirtualPress(int slot, int idx, bool pressed) {
  SlaveInfo* sl = getSlaveInfoForSlot(slot);
  if (!sl) return;
  bool prev = oscVirtualBtn[slot][idx];
  if (prev == pressed) return;
  oscVirtualBtn[slot][idx] = pressed;
  oscInjecting = true;
  if (sl->type == 41) processPedalButtonChange(sl, slot, pressed);
  else                processButtonChange(sl, slot, idx, pressed, prev);
  oscInjecting = false;
}

void oscSetLED(int slot, int idx, bool on, CRGB color) {
  SlaveInfo* sl = getSlaveInfoForSlot(slot);
  if (!sl) return;
  ledOverrideOn[slot][idx] = on;
  ledOverrideColor[slot][idx] = color;
  updateDeviceLED(slot, idx, buttonLEDDesiredState[slot][idx]);
}

void oscDispatch(const String& addr, OscArg* a, int na) {
  bool ledHit = oscDriveLeds(addr, a, na);
  const String& P = oscCfg.prefix;
  if (P.length() && !addr.startsWith(P)) { if (!ledHit) oscLog("? ignored (prefix) " + addr); return; }
  String tail = addr.substring(P.length());
  const char* t = tail.c_str();
  bool hasArg = na > 0;
  bool on = hasArg ? (oscArgF(a[0]) >= 0.5f) : true;
  int S, B, c;

  if (tail == "/ping") { OscMsg m(oscA("/pong")); m.str(wifiHostname); oscSend(m); return; }
  if (tail == "/sync") { oscSendSync(); return; }

  if (tail == "/layer" && hasArg) { applyGlobalLayer(oscArgI(a[0]) - 1); return; }
  if (tail == "/layer/next" || tail == "/layer/prev") {
    if (!on) return;   // ignore button release
    int cur = getCurrentActiveLayer();
    int nl = (tail == "/layer/next") ? (cur + 1) % configNumLayers : (cur - 1 + configNumLayers) % configNumLayers;
    applyGlobalLayer(nl);
    return;
  }
  if (tail == "/led/auto") {
    for (int sl = 0; sl < 8; sl++) for (int b = 0; b < 4; b++)
      if (ledOverrideOn[sl][b]) oscSetLED(sl, b, false, CRGB::Black);
    return;
  }
  if (tail == "/midi/note" && na >= 2) {
    int ch = constrain(oscArgI(a[0]), 1, 16), note = constrain(oscArgI(a[1]), 0, 127);
    int vel = na >= 3 ? constrain(oscArgI(a[2]), 0, 127) : 127;
    sendMidiNote(note, vel > 0, ch, vel > 0 ? vel : 0);
    return;
  }
  if (tail == "/midi/cc" && na >= 3) {
    sendMidiCC(constrain(oscArgI(a[1]), 0, 127), constrain(oscArgI(a[2]), 0, 127), constrain(oscArgI(a[0]), 1, 16));
    return;
  }
  if (tail == "/midi/panic") {
    for (uint8_t ch = 1; ch <= 16; ch++) { sendMidiCC(123, 0, ch); sendMidiCC(120, 0, ch); }
    return;
  }

  // ---- per-slot addresses ----
  c = -1;
  if (sscanf(t, "/slot/%d/button/%d%n", &S, &B, &c) == 2 && c > 0 && t[c] == 0) {
    SlaveInfo* sl = getSlaveInfoForSlot(S - 1);
    int idx = sl ? oscButtonIndex(sl, B) : -1;
    if (idx < 0) { oscLog("? no such button " + addr); return; }
    if (!hasArg) { oscVirtualPress(S - 1, idx, true); oscVirtualPress(S - 1, idx, false); }  // tap
    else oscVirtualPress(S - 1, idx, on);
    return;
  }
  c = -1;
  if (sscanf(t, "/slot/%d/led/%d/auto%n", &S, &B, &c) == 2 && c > 0 && t[c] == 0) {
    SlaveInfo* sl = getSlaveInfoForSlot(S - 1);
    int idx = sl ? oscButtonIndex(sl, B) : -1;
    if (idx >= 0) oscSetLED(S - 1, idx, false, CRGB::Black);
    return;
  }
  c = -1;
  if (sscanf(t, "/slot/%d/led/%d%n", &S, &B, &c) == 2 && c > 0 && t[c] == 0) {
    SlaveInfo* sl = getSlaveInfoForSlot(S - 1);
    int idx = sl ? oscButtonIndex(sl, B) : -1;
    if (idx < 0 || !hasArg) { oscLog("? bad LED message " + addr); return; }
    DeviceConfig& cfg = sl->config;
    CRGB own = cfg.ledFollowLayer[idx] ? getCustomLayerColor(getCurrentActiveLayer()) : hexToRGB(cfg.ledColor[idx]);
    if (a[0].t == 's') {
      String v = a[0].s; v.toLowerCase();
      if (v == "auto")      oscSetLED(S - 1, idx, false, CRGB::Black);
      else if (v == "on")   oscSetLED(S - 1, idx, true, own);
      else if (v == "off")  oscSetLED(S - 1, idx, true, CRGB::Black);
      else                  oscSetLED(S - 1, idx, true, hexToRGB(a[0].s));
    } else if (na >= 3) {
      bool norm = (a[0].t == 'f' || a[1].t == 'f' || a[2].t == 'f') &&
                  oscArgF(a[0]) <= 1.0f && oscArgF(a[1]) <= 1.0f && oscArgF(a[2]) <= 1.0f;
      float k = norm ? 255.0f : 1.0f;
      oscSetLED(S - 1, idx, true, CRGB(constrain((int)(oscArgF(a[0]) * k), 0, 255),
                                       constrain((int)(oscArgF(a[1]) * k), 0, 255),
                                       constrain((int)(oscArgF(a[2]) * k), 0, 255)));
    } else {
      oscSetLED(S - 1, idx, true, on ? own : CRGB::Black);
    }
    return;
  }
  c = -1;
  if (sscanf(t, "/slot/%d/pedal%n", &S, &c) == 1 && c > 0 && t[c] == 0 && hasArg) {
    SlaveInfo* sl = getSlaveInfoForSlot(S - 1);
    if (!sl || !(sl->type >= TYPE_PEDAL_MIN && sl->type <= TYPE_PEDAL_MAX)) { oscLog("? not a pedal " + addr); return; }
    int v = (a[0].t == 'f' && a[0].f <= 1.0f) ? (int)lroundf(a[0].f * 127) : oscArgI(a[0]);
    v = constrain(v, 0, 127);
    DeviceConfig& cfg = sl->config;
    if (cfg.layerMidiType[0][0] == "cc" && cfg.layerMidiMapping[0][0] >= 0)
      sendMidiCC(cfg.layerMidiMapping[0][0], v, cfg.layerMidiChannel[0][0]);
    return;
  }
  c = -1;
  if (sscanf(t, "/slot/%d/encoder/button%n", &S, &c) == 1 && c > 0 && t[c] == 0) {
    SlaveInfo* sl = getSlaveInfoForSlot(S - 1);
    if (!sl || !(sl->type >= TYPE_ENCODER_MIN && sl->type <= TYPE_ENCODER_MAX)) return;
    oscInjecting = true;
    if (!hasArg) { handleI2CEncoderButton(S - 1, true, sl->config); handleI2CEncoderButton(S - 1, false, sl->config); }
    else handleI2CEncoderButton(S - 1, on, sl->config);
    oscInjecting = false;
    return;
  }
  c = -1;
  if (sscanf(t, "/slot/%d/encoder%n", &S, &c) == 1 && c > 0 && t[c] == 0 && hasArg) {
    SlaveInfo* sl = getSlaveInfoForSlot(S - 1);
    if (!sl || !(sl->type >= TYPE_ENCODER_MIN && sl->type <= TYPE_ENCODER_MAX)) return;
    int steps = constrain(oscArgI(a[0]), -32, 32);
    if (steps) { oscInjecting = true; handleI2CEncoderRotation(S - 1, steps, sl->config); oscInjecting = false; }
    return;
  }
  c = -1;
  if (sscanf(t, "/slot/%d/layer%n", &S, &c) == 1 && c > 0 && t[c] == 0 && hasArg) {
    if (S < 1 || S > 8) return;
    shiftStates[S - 1].currentLayer = constrain(oscArgI(a[0]) - 1, 0, configNumLayers - 1);
    updateLayerLED(getCurrentActiveLayer());
    return;
  }
  if (!ledHit) oscLog("? unknown " + addr);
}

int oscReadStr(const uint8_t* b, int n, int pos, String& out) {
  int start = pos;
  while (pos < n && b[pos]) pos++;
  if (pos >= n) return -1;
  out = String((const char*)(b + start));
  pos++;
  return (pos + 3) & ~3;
}
uint32_t oscBE32(const uint8_t* b) { return ((uint32_t)b[0] << 24) | ((uint32_t)b[1] << 16) | ((uint32_t)b[2] << 8) | b[3]; }

void oscHandlePacket(const uint8_t* b, int n, int depth = 0) {
  if (n < 4 || depth > 4) return;
  if (n >= 16 && memcmp(b, "#bundle", 8) == 0) {
    int pos = 16;   // skip "#bundle\0" + 8-byte timetag
    while (pos + 4 <= n) {
      int sz = (int)oscBE32(b + pos);
      if (sz <= 0 || pos + 4 + sz > n) break;
      oscHandlePacket(b + pos + 4, sz, depth + 1);
      pos += 4 + sz;
    }
    return;
  }
  String addr, tags;
  int pos = oscReadStr(b, n, 0, addr);
  if (pos < 0 || addr.length() == 0 || addr[0] != '/') return;
  OscArg args[8]; int na = 0;
  if (pos < n && b[pos] == ',') {
    pos = oscReadStr(b, n, pos, tags);
    if (pos < 0) return;
    for (unsigned k = 1; k < tags.length() && na < 8; k++) {
      char ty = tags[k];
      OscArg& A = args[na];
      A.t = ty; A.i = 0; A.f = 0;
      if (ty == 'i' || ty == 'f') {
        if (pos + 4 > n) break;
        uint32_t u = oscBE32(b + pos); pos += 4;
        if (ty == 'i') A.i = (int32_t)u; else memcpy(&A.f, &u, 4);
      } else if (ty == 'd' || ty == 'h') {
        if (pos + 8 > n) break;
        uint64_t u = ((uint64_t)oscBE32(b + pos) << 32) | oscBE32(b + pos + 4); pos += 8;
        if (ty == 'd') { double d; memcpy(&d, &u, 8); A.t = 'f'; A.f = (float)d; }
        else { A.t = 'i'; A.i = (int32_t)u; }
      } else if (ty == 's') {
        pos = oscReadStr(b, n, pos, A.s);
        if (pos < 0) break;
      } else if (ty == 'T' || ty == 'F') {
        A.t = 'i'; A.i = (ty == 'T');
      } else {
        break;   // unsupported type: stop reading args
      }
      na++;
    }
  }
  String line = "< " + addr;
  for (int k = 0; k < na; k++) {
    line += " ";
    if (args[k].t == 'f') line += String(args[k].f, 3);
    else if (args[k].t == 's') line += "\"" + args[k].s + "\"";
    else line += String(args[k].i);
  }
  oscLog(line);
  oscDispatch(addr, args, na);
}

void stopOSC() {
  if (oscRunning) { oscUdp.stop(); oscRunning = false; Serial.println("[OSC] Stopped"); }
}
void startOSC() {
  stopOSC();
  if (!oscCfg.enabled) return;
  oscHaveTarget = oscCfg.targetIP.length() && oscTarget.fromString(oscCfg.targetIP);
  if (!oscUdp.begin(oscCfg.inPort)) { Serial.println("[OSC] UDP begin failed"); oscLog("! could not open port " + String(oscCfg.inPort)); return; }
  oscRunning = true;
  ensureMDNS();
  MDNS.addService("osc", "udp", oscCfg.inPort);
  Serial.printf("[OSC] Listening on %u, sending to %s:%u\n", oscCfg.inPort,
                oscHaveTarget ? oscCfg.targetIP.c_str() : "<last sender>", oscCfg.outPort);
  oscLog("listening on port " + String(oscCfg.inPort));
}
void loopOSC() {
  if (!oscRunning) return;
  static uint8_t buf[768];
  for (int k = 0; k < 8; k++) {
    int sz = oscUdp.parsePacket();
    if (sz <= 0) break;
    oscLastSender = oscUdp.remoteIP();
    oscHaveSender = true;
    oscRxCount++;
    if (sz > (int)sizeof(buf)) { oscUdp.flush(); oscLog("! packet too big"); continue; }
    int n = oscUdp.read(buf, sizeof(buf));
    oscHandlePacket(buf, n);
  }
}
void loadOscConfig() {
  prefs.begin("osc", true);
  oscCfg.enabled       = prefs.getBool("en", false);
  oscCfg.inPort        = prefs.getUShort("in", 8000);
  oscCfg.outPort       = prefs.getUShort("out", 9000);
  oscCfg.targetIP      = prefs.getString("ip", "");
  oscCfg.prefix        = oscNormalizePrefix(prefs.getString("pfx", "/fc"));
  oscCfg.sendControls  = prefs.getBool("ctl", false);
  oscCfg.mirrorMidiOut = prefs.getBool("mout", false);
  oscCfg.mirrorMidiIn  = prefs.getBool("min", false);
  prefs.end();
}

void pollDevicesWithShift() {
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (!slave) continue;

    if (millis() - slave->lastFailTime < 50) continue;

    // Handle buttons and footswitches with shift support
    if ((slave->type >= TYPE_BUTTON_MIN && slave->type <= TYPE_BUTTON_MAX) || 
        (slave->type >= TYPE_FOOTSWITCH_MIN && slave->type <= TYPE_FOOTSWITCH_MAX)) {

      // Send all LED states to slave BEFORE reading buttons.
      // This minimises the window where LEDs are dark — the slave has the
      // correct state before it processes the 0x02 read command.
      {
        CRGB leds[5] = {CRGB::Black, CRGB::Black, CRGB::Black, CRGB::Black, CRGB::Black};
        for (int btn = 0; btn < 4; btn++) {
          if (ledOverrideOn[slot][btn]) {
            leds[btn + 1] = ledOverrideColor[slot][btn];
          } else if (buttonLEDDesiredState[slot][btn]) {
            String mode = slave->config.ledMode[btn];
            leds[btn + 1] = (mode == "layer" || slave->config.ledFollowLayer[btn])
                            ? getCustomLayerColor(getCurrentActiveLayer())
                            : hexToRGB(slave->config.ledColor[btn]);
          }
        }
        {
          static unsigned long _ledDbg = 0;
          if (millis() - _ledDbg > 500) {  // throttle so serial isn't flooded
            _ledDbg = millis();
            for (int b = 0; b < 4; b++) {
              Serial.printf("[LEDTX] slot%d btn%d mode=%s follow=%d color=%s desired=%d -> [%d,%d,%d]\n",
                slot, b,
                slave->config.ledMode[b].c_str(),
                slave->config.ledFollowLayer[b],
                slave->config.ledColor[b].c_str(),
                buttonLEDDesiredState[slot][b],
                leds[b + 1].r, leds[b + 1].g, leds[b + 1].b);
            }
          }
        }
        setAllDeviceLEDs(slave->i2cAddress, leds);
      }

      Wire.beginTransmission(slave->i2cAddress);
      Wire.write(0x02);
      uint8_t err = Wire.endTransmission();
      if (err != 0) {
        slave->lastFailTime = millis();
        continue;
      }

      Wire.requestFrom(slave->i2cAddress, (uint8_t)1);
      if (Wire.available() < 1) continue;
      uint8_t states = Wire.read();

      ShiftState& shiftState = shiftStates[slot];
      DeviceConfig& config = slave->config;

      // Handle shift button logic
      if (config.shiftEnabled && config.shiftButton >= 0 && config.shiftButton < 4) {
        bool shiftPressed = states & (1 << config.shiftButton);
        bool shiftPrevPressed = slave->lastState & (1 << config.shiftButton);

        if (config.shiftMode) {
          shiftState.isShiftPressed = shiftPressed;
          shiftState.currentLayer = shiftPressed ? 1 : 0;
        } else {
          if (shiftPressed && !shiftPrevPressed) {
            shiftState.shiftToggled = !shiftState.shiftToggled;
            shiftState.currentLayer = shiftState.shiftToggled ? 1 : 0;
          }
        }
      }

      // Process all buttons
      for (int btn = 0; btn < 4; btn++) {
        bool pressed = states & (1 << btn);
        bool prev = slave->lastState & (1 << btn);
        if (pressed) lastButtonLEDUpdate[slot][btn] = millis();
        if (pressed != prev) {
          oscOnButton(slot, btn + 1, pressed);
          pqOnButton(slot, btn, pressed);
          processButtonChange(slave, slot, btn, pressed, prev);
        }
      } // End of button loop
      
      slave->prevState = slave->lastState;
      slave->lastState = states;
    }
    // Pedal handling
    else if (slave->type >= TYPE_PEDAL_MIN && slave->type <= TYPE_PEDAL_MAX) {
      bool hasBtnLed = (slave->type == 41);  // pedal + button + LED variant

      // Pre-send LED state for pedal+button devices (same protocol as button slaves)
      if (hasBtnLed) {
        CRGB leds[5] = {CRGB::Black, CRGB::Black, CRGB::Black, CRGB::Black, CRGB::Black};
        if (ledOverrideOn[slot][1]) {
          leds[1] = ledOverrideColor[slot][1];
        } else if (buttonLEDDesiredState[slot][1]) {
          String mode = slave->config.ledMode[1];
          leds[1] = (mode == "layer" || slave->config.ledFollowLayer[1])
                    ? getCustomLayerColor(getCurrentActiveLayer())
                    : hexToRGB(slave->config.ledColor[1]);
        }
        setAllDeviceLEDs(slave->i2cAddress, leds);
      }

      Wire.beginTransmission(slave->i2cAddress);
      Wire.write(0x02);  // command 0x02 = send 16-bit pot value (high byte, low byte)
      uint8_t err = Wire.endTransmission();
      if (err != 0) {
        slave->lastFailTime = millis();
        continue;
      }

      uint8_t numBytes = hasBtnLed ? 3 : 2;
      Wire.requestFrom(slave->i2cAddress, numBytes);
      if (Wire.available() < 2) continue;

      int rawValue = (Wire.read() << 8) | Wire.read();

      PedalRawData& data = pedalRawReadouts[slot];
      data.rawValue  = rawValue;
      data.lastUpdate = millis();

      DeviceConfig& config = slave->config;
      int calMin   = config.pedalCalMin;
      int calMax   = config.pedalCalMax;
      int calibrated = map(constrain(rawValue, calMin, calMax), calMin, calMax, 0, 127);
      if (config.pedalInvert) calibrated = 127 - calibrated;
      data.calibratedValue = calibrated;

      if (abs(calibrated - slave->lastState) >= 2) {
        String midiType    = config.layerMidiType[0][0];
        int    midiValue   = config.layerMidiMapping[0][0];
        int    midiChannel = config.layerMidiChannel[0][0];
        if (midiType == "cc" && midiValue >= 0)
          sendMidiCC(midiValue, calibrated, midiChannel);
        else if (midiType == "osc") {
          float lo = config.layerOscOff[0][0], hi = config.layerOscOn[0][0];
          oscSendAction(config.layerOscAddr[0][0], config.layerOscArg[0][0], lo + (hi - lo) * (calibrated / 127.0f));
        }
        slave->lastState = calibrated;
        oscOnPedal(slot, calibrated);
        pqOnPedal(slot, calibrated, rawValue);
      }

      // Button processing for type 41 (pedal + button)
      // Uses config index 1 — index 0 is reserved for the pedal CC.
      if (hasBtnLed && Wire.available() >= 1) {
        uint8_t btnByte = Wire.read();
        bool pressed = (btnByte & 0x01);
        bool prev    = (slave->prevState & 0x01);
        if (pressed) lastButtonLEDUpdate[slot][1] = millis();
        if (pressed != prev) {
          oscOnButton(slot, 1, pressed);
          pqOnButton(slot, 1, pressed);
          processPedalButtonChange(slave, slot, pressed);
        }
        slave->prevState = btnByte;
      }
    }
  }
}








void handleSaveDeviceControl() {
  // Parse JSON body for POST request
  if (server.method() == HTTP_POST) {
    String body = server.arg("plain");
    if (body.length() == 0) {
      server.send(400, "text/plain", "No JSON body");
      return;
    }

    Serial.println("[SAVE DC] Received JSON: " + body);

    // Simple JSON parsing for our specific case
    int device = -1, button = -1, targetLayer = 0;
    String controlType = "none";
    String mode = "momentary";  // CHANGED

    // Extract values from JSON string
    int deviceStart = body.indexOf("\"device\":") + 9;
    if (deviceStart > 8) {
      int deviceEnd = body.indexOf(",", deviceStart);
      if (deviceEnd == -1) deviceEnd = body.indexOf("}", deviceStart);
      device = body.substring(deviceStart, deviceEnd).toInt();
    }

    int buttonStart = body.indexOf("\"button\":") + 9;
    if (buttonStart > 8) {
      int buttonEnd = body.indexOf(",", buttonStart);
      if (buttonEnd == -1) buttonEnd = body.indexOf("}", buttonStart);
      button = body.substring(buttonStart, buttonEnd).toInt();
    }

    int typeStart = body.indexOf("\"controlType\":\"") + 15;
    if (typeStart > 14) {
      int typeEnd = body.indexOf("\"", typeStart);
      controlType = body.substring(typeStart, typeEnd);
    }

    int layerStart = body.indexOf("\"targetLayer\":") + 14;
    if (layerStart > 13) {
      int layerEnd = body.indexOf(",", layerStart);
      if (layerEnd == -1) layerEnd = body.indexOf("}", layerStart);
      targetLayer = body.substring(layerStart, layerEnd).toInt();
    }

    // CHANGED: Parse mode instead of momentary
    int modeStart = body.indexOf("\"mode\":\"") + 8;
    if (modeStart > 7) {
      int modeEnd = body.indexOf("\"", modeStart);
      mode = body.substring(modeStart, modeEnd);
    }

    Serial.printf("[SAVE DC] Parsed: Device=%d, Button=%d, Type=%s, Target=%d, Mode=%s\n",
                  device, button, controlType.c_str(), targetLayer, mode.c_str());

    if (device < 0 || device >= 8 || button < 0 || button >= 4) {
      server.send(400, "text/plain", "Invalid parameters");
      return;
    }

    SlaveInfo* slave = getSlaveInfoForSlot(device);
    if (!slave) {
      server.send(400, "text/plain", "No device mapped to this slot");
      return;
    }

    // Update device control configuration
    slave->config.deviceControlType[button] = controlType;
    slave->config.deviceControlTarget[button] = constrain(targetLayer, 0, 3);
    slave->config.deviceControlMode[button] = mode;  // CHANGED

    // Save to preferences
    saveDeviceConfigWithLayers(deviceMapping[device]);

    Serial.println("[SAVE DC] Configuration saved successfully");
    server.send(200, "text/plain", "OK");
  } else {
    server.send(405, "text/plain", "Method not allowed");
  }
}

void handleGetDeviceControl() {
  if (!server.hasArg("device") || !server.hasArg("button")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }

  int slot = server.arg("device").toInt();
  int button = server.arg("button").toInt();

  if (slot < 0 || slot >= 8 || button < 0 || button >= 4) {
    server.send(400, "text/plain", "Invalid parameters");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave) {
    server.send(404, "text/plain", "No device mapped to this slot");
    return;
  }

  // Debug output
  Serial.printf("[GET_DEVICE_CONTROL] Slot %d Btn %d: Type='%s'\n",
                slot, button, slave->config.deviceControlType[button].c_str());

  String json = "{";
  json += "\"controlType\":\"" + slave->config.deviceControlType[button] + "\",";
  json += "\"targetLayer\":" + String(slave->config.deviceControlTarget[button]) + ",";
  json += "\"mode\":\"" + slave->config.deviceControlMode[button] + "\"";
  json += "}";

  server.send(200, "application/json", json);
}

void handleSaveGlobalLayerControl() {
  if (server.hasArg("enabled")) {
    globalLayerControl.enabled = server.arg("enabled") == "true";
    spiffsConfig.saveGlobalConfig(globalLayerControl.enabled,
                                  globalLayerControl.currentGlobalLayer);
    server.send(200, "text/plain", "OK");
  } else {
    server.send(400, "text/plain", "Missing enabled parameter");
  }
}




// Web handlers for shift/layer configuration
void handleSaveShiftConfig() {
  if (!server.hasArg("device") || !server.hasArg("shiftButton")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }

  int slot = server.arg("device").toInt();
  int shiftButton = server.arg("shiftButton").toInt();
  bool shiftMode = server.hasArg("shiftMode") ? (server.arg("shiftMode") == "true") : true;
  bool shiftEnabled = server.hasArg("shiftEnabled") ? (server.arg("shiftEnabled") == "true") : false;

  if (slot < 0 || slot >= 8) {
    server.send(400, "text/plain", "Invalid slot");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave) {
    server.send(400, "text/plain", "No device mapped to this slot");
    return;
  }

  // Update configuration
  slave->config.shiftButton = (shiftButton >= 0 && shiftButton < 4) ? shiftButton : -1;
  slave->config.shiftMode = shiftMode;
  slave->config.shiftEnabled = shiftEnabled;

  // Reset shift state
  shiftStates[slot] = { false, false, 0, 0 };

  // Save to preferences
  saveDeviceConfigWithLayers(deviceMapping[slot]);

  server.send(200, "text/plain", "OK");
}

void handleGetLayerMidi() {
  if (!server.hasArg("device") || !server.hasArg("layer") || !server.hasArg("button")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }

  int slot = server.arg("device").toInt();
  int layer = server.arg("layer").toInt();
  int button = server.arg("button").toInt();

  if (slot < 0 || slot >= 8 || layer < 0 || layer >= 4 || button < 0 || button >= 4) {
    server.send(400, "text/plain", "Invalid parameters");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave) {
    server.send(404, "text/plain", "No device mapped to this slot");
    return;
  }

  // Return regular MIDI settings
  String json = "{";
  json += "\"type\":\"" + slave->config.layerMidiType[layer][button] + "\",";
  json += "\"number\":" + String(slave->config.layerMidiMapping[layer][button]) + ",";
  json += "\"channel\":" + String(slave->config.layerMidiChannel[layer][button]) + ",";
  json += "\"behavior\":\"" + slave->config.layerMidiBehavior[layer][button] + "\",";
  json += "\"oscAddr\":\"" + jsonEsc(slave->config.layerOscAddr[layer][button]) + "\",";
  json += "\"oscOn\":" + String(slave->config.layerOscOn[layer][button], 4) + ",";
  json += "\"oscOff\":" + String(slave->config.layerOscOff[layer][button], 4) + ",";
  json += "\"oscArg\":\"" + slave->config.layerOscArg[layer][button] + "\",";
  json += "\"deviceControl\":\"" + slave->config.deviceControlType[button] + "\"";
  json += "}";

  server.send(200, "application/json", json);
}

void handleSaveLayerMidi() {
  if (server.method() != HTTP_POST) { server.send(405, "text/plain", "Method not allowed"); return; }
  DynamicJsonDocument doc(1024);
  DeserializationError err = deserializeJson(doc, server.arg("plain"));
  if (err) { server.send(400, "text/plain", String("JSON parse error: ") + err.c_str()); return; }

  int device = doc["device"] | -1, layer = doc["layer"] | -1, button = doc["button"] | -1;
  if (device < 0 || device >= 8 || layer < 0 || layer >= 4 || button < 0 || button >= 4) {
    server.send(400, "text/plain", "Invalid parameters");
    return;
  }
  SlaveInfo* slave = getSlaveInfoForSlot(device);
  if (!slave) { server.send(400, "text/plain", "No device mapped to this slot"); return; }
  DeviceConfig& c = slave->config;

  if (doc["clearDeviceControl"] | false) {
    c.deviceControlType[button] = "none";
    c.deviceControlTarget[button] = 0;
    c.deviceControlMode[button] = "momentary";
  }
  String type = doc["type"] | c.layerMidiType[layer][button];
  if (type != "note" && type != "cc" && type != "osc") type = "note";
  c.layerMidiType[layer][button]     = type;
  c.layerMidiMapping[layer][button]  = constrain((int)(doc["number"]  | c.layerMidiMapping[layer][button]), 0, 127);
  c.layerMidiChannel[layer][button]  = constrain((int)(doc["channel"] | c.layerMidiChannel[layer][button]), 1, 16);
  c.layerMidiBehavior[layer][button] = doc["behavior"] | c.layerMidiBehavior[layer][button];
  c.layerOscAddr[layer][button]      = doc["oscAddr"]  | c.layerOscAddr[layer][button];
  c.layerOscOn[layer][button]        = doc["oscOn"]    | c.layerOscOn[layer][button];
  c.layerOscOff[layer][button]       = doc["oscOff"]   | c.layerOscOff[layer][button];
  c.layerOscArg[layer][button]       = doc["oscArg"]   | c.layerOscArg[layer][button];
  c.layerOscAddr[layer][button].trim();

  saveDeviceConfigWithLayers(deviceMapping[device]);
  Serial.printf("[SAVE LAYER] slot %d L%d B%d type=%s\n", device, layer, button, type.c_str());
  server.send(200, "text/plain", "OK");
}


void handleGetDeviceShiftConfig() {
  if (!server.hasArg("device")) {
    server.send(400, "text/plain", "Missing device parameter");
    return;
  }

  int slot = server.arg("device").toInt();
  if (slot < 0 || slot >= 8) {
    server.send(400, "text/plain", "Invalid slot");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave) {
    server.send(400, "text/plain", "No device mapped to this slot");
    return;
  }

  String json = "{";
  json += "\"shiftButton\":" + String(slave->config.shiftButton) + ",";
  json += "\"shiftMode\":" + String(slave->config.shiftMode ? "true" : "false") + ",";
  json += "\"shiftEnabled\":" + String(slave->config.shiftEnabled ? "true" : "false") + ",";
  json += "\"currentLayer\":" + String(shiftStates[slot].currentLayer) + ",";
  json += "\"layers\":[";

  for (int layer = 0; layer < 4; layer++) {
    if (layer > 0) json += ",";
    json += "{\"layer\":" + String(layer) + ",\"buttons\":[";
    for (int btn = 0; btn < 4; btn++) {
      if (btn > 0) json += ",";
      json += "{\"midiMapping\":" + String(slave->config.layerMidiMapping[layer][btn]);
      json += ",\"midiType\":\"" + slave->config.layerMidiType[layer][btn] + "\"";
      json += ",\"midiChannel\":" + String(slave->config.layerMidiChannel[layer][btn]) + "}";
    }
    json += "]}";
  }
  json += "]}";

  server.send(200, "application/json", json);
}

void initializeShiftStates() {
  for (int i = 0; i < 8; i++) {
    shiftStates[i] = { false, false, 0, 0 };
  }
}










// Device Configuration Management
void loadDeviceConfig(int slaveIndex) {
  loadDeviceConfigWithLayers(slaveIndex);
}

void saveDeviceConfig(int slaveIndex) {
  saveDeviceConfigWithLayers(slaveIndex);
}

// I2C helpers
bool getSlaveUniqueID(uint8_t address, uint64_t& id) {
  Wire.beginTransmission(address);
  Wire.write(0x00);
  if (Wire.endTransmission() != 0) return false;
  Wire.requestFrom(address, (uint8_t)8);
  if (Wire.available() < 8) return false;
  id = 0;
  for (int i = 0; i < 8; i++) {
    id <<= 8;
    id |= Wire.read();
  }
  return true;
}

bool getSlaveType(uint8_t address, uint8_t& type) {
  Wire.beginTransmission(address);
  Wire.write(0x01);
  if (Wire.endTransmission() != 0) return false;
  Wire.requestFrom(address, (uint8_t)1);
  if (Wire.available() < 1) return false;
  type = Wire.read();
  return true;
}

void scanSlaves() {
  size_t freeHeap = ESP.getFreeHeap();
  Serial.printf("Scanning I2C bus (free heap: %d bytes)...\n", freeHeap);
  if (freeHeap < MIN_FREE_HEAP)
    Serial.printf("WARNING: Low memory (%d bytes), scan proceeding anyway\n", freeHeap);

  numDetected = 0;

  uint8_t  tempAddresses[MAX_SLAVES];
  uint64_t tempIDs[MAX_SLAVES];
  uint8_t  tempTypes[MAX_SLAVES];

  for (uint8_t addr = 1; addr < 0x78 && numDetected < MAX_SLAVES; addr++) {
    uint64_t id;
    uint8_t  type;
    if (getSlaveUniqueID(addr, id) && getSlaveType(addr, type)) {
      tempAddresses[numDetected] = addr;
      tempIDs[numDetected]       = id;
      tempTypes[numDetected]     = type;
      numDetected++;
      Serial.printf("  Found 0x%02X → ID:0x%s Type:%s\n",
                    addr, idToHex(id).c_str(), typeToName(type).c_str());
    }
    delay(5);
    yield();
  }

  for (int i = 0; i < numDetected; i++)
    detectedSlaves[i] = { tempAddresses[i], tempIDs[i], tempTypes[i], 0, 0, 0 };

  Serial.printf("I2C scan complete — %d device(s) found.\n", numDetected);
}

void autoMapDevices() {
  Serial.println("Auto-mapping discovered devices by unique ID...");

  for (int i = 0; i < 8; i++) deviceMapping[i] = -1;

  // First pass: restore to preferred slots
  for (int d = 0; d < numDetected; d++) {
    SlaveInfo& slave = detectedSlaves[d];
    int preferredSlot = slave.config.preferredSlot;

    if (preferredSlot >= 0 && preferredSlot < 8 && deviceMapping[preferredSlot] == -1) {
      deviceMapping[preferredSlot] = d;
      Serial.printf("Restored device %s to preferred slot %d\n", idToShortKey(slave.uniqueID).c_str(), preferredSlot);
    }
  }

  // Second pass: assign remaining devices
  for (int d = 0; d < numDetected; d++) {
    bool alreadyMapped = false;
    for (int i = 0; i < 8; i++) {
      if (deviceMapping[i] == d) {
        alreadyMapped = true;
        break;
      }
    }

    if (!alreadyMapped) {
      for (int i = 0; i < 8; i++) {
        if (deviceMapping[i] == -1) {
          deviceMapping[i] = d;
          detectedSlaves[d].config.preferredSlot = i;
          saveDeviceConfig(d);
          Serial.printf("Auto-mapped device %s to slot %d\n", idToShortKey(detectedSlaves[d].uniqueID).c_str(), i);
          break;
        }
      }
    }
  }
  Serial.println("Auto-mapping complete.");
}

// FIXED: Poll devices - removed duplicate pedal handling code
void pollDevices() {
  pollDevicesWithShift();
}

// Wi-Fi Management
void startWiFi() {
  spiffsConfig.loadWiFiConfig(wifiPrimarySSID, wifiPrimaryPass,
                              wifiFallbackSSID, wifiFallbackPass, wifiHostname);

  WiFi.setHostname(wifiHostname.c_str());

  if (wifiPrimarySSID != "") {
    WiFi.begin(wifiPrimarySSID.c_str(), wifiPrimaryPass.c_str());
    wifiStartTime = millis();
    wifiState = WIFI_PRIMARY;
    Serial.println("Connecting to primary WiFi: " + wifiPrimarySSID);
  } else {
    wifiState = WIFI_AP_MODE;
    Serial.println("No WiFi configured, starting AP mode");
  }
}


void checkWiFi() {
  unsigned long now = millis();
  switch (wifiState) {
    case WIFI_PRIMARY:
      if (WiFi.status() == WL_CONNECTED) {
        wifiConnected = true;
        wifiState = WIFI_DONE;
        Serial.println("\nConnected to primary Wi-Fi! IP: " + WiFi.localIP().toString());
      } else if (now - wifiStartTime > 15000) {
        if (wifiFallbackSSID != "") {
          WiFi.begin(wifiFallbackSSID.c_str(), wifiFallbackPass.c_str());
          wifiStartTime = now;
          wifiState = WIFI_FALLBACK;
          Serial.println("\nTrying fallback hotspot...");
        } else wifiState = WIFI_AP_MODE;
      }
      break;
    case WIFI_FALLBACK:
      if (WiFi.status() == WL_CONNECTED) {
        wifiConnected = true;
        wifiState = WIFI_DONE;
        Serial.println("\nConnected to fallback hotspot! IP: " + WiFi.localIP().toString());
      } else if (now - wifiStartTime > 15000) {
        wifiState = WIFI_AP_MODE;
        Serial.println("\nPrimary/fallback failed. Starting AP...");
      }
      break;
    case WIFI_AP_MODE:
      if (!apModeActive) {
        WiFi.mode(WIFI_AP);
        WiFi.softAP("ESP32_Fallback");
        apModeActive = true;
        wifiConnected = false;
        Serial.println("\nFallback AP started. Connect to ESP32_Fallback");
      }
      wifiState = WIFI_DONE;
      break;
    case WIFI_DONE: break;
  }

  if ((wifiConnected || apModeActive) && !webServerStarted) {
    server.begin();
    webServerStarted = true;
    Serial.println("Web server started");
    // Start RTP-MIDI session if enabled and we have real WiFi (not AP-only)
    if (midi2Pref && wifiConnected) startMidi2Network();
    if (oscCfg.enabled) startOSC();
  }
}

// AJAX Handlers
void handleScanWiFiAjax() {
  int n = WiFi.scanNetworks();
  numScannedSSIDs = n;
  for (int i = 0; i < n && i < 32; i++) scannedSSIDs[i] = WiFi.SSID(i);

  String json = "[";
  for (int i = 0; i < numScannedSSIDs; i++) {
    json += "\"" + scannedSSIDs[i] + "\"";
    if (i < numScannedSSIDs - 1) json += ",";
  }
  json += "]";
  server.send(200, "application/json", json);
}

void handleWiFiStatusJson() {
  String status = (wifiState == WIFI_PRIMARY) ? "Primary" : (wifiState == WIFI_FALLBACK) ? "Fallback"
                                                          : (wifiState == WIFI_AP_MODE)  ? "AP Mode"
                                                                                         : "Connected";
  String ipStr = (wifiConnected && WiFi.status() == WL_CONNECTED) ? WiFi.localIP().toString() : (apModeActive ? "AP Mode" : "Not connected");
  String json = "{\"status\":\"" + status + "\",\"ip\":\"" + ipStr + "\"}";
  server.send(200, "application/json", json);
}

void handleDeviceStateJson() {
  String json = "{";
  bool first = true;
  for (int i = 0; i < 8; i++) {
    SlaveInfo* slave = getSlaveInfoForSlot(i);
    if (!slave) continue;
    if (!first) json += ",";
    first = false;
    json += "\"" + String(i) + "\":" + String(slave->lastState);
  }
  json += "}";
  server.send(200, "application/json", json);
}

void handleScanI2CAjax() {
  Serial.println("Manual I2C scan requested...");
  scanSlaves();
  Serial.println("Loading device configurations...");
  for (int i = 0; i < numDetected; i++) loadDeviceConfig(i);
  autoMapDevices();
  initializeShiftStates();
  server.send(200, "text/plain", "I2C scan complete");
}

// Save Handlers
void handleSaveWiFi() {
  String primarySSID = server.arg("ssid_primary");
  String primaryPass = server.arg("pass_primary");
  String fallbackSSID = server.arg("ssid_fallback");
  String fallbackPass = server.arg("pass_fallback");
  String hostname = server.arg("hostname");

  spiffsConfig.saveWiFiConfig(primarySSID, primaryPass, fallbackSSID, fallbackPass, hostname);

  // Update global variables
  wifiPrimarySSID = primarySSID;
  wifiPrimaryPass = primaryPass;
  wifiFallbackSSID = fallbackSSID;
  wifiFallbackPass = fallbackPass;
  wifiHostname = hostname;

  WiFi.setHostname(wifiHostname.c_str());
  startWiFi();
  server.sendHeader("Location", "/");
  server.send(303);
}





void handleSaveMidi() {
  if (!server.hasArg("device") || !server.hasArg("button") || !server.hasArg("val")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }

  int slot = server.arg("device").toInt();
  int button = server.arg("button").toInt();
  int value = server.arg("val").toInt();
  String type = server.hasArg("type") ? server.arg("type") : "note";
  int channel = server.hasArg("channel") ? server.arg("channel").toInt() : 1;

  if (slot < 0 || slot >= 8 || button < 0 || button >= 4) {
    server.send(400, "text/plain", "Invalid slot or button");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave) {
    server.send(400, "text/plain", "No device mapped to this slot");
    return;
  }

  // Save to layer 0 (base layer) for backward compatibility
  slave->config.layerMidiMapping[0][button] = value;
  slave->config.layerMidiType[0][button] = type;
  slave->config.layerMidiChannel[0][button] = channel;

  // Also update legacy fields
  slave->config.midiMapping[button] = value;
  slave->config.midiType[button] = type;
  slave->config.midiChannel[button] = channel;

  saveDeviceConfigWithLayers(deviceMapping[slot]);

  server.send(200, "text/plain", "OK");
}











void handleSavePedalCal() {
  Serial.println("=== PEDAL CAL SAVE REQUEST ===");
  Serial.println("Device: " + server.arg("device"));
  Serial.println("Min: " + server.arg("min"));
  Serial.println("Max: " + server.arg("max"));
  Serial.println("Invert: " + server.arg("invert"));

  if (!server.hasArg("device") || !server.hasArg("min") || !server.hasArg("max")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }

  int slot = server.arg("device").toInt();
  int minVal = server.arg("min").toInt();
  int maxVal = server.arg("max").toInt();
  int invert = server.hasArg("invert") ? server.arg("invert").toInt() : 0;

  if (slot < 0 || slot >= 8) {
    server.send(400, "text/plain", "Invalid slot");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave) {
    server.send(400, "text/plain", "No device mapped to slot");
    return;
  }

  if (!(slave->type >= TYPE_PEDAL_MIN && slave->type <= TYPE_PEDAL_MAX)) {
    server.send(400, "text/plain", "Device is not a pedal");
    return;
  }

  String deviceKey = idToShortKey(slave->uniqueID);
  Serial.println("Device key: " + deviceKey);

  minVal = constrain(minVal, 0, 4095);
  maxVal = constrain(maxVal, 0, 4095);
  invert = (invert != 0) ? 1 : 0;

  // Update runtime config first
  slave->config.pedalCalMin = minVal;
  slave->config.pedalCalMax = maxVal;
  slave->config.pedalInvert = (invert == 1);

  // Save to preferences with error checking
  prefs.begin("devices", false);

  String minKey = deviceKey + "_cal_min";
  String maxKey = deviceKey + "_cal_max";
  String invKey = deviceKey + "_cal_invert";

  Serial.println("Saving keys:");
  Serial.println("  " + minKey + " = " + String(minVal));
  Serial.println("  " + maxKey + " = " + String(maxVal));
  Serial.println("  " + invKey + " = " + String(invert));

  bool minSaved = prefs.putInt(minKey.c_str(), minVal);
  bool maxSaved = prefs.putInt(maxKey.c_str(), maxVal);
  bool invSaved = prefs.putBool(invKey.c_str(), (invert == 1));

  Serial.printf("Save results: min=%s, max=%s, invert=%s\n",
                minSaved ? "OK" : "FAIL",
                maxSaved ? "OK" : "FAIL",
                invSaved ? "OK" : "FAIL");

  prefs.end();

  // Small delay to ensure NVS write completes
  delay(100);

  // Verify save by reading back
  prefs.begin("devices", true);
  int savedMin = prefs.getInt(minKey.c_str(), -999);
  int savedMax = prefs.getInt(maxKey.c_str(), -999);
  bool savedInvert = prefs.getBool(invKey.c_str(), false);
  prefs.end();

  Serial.printf("VERIFICATION: Read back Min=%d, Max=%d, Invert=%d\n", savedMin, savedMax, savedInvert);

  if (savedMin == -999 || savedMax == -999) {
    Serial.println("ERROR: Calibration save failed!");
    server.send(500, "text/plain", "Save failed - storage error");
    return;
  }

  Serial.printf("Saved pedal cal: Device %s, Min=%d, Max=%d, Invert=%d\n", deviceKey.c_str(), minVal, maxVal, invert);
  saveDeviceConfigWithLayers(deviceMapping[slot]);
  server.send(200, "text/plain", "OK");
}









void handleGetPedalCal() {
  if (!server.hasArg("device")) {
    server.send(400, "text/plain", "Missing device parameter");
    return;
  }

  int slot = server.arg("device").toInt();
  if (slot < 0 || slot >= 8) {
    server.send(400, "text/plain", "Invalid slot");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave || !(slave->type >= TYPE_PEDAL_MIN && slave->type <= TYPE_PEDAL_MAX)) {
    server.send(400, "text/plain", "Device is not a pedal");
    return;
  }

  String deviceKey = idToShortKey(slave->uniqueID);

  // Defaults
  int minVal = 0;
  int maxVal = 4095;
  int invert = 0;

  // Load from prefs
  prefs.begin("devices", true);
  if (prefs.isKey((deviceKey + "_cal_min").c_str())) {
    minVal = prefs.getInt((deviceKey + "_cal_min").c_str(), minVal);
    maxVal = prefs.getInt((deviceKey + "_cal_max").c_str(), maxVal);
    invert = prefs.getBool((deviceKey + "_cal_invert").c_str(), invert);
  }
  prefs.end();

  // Update runtime config too, so it matches what we send
  slave->config.pedalCalMin = minVal;
  slave->config.pedalCalMax = maxVal;
  slave->config.pedalInvert = (invert == 1);

  String json = "{\"min\":" + String(minVal) + ",\"max\":" + String(maxVal) + ",\"invert\":" + String(invert) + "}";
  server.send(200, "application/json", json);
}

void handleGetPedalRawData() {
  if (!server.hasArg("device")) {
    server.send(400, "application/json", "{\"error\":\"Missing device\"}");
    return;
  }
  int slot = server.arg("device").toInt();
  if (slot < 0 || slot >= 8) {
    server.send(400, "application/json", "{\"error\":\"Invalid slot\"}");
    return;
  }

  PedalRawData& data = pedalRawReadouts[slot];
  bool isActive = data.lastUpdate > 0 && (millis() - data.lastUpdate) < 500;

  // Check if a live slave is present and recognised as a pedal
  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  bool hasSlave = slave && (slave->type >= TYPE_PEDAL_MIN && slave->type <= TYPE_PEDAL_MAX);

  String json = "{";
  json += "\"raw\":"        + String(data.rawValue)       + ",";
  json += "\"calibrated\":" + String(data.calibratedValue)+ ",";
  json += "\"midi\":"       + String(hasSlave ? slave->lastState : 0) + ",";
  json += "\"active\":"     + String(isActive && hasSlave ? "true" : "false") + ",";
  json += "\"connected\":"  + String(hasSlave ? "true" : "false");
  json += "}";

  server.send(200, "application/json", json);
}

void handleSaveMidiChannelName() {
  if (!server.hasArg("channel") || !server.hasArg("name")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }
  int channel = server.arg("channel").toInt();
  String name = server.arg("name");

  // Save to a simple JSON file for channel names
  DynamicJsonDocument doc(1024);
  File file = SPIFFS.open("/midi_channels.json", "r");
  if (file) {
    deserializeJson(doc, file);
    file.close();
  }

  doc["ch" + String(channel)] = name;

  file = SPIFFS.open("/midi_channels.json", "w");
  if (!file) { server.send(500, "text/plain", "File open failed"); return; }
  serializeJson(doc, file);
  file.close();
  server.send(200, "text/plain", "OK");
}

void handleGetMidiChannelNames() {
  DynamicJsonDocument doc(1024);
  File file = SPIFFS.open("/midi_channels.json", "r");
  if (file) {
    deserializeJson(doc, file);
    file.close();
  }

  String json = "{";
  bool first = true;
  for (int ch = 1; ch <= 16; ch++) {
    String key = "ch" + String(ch);
    if (doc.containsKey(key)) {
      if (!first) json += ",";
      json += "\"" + key + "\":\"" + doc[key].as<String>() + "\"";
      first = false;
    }
  }
  json += "}";

  server.send(200, "application/json", json);
}

void handleSaveSlotAssignment() {
  if (!server.hasArg("deviceId") || !server.hasArg("newSlot")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }

  String deviceIdStr = server.arg("deviceId");
  int newSlot = server.arg("newSlot").toInt();

  if (newSlot < 0 || newSlot >= 8) {
    server.send(400, "text/plain", "Invalid slot");
    return;
  }

  // Find device by ID
  int deviceIndex = -1;
  for (int i = 0; i < numDetected; i++) {
    if (idToShortKey(detectedSlaves[i].uniqueID) == deviceIdStr) {
      deviceIndex = i;
      break;
    }
  }

  if (deviceIndex == -1) {
    server.send(400, "text/plain", "Device not found");
    return;
  }

  // Find current slot of this device
  int currentSlot = -1;
  for (int slot = 0; slot < 8; slot++) {
    if (deviceMapping[slot] == deviceIndex) {
      currentSlot = slot;
      break;
    }
  }

  // Handle slot assignment
  int deviceInTargetSlot = deviceMapping[newSlot];

  if (deviceInTargetSlot >= 0 && deviceInTargetSlot != deviceIndex) {
    // Target slot occupied by different device - swap them
    if (currentSlot >= 0) {
      deviceMapping[currentSlot] = deviceInTargetSlot;
      detectedSlaves[deviceInTargetSlot].config.preferredSlot = currentSlot;
      saveDeviceConfig(deviceInTargetSlot);
    } else {
      // Find an empty slot for the displaced device
      bool foundEmptySlot = false;
      for (int slot = 0; slot < 8; slot++) {
        if (deviceMapping[slot] == -1) {
          deviceMapping[slot] = deviceInTargetSlot;
          detectedSlaves[deviceInTargetSlot].config.preferredSlot = slot;
          saveDeviceConfig(deviceInTargetSlot);
          foundEmptySlot = true;
          break;
        }
      }
      if (!foundEmptySlot) {
        server.send(400, "text/plain", "No available slots for displaced device");
        return;
      }
    }
  }

  // Clear current slot if device was already mapped
  if (currentSlot >= 0) {
    deviceMapping[currentSlot] = -1;
  }

  // Assign device to new slot
  deviceMapping[newSlot] = deviceIndex;
  detectedSlaves[deviceIndex].config.preferredSlot = newSlot;
  saveDeviceConfig(deviceIndex);

  Serial.printf("Device %s moved to slot %d\n", deviceIdStr.c_str(), newSlot);
  server.send(200, "text/plain", "OK");
}

void handleGetSlotMappings() {
  String json = "{\"mappings\":[";
  bool first = true;

  for (int slot = 0; slot < 8; slot++) {
    if (!first) json += ",";
    first = false;

    json += "{\"slot\":" + String(slot) + ",";

    if (deviceMapping[slot] >= 0 && deviceMapping[slot] < numDetected) {
      SlaveInfo& slave = detectedSlaves[deviceMapping[slot]];
      json += "\"deviceId\":\"" + idToShortKey(slave.uniqueID) + "\",";
      json += "\"deviceType\":\"" + typeToName(slave.type) + "\",";
      json += "\"hasButton\":" + String(slave.type == 41 ? "true" : "false") + ",";
      json += "\"i2cAddress\":" + String(slave.i2cAddress);
    } else {
      json += "\"deviceId\":null,";
      json += "\"deviceType\":null,";
      json += "\"i2cAddress\":null";
    }

    json += "}";
  }

  json += "]}";
  server.send(200, "application/json", json);
}

void handleSaveTheme() {
  if (server.hasArg("color")) {
    String newColor = server.arg("color");
    if (newColor.startsWith("#") && newColor.length() == 7) {
      THEME_COLOR = newColor;
      spiffsConfig.saveUIConfig(THEME_COLOR, advancedOptionsEnabled, midiOutputMode);
    }
  }
  server.send(200, "text/plain", "OK");
}

void handleMidiPanic() {
  for (uint8_t ch = 1; ch <= 16; ch++) {
    sendMidiCC(123, 0, ch);
    sendMidiCC(120, 0, ch);
  }
  Serial.println("[MIDI] Panic sent - all notes off");
  server.send(200, "text/plain", "OK");
}

void handleSystemInfo() {
  unsigned long uptimeMs = millis();
  unsigned long seconds = uptimeMs / 1000;
  unsigned long minutes = seconds / 60;
  unsigned long hours = minutes / 60;

  String uptimeStr = String(hours) + "h " + String(minutes % 60) + "m " + String(seconds % 60) + "s";
  String json = "{\"freeHeap\":" + String(ESP.getFreeHeap()) + ",\"uptime\":\"" + uptimeStr + "\"}";

  server.send(200, "application/json", json);
}

void handleFactoryReset() {
  spiffsConfig.factoryReset();
  server.send(200, "text/plain", "Factory reset complete");
  delay(1000);
  ESP.restart();
}

void handleGetDeviceConfigs() {
  String json = "{\"devices\":[";
  bool first = true;

  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (!slave) continue;

    if (!first) json += ",";
    first = false;

    json += "{\"slot\":" + String(slot) + ",";

    // Determine active layer (same logic as MIDI output)
    int activeLayer = getSlotActiveLayer(slot);

    json += "\"midiMapping\":[";
    for (int btn = 0; btn < 4; btn++) {
      json += String(slave->config.layerMidiMapping[activeLayer][btn]);
      if (btn < 3) json += ",";
    }
    json += "],";
    json += "\"midiType\":[";
    for (int btn = 0; btn < 4; btn++) {
      json += "\"" + slave->config.layerMidiType[activeLayer][btn] + "\"";
      if (btn < 3) json += ",";
    }
    json += "],";
    json += "\"midiChannel\":[";
    for (int btn = 0; btn < 4; btn++) {
      json += String(slave->config.layerMidiChannel[activeLayer][btn]);
      if (btn < 3) json += ",";
    }
    json += "],";

    // Add debugging info
    json += "\"activeLayer\":" + String(activeLayer) + ",";
    json += "\"globalLayerEnabled\":" + String(globalLayerControl.enabled ? "true" : "false") + ",";
    json += "\"globalLayer\":" + String(globalLayerControl.currentGlobalLayer) + ",";
    json += "\"deviceLayer\":" + String(shiftStates[slot].currentLayer) + ",";
    json += "\"shiftEnabled\":" + String(slave->config.shiftEnabled ? "true" : "false") + ",";
    json += "\"shiftButton\":" + String(slave->config.shiftButton) + ",";
    json += "\"currentLayer\":" + String(shiftStates[slot].currentLayer) + ",";
    json += "\"ledMode\":[";
    for (int btn = 0; btn < 4; btn++) { json += "\"" + slave->config.ledMode[btn] + "\""; if(btn<3) json += ","; }
    json += "],\"ledColor\":[";
    for (int btn = 0; btn < 4; btn++) { json += "\"" + slave->config.ledColor[btn] + "\""; if(btn<3) json += ","; }
    json += "],\"ledFollowLayer\":[";
    for (int btn = 0; btn < 4; btn++) { json += slave->config.ledFollowLayer[btn]?"true":"false"; if(btn<3) json += ","; }
    json += "],\"dcType\":[";
    for (int btn = 0; btn < 4; btn++) { json += "\"" + slave->config.deviceControlType[btn] + "\""; if(btn<3) json += ","; }
    json += "],\"dcTarget\":[";
    for (int btn = 0; btn < 4; btn++) { json += String(slave->config.deviceControlTarget[btn]); if(btn<3) json += ","; }
    json += "]";
    json += "}";
  }

  json += "],";
  json += "\"globalLayerControl\":{";
  json += "\"enabled\":" + String(globalLayerControl.enabled ? "true" : "false") + ",";
  json += "\"currentLayer\":" + String(globalLayerControl.currentGlobalLayer);
  json += "}}";
  server.send(200, "application/json", json);
}










// Web UI Handler with Enhanced Slot-Based Design
void handleRoot() {
  prefs.begin("wifi", true);
  String savedPrimaryPass = prefs.getString("pass_primary", "");
  prefs.end();

  String ipStr = (wifiConnected && WiFi.status() == WL_CONNECTED)
                   ? WiFi.localIP().toString()
                   : (apModeActive ? "AP Mode" : "Not connected");
  String status = (wifiState == WIFI_PRIMARY)  ? "Primary"
                : (wifiState == WIFI_FALLBACK) ? "Fallback"
                : (wifiState == WIFI_AP_MODE)  ? "AP Mode" : "Connected";

  String lighterColor    = getLighterColor(THEME_COLOR);
  String lightBackground = getLightBackground(THEME_COLOR);
  String safeSSID = wifiPrimarySSID;
  safeSSID.replace("\\", "\\\\");
  safeSSID.replace("'", "\\'");

  String midiChannelOptions = "";
  prefs.begin("midi_names", true);
  for (int ch = 1; ch <= 16; ch++) {
    String name = prefs.getString(("ch" + String(ch)).c_str(), "");
    String dn = (name.length() > 0) ? name : ("Channel " + String(ch));
    midiChannelOptions += "<option value='" + String(ch) + "'>" + String(ch) + ": " + dn + "</option>";
  }
  prefs.end();

  String midiChannelsConfig = "";
  prefs.begin("midi_names", true);
  for (int ch = 1; ch <= 16; ch++) {
    String name = prefs.getString(("ch" + String(ch)).c_str(), "");
    name.replace("'", "&#39;");
    midiChannelsConfig += "<div class='input-group'><label class='input-label'>Channel " + String(ch) + "</label>";
    midiChannelsConfig += "<input type='text' id='ch" + String(ch) + "Name' class='text-field' value='" + name + "' onchange='saveMidiChannelName(" + String(ch) + ")' placeholder='Channel " + String(ch) + "'></div>";
  }
  prefs.end();

  // Requires PSRAM: Tools → PSRAM → OPI PSRAM
  String html = String(HTML_TEMPLATE);
  if (html.length() == 0) {
    server.send(503, "text/plain", "Out of memory. Enable PSRAM: Tools → PSRAM → OPI PSRAM");
    return;
  }

  html.replace("%THEME_COLOR%",          THEME_COLOR);
  html.replace("%LIGHTER_COLOR%",        lighterColor);
  html.replace("%LIGHT_BACKGROUND%",     lightBackground);
  html.replace("%THEME_COLOR_RGBA%",     THEME_COLOR + "40");
  html.replace("%THEME_COLOR_RGBA_LIGHT%", THEME_COLOR + "20");
  html.replace("%WIFI_PRIMARY_SSID%",    safeSSID);
  html.replace("%SAVED_PRIMARY_PASS%",   savedPrimaryPass);
  html.replace("%WIFI_FALLBACK_SSID%",   wifiFallbackSSID);
  html.replace("%WIFI_FALLBACK_PASS%",   wifiFallbackPass);
  html.replace("%WIFI_HOSTNAME%",        wifiHostname);
  html.replace("%IP_ADDRESS%",           ipStr);
  html.replace("%STATUS%",               status);
  html.replace("%MIDI_CHANNELS%",        midiChannelsConfig);
  html.replace("%MIDI_CHANNEL_OPTIONS%", midiChannelOptions);
  html.replace("%BUILD_DATE%",           __DATE__ " " __TIME__);
  html.replace("%ADVANCED_CHECKED%",     advancedOptionsEnabled ? " checked" : "");
  html.replace("%ADVANCED_DISPLAY%",     advancedOptionsEnabled ? "block" : "none");
  html.replace("%GREEN_SELECTED%",       (THEME_COLOR == "#4CAF50") ? " selected" : "");
  html.replace("%BLUE_SELECTED%",        (THEME_COLOR == "#2196F3") ? " selected" : "");
  html.replace("%ORANGE_SELECTED%",      (THEME_COLOR == "#FF5722") ? " selected" : "");
  html.replace("%RED_SELECTED%",         (THEME_COLOR == "#F44336") ? " selected" : "");
  html.replace("%PURPLE_SELECTED%",      (THEME_COLOR == "#9C27B0") ? " selected" : "");
  html.replace("%BLUE_GREY_SELECTED%",   (THEME_COLOR == "#607D8B") ? " selected" : "");
  html.replace("%AMBER_SELECTED%",       (THEME_COLOR == "#f5b227") ? " selected" : "");

  server.send(200, "text/html; charset=utf-8", html);
}




// OLED Display support
void initDisplay() {
  // Initialize secondary I2C bus for display
  lcd.begin(4, 5, 400000);  // SDA=4, SCL=5, 400kHz

  // Initialize display using the secondary I2C bus
  if (!display.begin(SSD1306_SWITCHCAPVCC, OLED_ADDR)) {
    Serial.println("SSD1306 allocation failed");
    return;
  }

  display.clearDisplay();
  display.setTextSize(1);
  display.setTextColor(SSD1306_WHITE);
  display.setCursor(0, 0);
  display.println("...INITIALISING...");
  display.display();
  Serial.println("Display initialized on I2C1 (pins 4,5)");
}

void showWiFiInfo() {
  display.clearDisplay();
  display.setTextSize(1);
  display.setCursor(0, 0);
  display.println("SSID: ");
  display.println(WiFi.SSID());
  display.println();
  display.println("IP: ");
  display.println(WiFi.localIP());
  display.println();
  display.println("Devices: ");
  display.println(numDetected);
  display.display();
  display.println();
}

void updateDisplayWithShiftInfo() {
  display.clearDisplay();
  display.setTextSize(1);
  display.setCursor(0, 0);

  if (wifiConnected) {
    display.print("IP: ");
    display.println(WiFi.localIP());
  } else {
    display.println("WiFi: Connecting...");
  }

  display.println("Devices: " + String(numDetected));

  // Show active shift layers
  bool hasActiveShifts = false;
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (slave && slave->config.shiftEnabled && shiftStates[slot].currentLayer > 0) {
      if (!hasActiveShifts) {
        display.println("Active Shifts:");
        hasActiveShifts = true;
      }
      display.printf("S%d:L%d ", slot + 1, shiftStates[slot].currentLayer + 1);
    }
  }

  display.display();
}

void showGlobalLayerInfo() {
  display.clearDisplay();

  // Show large global layer number in center
  if (globalLayerControl.enabled) {
    display.setTextSize(4);
    // Center the number horizontally (assuming single digit)
    int x = (SCREEN_WIDTH - 24) / 2;   // 24 pixels wide for size 4 text
    int y = (SCREEN_HEIGHT - 32) / 2;  // 32 pixels tall for size 4 text
    display.setCursor(x, y);
    display.println(globalLayerControl.currentGlobalLayer + 1);

    // Label above the number
    //display.setTextSize(1);
    //display.setCursor((SCREEN_WIDTH - 72) / 2, y - 12); // "Global Layer" is ~72 pixels
    //display.println("Global Layer");
  } else {
    display.setTextSize(4);
    // Center the number horizontally (assuming single digit)
    int x = (SCREEN_WIDTH - 24) / 2;   // 24 pixels wide for size 4 text
    int y = (SCREEN_HEIGHT - 32) / 2;  // 32 pixels tall for size 4 text
    display.setCursor(x, y);
    display.println("1");

    // Label above the number
    //display.setTextSize(1);
    //display.setCursor((SCREEN_WIDTH - 72) / 2, y - 12); // "Global Layer" is ~72 pixels
    //display.println("Global Layer");
  }

  // Show individual device layers at bottom
  display.setTextSize(1);
  display.setCursor(0, SCREEN_HEIGHT - 8);  // Bottom line

  bool hasActiveLayers = false;
  String deviceLayers = "";

  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (slave) {
      int layer = shiftStates[slot].currentLayer + 1;
      deviceLayers += String(layer);
      if (slot < 7) deviceLayers += " ";
      hasActiveLayers = true;
    } else {
      deviceLayers += "- ";
    }
  }

  if (hasActiveLayers) {
    //display.print("D: ");
    display.print(deviceLayers);
  } else {
    display.print("No devices");
  }

  display.display();
}


void handleSaveAdvancedToggle() {
  if (server.hasArg("enabled")) {
    advancedOptionsEnabled = server.arg("enabled") == "true";
    spiffsConfig.saveUIConfig(THEME_COLOR, advancedOptionsEnabled, midiOutputMode);
  }
  server.send(200, "text/plain", "OK");
}

void handleSaveI2CSpeed() {
  if (server.hasArg("speed")) {
    int speed = server.arg("speed").toInt();
    prefs.begin("hardware", false);
    prefs.putInt("i2c_speed", speed);
    prefs.end();
    server.send(200, "text/plain", "OK");
    delay(1000);
    ESP.restart();  // Restart to apply I2C speed change
  } else {
    server.send(400, "text/plain", "Missing speed parameter");
  }
}

void handleSaveMidiPolling() {
  if (server.hasArg("rate")) {
    int rate = server.arg("rate").toInt();
    prefs.begin("hardware", false);
    prefs.putInt("midi_polling", rate);
    prefs.end();
    server.send(200, "text/plain", "OK");
  } else {
    server.send(400, "text/plain", "Missing rate parameter");
  }
}

void handleToggleDebug() {
  if (server.hasArg("enabled")) {
    bool enabled = server.arg("enabled") == "true";
    prefs.begin("debug", false);
    prefs.putBool("debug_enabled", enabled);
    prefs.end();
    server.send(200, "text/plain", "OK");
  } else {
    server.send(400, "text/plain", "Missing enabled parameter");
  }
}

void handleGetLCDContent() {
  String json = "{";

  String line1 = "";
  String line2 = "";
  String line3 = "";
  String line4 = "";

  // Show WiFi info until any button is pressed
  if (!anyButtonPressed) {
    if (wifiConnected) {
      line1 = "SSID: " + WiFi.SSID();
      line2 = "";
      line3 = "IP: " + WiFi.localIP().toString();
      line4 = "Devices: " + String(numDetected);
    } else {
      line1 = "WiFi: Connecting...";
      line2 = "Devices: " + String(numDetected);
      line3 = "";
      line4 = "";
    }
  } else {
    // After button press, show global layer info
    line1 = "";

    if (globalLayerControl.enabled) {
      line2 = "       " + String(globalLayerControl.currentGlobalLayer + 1);
    } else {
      line2 = "       1";
    }

    line3 = "";

    // Device layers at bottom
    String deviceLayers = "";
    bool hasActiveLayers = false;

    for (int slot = 0; slot < 8; slot++) {
      SlaveInfo* slave = getSlaveInfoForSlot(slot);
      if (slave) {
        int layer = shiftStates[slot].currentLayer + 1;
        deviceLayers += String(layer);
        if (slot < 7) deviceLayers += " ";
        hasActiveLayers = true;
      } else {
        deviceLayers += "- ";
      }
    }

    if (hasActiveLayers) {
      line4 = deviceLayers;
    } else {
      line4 = "No devices";
    }
  }

  json += "\"line1\":\"" + line1 + "\",";
  json += "\"line2\":\"" + line2 + "\",";
  json += "\"line3\":\"" + line3 + "\",";
  json += "\"line4\":\"" + line4 + "\"";
  json += "}";

  server.send(200, "application/json", json);
}


void testSimpleDeviceControlSave() {
  Serial.println("=== SIMPLE DEVICE CONTROL TEST ===");

  if (prefs.begin("devices", false)) {
    Serial.println("Opened devices preferences for writing");

    // Test saving a simple device control setting
    size_t written1 = prefs.putString("test_dc_type", "layer_global");
    size_t written2 = prefs.putInt("test_dc_target", 2);
    size_t written3 = prefs.putBool("test_dc_momentary", false);

    Serial.printf("Write results: type=%d bytes, target=%d bytes, momentary=%d bytes\n",
                  written1, written2, written3);

    prefs.end();

    // Now try to read them back
    if (prefs.begin("devices", true)) {
      String type = prefs.getString("test_dc_type", "MISSING");
      int target = prefs.getInt("test_dc_target", -1);
      bool momentary = prefs.getBool("test_dc_momentary", true);

      Serial.printf("Read results: type='%s', target=%d, momentary=%s\n",
                    type.c_str(), target, momentary ? "Y" : "N");

      // Clean up test data
      prefs.remove("test_dc_type");
      prefs.remove("test_dc_target");
      prefs.remove("test_dc_momentary");
      prefs.end();
    }
  } else {
    Serial.println("FAILED to open devices preferences");
  }

  Serial.println("=== END SIMPLE TEST ===");
}
void initLayerLED() {
  FastLED.addLeds<WS2812, LED_PIN, GRB>(leds, NUM_LEDS);
  FastLED.setBrightness(globalLEDBrightness);

  // Start with Layer 1 color (red)
  updateLayerLED(0);

  Serial.println("Layer LED initialized on pin " + String(LED_PIN));
}

// Add this function to update LED color based on active layer
void updateLayerLED(int activeLayer) {
  // Ensure layer is within bounds
  activeLayer = constrain(activeLayer, 0, 3);

  // Update master LED
  leds[0] = getCustomLayerColor(activeLayer);
  FastLED.show();

  // Update all button device LEDs
  updateAllDeviceLayerLEDs(activeLayer);

  Serial.printf("Layer LED updated to Layer %d with custom color\n", activeLayer + 1);
  webSerial.printf("Layer LED updated to Layer %d with custom color\n", activeLayer + 1);
}






void handleGetSerialLog() {
  String log = webSerial.getBuffer();
  String encodedLog = base64::encode(log);
  
  String json = "{\"log\":\"" + encodedLog + "\",\"encoded\":true}";
  server.send(200, "application/json", json);
}

void handleClearSerialLog() {
  webSerial.clear();
  server.send(200, "text/plain", "OK");
}

void handleToggleWebSerial() {
  if (server.hasArg("enabled")) {
    webSerialEnabled = server.arg("enabled") == "true";
  }
  server.send(200, "text/plain", "OK");
}


void handleSaveLayerColor() {
  if (!server.hasArg("layer") || !server.hasArg("color")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }
  
  int layer = server.arg("layer").toInt();
  String color = server.arg("color");
  
  if (layer < 0 || layer >= 4 || !color.startsWith("#") || color.length() != 7) {
    server.send(400, "text/plain", "Invalid parameters");
    return;
  }
  
  DynamicJsonDocument doc(512);
  File file = SPIFFS.open("/layer_colors.json", "r");
  if (file) {
    deserializeJson(doc, file);
    file.close();
  }
  
  doc["layer" + String(layer)] = color;
  
  file = SPIFFS.open("/layer_colors.json", "w");
  if (!file) { server.send(500, "text/plain", "File open failed"); return; }
  serializeJson(doc, file);
  file.close();
  invalidateLayerColorCache();
  
  Serial.printf("Layer %d color saved: %s\n", layer, color.c_str());
  
  // Update LED immediately if this is the currently active layer
  int currentActiveLayer = getCurrentActiveLayer();
  if (currentActiveLayer == layer) {
    updateLayerLED(currentActiveLayer);
    Serial.println("LED updated with new custom color");
  }
  
  server.send(200, "text/plain", "OK");
}

void handleGetThemeSettings() {
  DynamicJsonDocument doc(1024);
  
  doc["mainColor"] = THEME_COLOR;
  
  JsonArray layerColors = doc.createNestedArray("layerColors");
  
  File file = SPIFFS.open("/layer_colors.json", "r");
  if (file) {
    DynamicJsonDocument layerDoc(512);
    deserializeJson(layerDoc, file);
    file.close();
    
    String defaultColors[] = {"#F44336", "#9C27B0", "#2196F3", "#4CAF50"};
    for (int i = 0; i < 4; i++) {
      String key = "layer" + String(i);
      if (layerDoc.containsKey(key)) {
        layerColors.add(layerDoc[key].as<String>());
      } else {
        layerColors.add(defaultColors[i]);
      }
    }
  } else {
    layerColors.add("#F44336");
    layerColors.add("#9C27B0");
    layerColors.add("#2196F3");
    layerColors.add("#4CAF50");
  }
  
  String json;
  serializeJson(doc, json);
  server.send(200, "application/json", json);
}

void handleRefreshLED() {
  int currentLayer = getCurrentActiveLayer();
  updateLayerLED(currentLayer);
  server.send(200, "text/plain", "OK");
}


// Function to handle I2C encoder devices (3-pin rotary + integrated button)
// Function to handle I2C encoder devices (3-pin rotary + integrated button)
void pollI2CEncoders() {
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (!slave || !(slave->type >= TYPE_ENCODER_MIN && slave->type <= TYPE_ENCODER_MAX)) continue;
    
    if (millis() - slave->lastFailTime < 50) continue;
    
    // Request encoder data: [position_low, position_high, button_state]
    Wire.beginTransmission(slave->i2cAddress);
    Wire.write(0x04); // Encoder data command
    uint8_t err = Wire.endTransmission();
    if (err != 0) {
      slave->lastFailTime = millis();
      Serial.printf("[I2C ENCODER] Slot %d communication error: %d\n", slot, err);
      webSerial.printf("[I2C ENCODER] Slot %d communication error: %d\n", slot, err);
      continue;
    }
    
    Wire.requestFrom(slave->i2cAddress, (uint8_t)3);
    if (Wire.available() < 3) {
      Serial.printf("[I2C ENCODER] Slot %d insufficient data (got %d bytes, expected 3)\n", slot, Wire.available());
      webSerial.printf("[I2C ENCODER] Slot %d insufficient data (got %d bytes, expected 3)\n", slot, Wire.available());
      continue;
    }
    
    // Read 16-bit encoder position
    uint8_t posLow = Wire.read();
    uint8_t posHigh = Wire.read();
    int16_t currentPosition = (posHigh << 8) | posLow;
    
    uint8_t buttonState = Wire.read();
    bool buttonPressed = (buttonState & 0x01);
    bool buttonChanged = ((buttonState & 0x02) != 0);
    
    I2CEncoderState& state = encoderStates[slot];
    
    // Log raw data from encoder slave
    Serial.printf("[ENCODER RAW] Slot %d | Addr:0x%02X | PosH:0x%02X PosL:0x%02X | Pos16:%d | BtnState:0x%02X | BtnPressed:%s | BtnChanged:%s\n",
                  slot, slave->i2cAddress, posHigh, posLow, currentPosition, 
                  buttonState, buttonPressed ? "Y" : "N", buttonChanged ? "Y" : "N");
    
    webSerial.printf("[ENCODER RAW] Slot %d | Addr:0x%02X | Pos:%d | Btn:%s%s\n",
                    slot, slave->i2cAddress, currentPosition, 
                    buttonPressed ? "PRESSED" : "RELEASED", buttonChanged ? " (CHANGED)" : "");
    
    // Handle rotation (check for position change)
    if (currentPosition != state.lastPosition) {
  int delta = currentPosition - state.lastPosition;
  
  // Handle overflow/underflow for 16-bit signed values
  if (delta > 32767) delta -= 65536;
  if (delta < -32768) delta += 65536;
  
  Serial.printf("[ENCODER SIMPLE] Slot %d | Delta:%d | Processing immediately\n", slot, delta);
  webSerial.printf("[ENCODER SIMPLE] Slot %d | Delta:%d\n", slot, delta);
  
  // Process every single step change immediately
  if (delta != 0) {
    handleI2CEncoderRotation(slot, delta > 0 ? 1 : -1, slave->config);
    oscOnEncoder(slot, delta > 0 ? 1 : -1);
    pqOnEncoder(slot, delta > 0 ? 1 : -1);
  }
  
  state.lastPosition = currentPosition;
}
    
    // Handle button press/release
    if (buttonChanged) {
      Serial.printf("[ENCODER BUTTON] Slot %d | State changed to: %s | Mode:%s\n",
                    slot, buttonPressed ? "PRESSED" : "RELEASED", slave->config.encoderButtonMode.c_str());
      
      webSerial.printf("[ENCODER BUTTON] Slot %d | %s | Mode:%s\n",
                      slot, buttonPressed ? "PRESSED" : "RELEASED", slave->config.encoderButtonMode.c_str());
      
      handleI2CEncoderButton(slot, buttonPressed, slave->config);
      oscOnEncoderButton(slot, buttonPressed);
      pqOnEncoderButton(slot, buttonPressed);
      state.lastButtonState = buttonPressed;
    }
    
    state.lastUpdateTime = millis();
    
    // Periodic status update (every 5 seconds when encoder is active)
    static unsigned long lastStatusLog = 0;
    if (millis() - lastStatusLog > 5000) {
      Serial.printf("[ENCODER STATUS] Slot %d | Pos:%d | Button:%s\n",
                    slot, currentPosition, buttonPressed ? "DOWN" : "UP");
      lastStatusLog = millis();
    }
  }
}

// Handle I2C encoder rotation (both device and global layer options)
// Handle I2C encoder rotation (device, global layer, MIDI CC, and MIDI note options)
void handleI2CEncoderRotation(int slot, int detents, DeviceConfig& config) {
  bool clockwise = detents > 0;
  if (config.encoderInvertDirection) clockwise = !clockwise;
  
  int absDetents = abs(detents);
  
  if (config.encoderRotationMode == "layer_global") {
    // Control global layers
    globalLayerControl.enabled = true;
    int currentLayer = globalLayerControl.currentGlobalLayer;
    int newLayer = currentLayer;
    
    for (int i = 0; i < absDetents; i++) {
      if (clockwise) {
        newLayer = (newLayer + 1) % configNumLayers;
      } else {
        newLayer = (newLayer - 1 + configNumLayers) % configNumLayers;
      }
    }
    
    if (newLayer != currentLayer) {
      globalLayerControl.currentGlobalLayer = newLayer;
      updateLayerLED(globalLayerControl.currentGlobalLayer);
      Serial.printf("[I2C ENCODER] Slot %d Global layer: %d -> %d (%d detents)\n", 
                    slot, currentLayer, newLayer, detents);
      webSerial.printf("[I2C ENCODER] Slot %d Global layer: %d -> %d (%d detents)\n", 
                      slot, currentLayer, newLayer, detents);
      spiffsConfig.saveGlobalConfig(globalLayerControl.enabled, globalLayerControl.currentGlobalLayer);
    }
  } 
  else if (config.encoderRotationMode == "layer_device") {
    // Control device-specific layer
    int currentLayer = shiftStates[slot].currentLayer;
    int newLayer = currentLayer;
    
    for (int i = 0; i < absDetents; i++) {
      if (clockwise) {
        newLayer = (newLayer + 1) % configNumLayers;
      } else {
        newLayer = (newLayer - 1 + configNumLayers) % configNumLayers;
      }
    }
    
    if (newLayer != currentLayer) {
      shiftStates[slot].currentLayer = newLayer;
      updateLayerLED(getCurrentActiveLayer());
      Serial.printf("[I2C ENCODER] Slot %d Device layer: %d -> %d (%d detents)\n", 
                    slot, currentLayer, newLayer, detents);
      webSerial.printf("[I2C ENCODER] Slot %d Device layer: %d -> %d (%d detents)\n", 
                      slot, currentLayer, newLayer, detents);
    }
  }
  else if (config.encoderRotationMode == "midi_cc") {
    // MIDI CC control
    static int encoderValues[8] = {64, 64, 64, 64, 64, 64, 64, 64}; // Default to middle values
    
    int currentValue = encoderValues[slot];
    int newValue = currentValue;
    
    // Apply rotation steps
    for (int i = 0; i < absDetents; i++) {
      if (clockwise) {
        newValue = min(config.encoderMaxValue, newValue + 1);
      } else {
        newValue = max(config.encoderMinValue, newValue - 1);
      }
    }
    
    if (newValue != currentValue) {
      encoderValues[slot] = newValue;
      sendMidiCC(config.encoderMidiCC, newValue, config.encoderMidiChannel);
      
      Serial.printf("[I2C ENCODER] Slot %d CC %d: %d -> %d (Ch %d)\n", 
                    slot, config.encoderMidiCC, currentValue, newValue, config.encoderMidiChannel);
      webSerial.printf("[I2C ENCODER] Slot %d CC %d: %d -> %d (Ch %d)\n", 
                      slot, config.encoderMidiCC, currentValue, newValue, config.encoderMidiChannel);
    }
  }
  else if (config.encoderRotationMode == "osc") {
    static int encOscPos[8] = {64, 64, 64, 64, 64, 64, 64, 64};
    if (config.encoderOscMode == "absolute") {
      encOscPos[slot] = constrain(encOscPos[slot] + (clockwise ? absDetents : -absDetents), 0, 127);
      oscSendAction(config.encoderOscAddr, "f", encOscPos[slot] / 127.0f);
    } else {
      oscSendAction(config.encoderOscAddr, "i", clockwise ? absDetents : -absDetents);
    }
  }
  else if (config.encoderRotationMode == "midi_note") {
    // MIDI Note mode - send note on/off for each detent click
    // Note on/off pairs are transmitted in order on the wire (UART and USB
    // both serialize messages), so no blocking delays are needed here.
    // The old delay(5)/delay(10) calls stalled polling, the web server,
    // and all other MIDI I/O during fast encoder spins.
    for (int i = 0; i < absDetents; i++) {
      sendMidiNote(config.encoderNoteNumber, true, config.encoderNoteChannel, 127);
      sendMidiNote(config.encoderNoteNumber, false, config.encoderNoteChannel, 0);
    }

    // Log the overall rotation
    Serial.printf("[I2C ENCODER] Slot %d rotated %s, triggered %d note events\n", 
                  slot, clockwise ? "clockwise" : "counter-clockwise", absDetents);
    webSerial.printf("[I2C ENCODER] Slot %d rotated %s, triggered %d note events\n", 
                    slot, clockwise ? "clockwise" : "counter-clockwise", absDetents);
  }
}

// Handle I2C encoder button (integrated button)
void handleI2CEncoderButton(int slot, bool pressed, DeviceConfig& config) {
  if (config.encoderButtonMode == "layer_toggle" && pressed) {
    // Toggle between global and device layer modes
    globalLayerControl.enabled = !globalLayerControl.enabled;
    
    Serial.printf("[I2C ENCODER] Slot %d Layer mode: %s\n", 
                  slot, globalLayerControl.enabled ? "Global" : "Device");
    webSerial.printf("[I2C ENCODER] Slot %d Layer mode: %s\n", 
                    slot, globalLayerControl.enabled ? "Global" : "Device");
    
    spiffsConfig.saveGlobalConfig(globalLayerControl.enabled, globalLayerControl.currentGlobalLayer);
    updateLayerLED(getCurrentActiveLayer());
  } 
  else if (config.encoderButtonMode == "midi_momentary") {
    sendMidiNote(config.encoderButtonMidiNote, pressed, config.encoderButtonMidiChannel);
    
    Serial.printf("[I2C ENCODER] Slot %d Button MIDI Note %d: %s (Ch %d)\n", 
                  slot, config.encoderButtonMidiNote, pressed ? "ON" : "OFF", config.encoderButtonMidiChannel);
    webSerial.printf("[I2C ENCODER] Slot %d Button MIDI Note %d: %s (Ch %d)\n", 
                    slot, config.encoderButtonMidiNote, pressed ? "ON" : "OFF", config.encoderButtonMidiChannel);
  } 
  else if (config.encoderButtonMode == "osc") {
    oscSendAction(config.encoderButtonOscAddr, "i", pressed ? 1 : 0);
  }
  else if (config.encoderButtonMode == "midi_latching" && pressed) {
    static bool latchStates[8] = {false, false, false, false, false, false, false, false};
    latchStates[slot] = !latchStates[slot];
    
    sendMidiNote(config.encoderButtonMidiNote, latchStates[slot], config.encoderButtonMidiChannel);
    
    Serial.printf("[I2C ENCODER] Slot %d Button Latch Note %d: %s (Ch %d)\n", 
                  slot, config.encoderButtonMidiNote, latchStates[slot] ? "ON" : "OFF", config.encoderButtonMidiChannel);
    webSerial.printf("[I2C ENCODER] Slot %d Button Latch Note %d: %s (Ch %d)\n", 
                    slot, config.encoderButtonMidiNote, latchStates[slot] ? "ON" : "OFF", config.encoderButtonMidiChannel);
  }
}

// Initialize encoder states
void initializeEncoderStates() {
  for (int i = 0; i < 8; i++) {
    encoderStates[i] = {0, 0, false, false, 0};
  }
}





void handleGetEncoderConfig() {
  if (!server.hasArg("device")) {
    server.send(400, "text/plain", "Missing device parameter");
    return;
  }

  int slot = server.arg("device").toInt();
  if (slot < 0 || slot >= 8) {
    server.send(400, "text/plain", "Invalid slot");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave || !(slave->type >= TYPE_ENCODER_MIN && slave->type <= TYPE_ENCODER_MAX)) {
    server.send(400, "text/plain", "Not an encoder device");
    return;
  }

  String json = "{";
  json += "\"rotationMode\":\"" + slave->config.encoderRotationMode + "\",";
  json += "\"midiCC\":" + String(slave->config.encoderMidiCC) + ",";
  json += "\"midiChannel\":" + String(slave->config.encoderMidiChannel) + ",";
  json += "\"minValue\":" + String(slave->config.encoderMinValue) + ",";
  json += "\"maxValue\":" + String(slave->config.encoderMaxValue) + ",";
  json += "\"invertDirection\":" + String(slave->config.encoderInvertDirection ? "true" : "false") + ",";
  json += "\"buttonMode\":\"" + slave->config.encoderButtonMode + "\",";
  json += "\"buttonMidiNote\":" + String(slave->config.encoderButtonMidiNote) + ",";
  json += "\"buttonMidiChannel\":" + String(slave->config.encoderButtonMidiChannel) + ",";
  json += "\"stepsPerDetent\":" + String(slave->config.encoderStepsPerDetent) + ",";
  json += "\"noteNumber\":" + String(slave->config.encoderNoteNumber) + ",";
  json += "\"noteChannel\":" + String(slave->config.encoderNoteChannel) + ",";
  json += "\"oscAddr\":\"" + jsonEsc(slave->config.encoderOscAddr) + "\",";
  json += "\"oscMode\":\"" + slave->config.encoderOscMode + "\",";
  json += "\"buttonOscAddr\":\"" + jsonEsc(slave->config.encoderButtonOscAddr) + "\"";
  json += "}";

  server.send(200, "application/json", json);
}

void handleSaveEncoderConfig() {
  if (server.method() != HTTP_POST) {
    server.send(405, "text/plain", "Method not allowed");
    return;
  }

  String body = server.arg("plain");
  if (body.length() == 0) {
    server.send(400, "text/plain", "No JSON body");
    return;
  }

  // Parse with ArduinoJson. Any field missing from the payload falls back
  // to the device's current value instead of being reset to a default.
  DynamicJsonDocument doc(1024);
  DeserializationError jsonErr = deserializeJson(doc, body);
  if (jsonErr) {
    server.send(400, "text/plain", String("JSON parse error: ") + jsonErr.c_str());
    return;
  }

  int device = doc["device"] | -1;
  if (device < 0 || device >= 8) {
    server.send(400, "text/plain", "Invalid device");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(device);
  if (!slave || !(slave->type >= TYPE_ENCODER_MIN && slave->type <= TYPE_ENCODER_MAX)) {
    server.send(400, "text/plain", "Not an encoder device");
    return;
  }

  DeviceConfig& cfg = slave->config;
  cfg.encoderRotationMode      = doc["rotationMode"]    | cfg.encoderRotationMode;
  cfg.encoderMidiCC            = doc["midiCC"]          | cfg.encoderMidiCC;
  cfg.encoderMidiChannel       = doc["midiChannel"]     | cfg.encoderMidiChannel;
  cfg.encoderMinValue          = doc["minValue"]        | cfg.encoderMinValue;
  cfg.encoderMaxValue          = doc["maxValue"]        | cfg.encoderMaxValue;
  cfg.encoderNoteNumber        = doc["noteNumber"]      | cfg.encoderNoteNumber;
  cfg.encoderNoteChannel       = doc["noteChannel"]     | cfg.encoderNoteChannel;
  cfg.encoderInvertDirection   = doc["invertDirection"] | cfg.encoderInvertDirection;
  cfg.encoderStepsPerDetent    = doc["stepsPerDetent"]  | cfg.encoderStepsPerDetent;
  // Integrated encoder button (sent by the UI as buttonNumber/buttonChannel)
  cfg.encoderButtonMidiNote    = doc["buttonNumber"]    | cfg.encoderButtonMidiNote;
  cfg.encoderButtonMidiChannel = doc["buttonChannel"]   | cfg.encoderButtonMidiChannel;
  cfg.encoderButtonMode        = doc["buttonMode"]      | cfg.encoderButtonMode;
  cfg.encoderOscAddr           = doc["oscAddr"]         | cfg.encoderOscAddr;
  cfg.encoderOscMode           = doc["oscMode"]         | cfg.encoderOscMode;
  cfg.encoderButtonOscAddr     = doc["buttonOscAddr"]   | cfg.encoderButtonOscAddr;

  // Save configuration
  saveDeviceConfigWithLayers(deviceMapping[device]);

  server.send(200, "text/plain", "OK");
}

// Send layer color to a specific button device's LED 0
void sendLayerColorToDevice(uint8_t i2cAddress, CRGB color) {
  Wire.beginTransmission(i2cAddress);
  Wire.write(0x22);  // Command: Set layer color
  CRGB c = scaleForBrightness(color);
  Wire.write(c.r);
  Wire.write(c.g);
  Wire.write(c.b);
  uint8_t error = Wire.endTransmission();
  
  if (error == 0) {
    Serial.printf("[LED] Sent layer color to 0x%02X: R=%d G=%d B=%d\n", 
                  i2cAddress, color.r, color.g, color.b);
    webSerial.printf("[LED] Sent layer color to 0x%02X: R=%d G=%d B=%d\n", 
                     i2cAddress, color.r, color.g, color.b);
  } else {
    Serial.printf("[LED] Failed to send layer color to 0x%02X (error %d)\n", 
                  i2cAddress, error);
    webSerial.printf("[LED] Failed to send layer color to 0x%02X (error %d)\n", 
                     i2cAddress, error);
  }
}



void updateDeviceLED(int slot, int button, bool outputState) {
  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave) return;
  bool isPedalBtn = (slave->type == 41);
  if (!((slave->type >= TYPE_FOOTSWITCH_MIN && slave->type <= TYPE_FOOTSWITCH_MAX) ||
        (slave->type >= TYPE_BUTTON_MIN && slave->type <= TYPE_BUTTON_MAX) || isPedalBtn)) return;

  lastButtonLEDUpdate[slot][button] = millis();
  
  CRGB color = CRGB::Black;
  String mode = slave->config.ledMode[button];
  
  if (mode == "off") { color = CRGB::Black; }
  else if (mode == "button_state" || mode == "midi_out" || mode == "midi_out_triggered" || mode == "midi_in" || mode == "osc_in") {
    if (outputState) {
      color = slave->config.ledFollowLayer[button]
        ? getCustomLayerColor(getCurrentActiveLayer())
        : hexToRGB(slave->config.ledColor[button]);
    }
  }
  else if (mode == "layer") { color = getCustomLayerColor(getCurrentActiveLayer()); }

  buttonLEDDesiredState[slot][button] = (color != CRGB::Black);
  if (ledOverrideOn[slot][button]) color = ledOverrideColor[slot][button];
  setDeviceLED(slave->i2cAddress, isPedalBtn ? 1 : button + 1, color);
}



void updateAllDeviceLayerLEDs(int layerIndex) {
  CRGB layerColor = getCustomLayerColor(layerIndex);
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (!slave) continue;
    if (!(slave->type >= TYPE_FOOTSWITCH_MIN && slave->type <= TYPE_FOOTSWITCH_MAX)) continue;
    sendLayerColorToDevice(slave->i2cAddress, layerColor);
    delay(2);
    for (int btn = 0; btn < 4; btn++) {
      if (buttonLEDDesiredState[slot][btn]) {
        String mode = slave->config.ledMode[btn];
        CRGB color = (mode == "layer") ? layerColor
          : slave->config.ledFollowLayer[btn] ? layerColor
          : hexToRGB(slave->config.ledColor[btn]);
        setDeviceLED(slave->i2cAddress, btn + 1, color);
        delay(1);
      }
    }
  }
}

// Set a specific LED on a device (for MIDI feedback mode - future use)
void setDeviceLED(uint8_t i2cAddress, uint8_t ledIndex, CRGB color) {
  CRGB c = scaleForBrightness(color);
  Wire.beginTransmission(i2cAddress);
  Wire.write(0x20);  // Command: Set single LED
  Wire.write(ledIndex);
  Wire.write(c.r);
  Wire.write(c.g);
  Wire.write(c.b);
  uint8_t error = Wire.endTransmission();
  
  if (error != 0) {
    Serial.printf("[LED] Failed to set LED %d on 0x%02X (error %d)\n", 
                  ledIndex, i2cAddress, error);
  }
}




// Set all LEDs on a device at once (for MIDI feedback mode - future use)
void setAllDeviceLEDs(uint8_t i2cAddress, CRGB leds[5]) {
  Wire.beginTransmission(i2cAddress);
  Wire.write(0x21);  // Command: Set all LEDs
  for (int i = 0; i < 5; i++) {
    CRGB c = scaleForBrightness(leds[i]);
    Wire.write(c.r);
    Wire.write(c.g);
    Wire.write(c.b);
  }
  uint8_t error = Wire.endTransmission();
  
  if (error != 0) {
    Serial.printf("[LED] Failed to set all LEDs on 0x%02X (error %d)\n", 
                  i2cAddress, error);
  }
}



void handleGetLEDConfig() {
  if (!server.hasArg("device") || !server.hasArg("button")) {
    server.send(400, "text/plain", "Missing parameters");
    return;
  }
  
  int slot = server.arg("device").toInt();
  int button = server.arg("button").toInt();
  
  SlaveInfo* slave = getSlaveInfoForSlot(slot);
  if (!slave) {
    server.send(404, "text/plain", "Device not found");
    return;
  }
  
  String json = "{";
  json += "\"mode\":\"" + slave->config.ledMode[button] + "\",";
  json += "\"color\":\"" + slave->config.ledColor[button] + "\",";
  json += "\"midiNote\":" + String(slave->config.ledMidiNote[button]) + ",";
  json += "\"midiChannel\":" + String(slave->config.ledMidiChannel[button]) + ",";
  json += "\"followLayer\":" + String(slave->config.ledFollowLayer[button] ? "true" : "false") + ",";
  json += "\"oscAddr\":\"" + jsonEsc(slave->config.ledOscAddr[button]) + "\"";
  json += "}";
  
  server.send(200, "application/json", json);
}

void handleSaveLEDConfig() {
  if (server.method() != HTTP_POST) {
    server.send(405, "text/plain", "Method not allowed");
    return;
  }
  
  String body = server.arg("plain");
  
  // Parse with ArduinoJson. Missing fields keep their current values
  // instead of being reset to defaults.
  DynamicJsonDocument doc(512);
  DeserializationError jsonErr = deserializeJson(doc, body);
  if (jsonErr) {
    server.send(400, "text/plain", String("JSON parse error: ") + jsonErr.c_str());
    return;
  }

  int device = doc["device"] | -1;
  int button = doc["button"] | -1;
  if (device < 0 || device >= 8 || button < 0 || button >= 4) {
    server.send(400, "text/plain", "Invalid parameters");
    return;
  }

  SlaveInfo* slave = getSlaveInfoForSlot(device);
  if (!slave) {
    server.send(404, "text/plain", "Device not found");
    return;
  }

  DeviceConfig& cfg = slave->config;
  cfg.ledMode[button]        = doc["mode"]        | cfg.ledMode[button];
  cfg.ledColor[button]       = doc["color"]       | cfg.ledColor[button];
  cfg.ledMidiNote[button]    = doc["midiNote"]    | cfg.ledMidiNote[button];
  cfg.ledMidiChannel[button] = doc["midiChannel"] | cfg.ledMidiChannel[button];
  cfg.ledFollowLayer[button] = doc["followLayer"] | cfg.ledFollowLayer[button];
  cfg.ledOscAddr[button]     = doc["oscAddr"]     | cfg.ledOscAddr[button];
  
  saveDeviceConfigWithLayers(deviceMapping[device]);
  
  // Update LED immediately
  bool currentState = slave->lastState & (1 << button);
  updateDeviceLED(device, button, currentState);
  
  server.send(200, "text/plain", "OK");
}

// ─── Mapping overview + global LED brightness endpoints ──────────────────────
void handleGetFullMapping() {
  String json = "{\"slots\":[";
  bool firstSlot = true;
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (!slave) continue;
    if (!firstSlot) json += ",";
    firstSlot = false;

    bool isPedal   = (slave->type >= TYPE_PEDAL_MIN   && slave->type <= TYPE_PEDAL_MAX);
    bool isEncoder = (slave->type >= TYPE_ENCODER_MIN && slave->type <= TYPE_ENCODER_MAX);

    json += "{\"slot\":" + String(slot) + ",";
    json += "\"deviceId\":\"" + idToShortKey(slave->uniqueID) + "\",";
    json += "\"type\":\"" + typeToName(slave->type) + "\",";
    json += "\"isPedal\":" + String(isPedal ? "true" : "false") + ",";
    json += "\"isEncoder\":" + String(isEncoder ? "true" : "false") + ",";

    if (isEncoder) {
      json += "\"encoder\":{";
      json += "\"rotationMode\":\"" + slave->config.encoderRotationMode + "\",";
      json += "\"midiCC\":" + String(slave->config.encoderMidiCC) + ",";
      json += "\"midiChannel\":" + String(slave->config.encoderMidiChannel) + ",";
      json += "\"noteNumber\":" + String(slave->config.encoderNoteNumber) + ",";
      json += "\"noteChannel\":" + String(slave->config.encoderNoteChannel) + "},";
    }

    json += "\"buttons\":[";
    int numBtns = isPedal ? (slave->type == 41 ? 2 : 1) : 4;
    for (int btn = 0; btn < numBtns; btn++) {
      if (btn > 0) json += ",";
      json += "{\"dc\":\"" + slave->config.deviceControlType[btn] + "\",";
      json += "\"dcTarget\":" + String(slave->config.deviceControlTarget[btn]) + ",";
      json += "\"dcMode\":\"" + slave->config.deviceControlMode[btn] + "\",";
      json += "\"shift\":" + String((slave->config.shiftEnabled && slave->config.shiftButton == btn) ? "true" : "false") + ",";
      json += "\"layers\":[";
      for (int layer = 0; layer < 4; layer++) {
        if (layer > 0) json += ",";
        json += "{\"t\":\"" + slave->config.layerMidiType[layer][btn] + "\",";
        json += "\"n\":" + String(slave->config.layerMidiMapping[layer][btn]) + ",";
        json += "\"c\":" + String(slave->config.layerMidiChannel[layer][btn]) + ",";
        json += "\"b\":\"" + slave->config.layerMidiBehavior[layer][btn] + "\",";
        json += "\"a\":\"" + jsonEsc(slave->config.layerOscAddr[layer][btn]) + "\"}";
      }
      json += "]}";
    }
    json += "]}";
    yield();
  }
  json += "],\"numLayers\":" + String(configNumLayers) + "}";
  server.send(200, "application/json", json);
}

void handleSaveLedBrightness() {
  if (!server.hasArg("value")) {
    server.send(400, "text/plain", "Missing value");
    return;
  }
  int v = constrain(server.arg("value").toInt(), 5, 255);
  globalLEDBrightness = (uint8_t)v;
  prefs.begin("ui", false);
  prefs.putInt("ledBright", v);
  prefs.end();
  FastLED.setBrightness(globalLEDBrightness);
  updateLayerLED(getCurrentActiveLayer());  // refresh onboard + device layer LEDs now
  Serial.printf("[LED] Global brightness set to %d\n", v);
  server.send(200, "text/plain", "OK");
}

String jsonEsc(const String& in) {
  String o; o.reserve(in.length() + 8);
  for (unsigned k = 0; k < in.length(); k++) {
    char ch = in[k];
    if (ch == '"' || ch == '\\') { o += '\\'; o += ch; }
    else if ((uint8_t)ch < 0x20) o += ' ';
    else o += ch;
  }
  return o;
}

void handleGetOsc() {
  String ip = wifiConnected ? WiFi.localIP().toString() : (apModeActive ? WiFi.softAPIP().toString() : "");
  String j = "{";
  j += "\"enabled\":" + String(oscCfg.enabled ? "true" : "false");
  j += ",\"running\":" + String(oscRunning ? "true" : "false");
  j += ",\"inPort\":" + String(oscCfg.inPort);
  j += ",\"outPort\":" + String(oscCfg.outPort);
  j += ",\"targetIP\":\"" + jsonEsc(oscCfg.targetIP) + "\"";
  j += ",\"prefix\":\"" + jsonEsc(oscCfg.prefix) + "\"";
  j += ",\"sendControls\":" + String(oscCfg.sendControls ? "true" : "false");
  j += ",\"mirrorMidiOut\":" + String(oscCfg.mirrorMidiOut ? "true" : "false");
  j += ",\"mirrorMidiIn\":" + String(oscCfg.mirrorMidiIn ? "true" : "false");
  j += ",\"deviceIP\":\"" + ip + "\"";
  j += ",\"lastSender\":\"" + String(oscHaveSender ? oscLastSender.toString() : "") + "\"";
  j += ",\"rx\":" + String(oscRxCount) + ",\"tx\":" + String(oscTxCount);
  j += "}";
  server.send(200, "application/json", j);
}

void handleSaveOsc() {
  if (server.hasArg("enabled"))      oscCfg.enabled       = server.arg("enabled") == "1";
  if (server.hasArg("inPort"))       oscCfg.inPort        = constrain(server.arg("inPort").toInt(), 1, 65535);
  if (server.hasArg("outPort"))      oscCfg.outPort       = constrain(server.arg("outPort").toInt(), 1, 65535);
  if (server.hasArg("targetIP"))   { oscCfg.targetIP      = server.arg("targetIP"); oscCfg.targetIP.trim(); }
  if (server.hasArg("prefix"))       oscCfg.prefix        = oscNormalizePrefix(server.arg("prefix"));
  if (server.hasArg("sendControls")) oscCfg.sendControls  = server.arg("sendControls") == "1";
  if (server.hasArg("mirrorMidiOut"))oscCfg.mirrorMidiOut = server.arg("mirrorMidiOut") == "1";
  if (server.hasArg("mirrorMidiIn")) oscCfg.mirrorMidiIn  = server.arg("mirrorMidiIn") == "1";
  IPAddress tmp;
  if (oscCfg.targetIP.length() && !tmp.fromString(oscCfg.targetIP)) {
    server.send(400, "text/plain", "Target IP is not a valid IPv4 address");
    return;
  }
  prefs.begin("osc", false);
  prefs.putBool("en", oscCfg.enabled);
  prefs.putUShort("in", oscCfg.inPort);
  prefs.putUShort("out", oscCfg.outPort);
  prefs.putString("ip", oscCfg.targetIP);
  prefs.putString("pfx", oscCfg.prefix);
  prefs.putBool("ctl", oscCfg.sendControls);
  prefs.putBool("mout", oscCfg.mirrorMidiOut);
  prefs.putBool("min", oscCfg.mirrorMidiIn);
  prefs.end();
  if (oscCfg.enabled && (wifiConnected || apModeActive)) startOSC(); else stopOSC();
  handleGetOsc();
}

void handleGetOscLog() {
  String j = "[";
  for (int k = 0; k < oscLogCount; k++) {
    int idx = (oscLogHead - oscLogCount + k + OSC_LOG_SIZE) % OSC_LOG_SIZE;
    if (k) j += ",";
    j += "\"" + jsonEsc(oscLogBuf[idx]) + "\"";
  }
  j += "]";
  server.send(200, "application/json", j);
}

void handleOscTest() {
  if (!oscRunning) { server.send(400, "text/plain", "OSC is not running"); return; }
  if (!oscHaveTarget && !oscHaveSender) {
    server.send(400, "text/plain", "No target yet: set a Send-to IP, or send any OSC message to the device first");
    return;
  }
  OscMsg m(oscA("/test")); m.str("hello from " + wifiHostname); oscSend(m);
  oscSendSync();
  server.send(200, "text/plain", "OK");
}

void handleOscSendTest() {
  if (!oscRunning) { server.send(400, "text/plain", "OSC is turned off. Enable it in the OSC tab."); return; }
  if (!oscHaveTarget && !oscHaveSender) { server.send(400, "text/plain", "No destination yet. Set a Send-to IP in the OSC tab."); return; }
  String addr = server.arg("addr"); addr.trim();
  if (!addr.startsWith("/")) { server.send(400, "text/plain", "Address must start with /"); return; }
  oscSendAction(addr, server.hasArg("arg") ? server.arg("arg") : "f", server.arg("val").toFloat());
  server.send(200, "text/plain", "Sent");
}

void handleGetLayerState() {
  server.send(200, "application/json", layerStateJson());
}

// Set the global layer from the web UI (also clears per-device overrides)
void handleSetGlobalLayer() {
  if (!server.hasArg("layer")) { server.send(400, "text/plain", "Missing layer"); return; }
  applyGlobalLayer(server.arg("layer").toInt());
  server.send(200, "application/json", layerStateJson());
}

void handleGetLedBrightness() {
  server.send(200, "text/plain", String(globalLEDBrightness));
}

void handleDeviceControl(int slot, int button, bool pressed, DeviceConfig& config) {
  String controlType = config.deviceControlType[button];
  int targetLayer = config.deviceControlTarget[button];
  String controlMode = config.deviceControlMode[button];

  Serial.printf("[DEVICE CONTROL] Slot %d, Btn %d, Type: %s, Target: %d, Mode: %s, Pressed: %s\n",
                slot, button, controlType.c_str(), targetLayer, controlMode.c_str(), pressed ? "Y" : "N");

  webSerial.printf("[DEVICE CONTROL] Slot %d, Btn %d, Type: %s, Target: %d, Mode: %s, Pressed: %s\n",
                slot, button, controlType.c_str(), targetLayer, controlMode.c_str(), pressed ? "Y" : "N");              

  if (controlType == "layer_device") {
    Serial.printf("[DEBUG] Device layer before: %d\n", shiftStates[slot].currentLayer);
    
    if (controlMode == "momentary") {
      if (pressed) {
        if (shiftStates[slot].currentLayer != targetLayer) {
          shiftStates[slot].currentLayer = targetLayer;
        } else {
          shiftStates[slot].currentLayer = 0;
        }
      } else {
        shiftStates[slot].currentLayer = 0;
      }
    } else if (controlMode == "latching") {
      if (pressed) {
        shiftStates[slot].currentLayer = (shiftStates[slot].currentLayer == targetLayer) ? 0 : targetLayer;
      }
    } else if (controlMode == "jump") {
      if (pressed && shiftStates[slot].currentLayer != targetLayer) {
        shiftStates[slot].currentLayer = targetLayer;
      }
    }

    Serial.printf("[DEBUG] Device layer after: %d\n", shiftStates[slot].currentLayer);
    Serial.printf("[DEVICE CONTROL] Device %d Layer changed to: %d\n", slot, shiftStates[slot].currentLayer);
    
    // Update LEDs for device layer changes
    updateLayerLED(getCurrentActiveLayer());
    
  } else if (controlType == "layer_global") {
    Serial.printf("[DEBUG] Global layer before: enabled=%s, layer=%d\n", 
                  globalLayerControl.enabled ? "Y" : "N", globalLayerControl.currentGlobalLayer);
    
    globalLayerControl.enabled = true;

    if (controlMode == "momentary") {
      if (pressed) {
        if (globalLayerControl.currentGlobalLayer != targetLayer) {
          globalLayerControl.currentGlobalLayer = targetLayer;
        } else {
          globalLayerControl.currentGlobalLayer = 0;
        }
      } else {
        globalLayerControl.currentGlobalLayer = 0;
      }
    } else if (controlMode == "latching") {
      if (pressed) {
        globalLayerControl.currentGlobalLayer = (globalLayerControl.currentGlobalLayer == targetLayer) ? 0 : targetLayer;
      }
    } else if (controlMode == "jump") {
      if (pressed && globalLayerControl.currentGlobalLayer != targetLayer) {
        globalLayerControl.currentGlobalLayer = targetLayer;
      }
    }

    Serial.printf("[DEBUG] Global layer after: enabled=%s, layer=%d\n", 
                  globalLayerControl.enabled ? "Y" : "N", globalLayerControl.currentGlobalLayer);
    
    // Update LEDs for global layer changes
    updateLayerLED(globalLayerControl.currentGlobalLayer);

    // Save global layer state immediately
    spiffsConfig.saveGlobalConfig(globalLayerControl.enabled, globalLayerControl.currentGlobalLayer);

    Serial.printf("[DEVICE CONTROL] Global Layer changed to: %d\n", globalLayerControl.currentGlobalLayer);
    webSerial.printf("[DEVICE CONTROL] Global Layer changed to: %d\n", globalLayerControl.currentGlobalLayer);              
  } 
  else if (controlType == "layer_up_device") {
    if (pressed) {
      int currentLayer = shiftStates[slot].currentLayer;
      int newLayer = (currentLayer + 1) % configNumLayers;
      shiftStates[slot].currentLayer = newLayer;
      
      Serial.printf("[DEVICE CONTROL] Device %d Layer UP: %d -> %d\n", slot, currentLayer, newLayer);
      webSerial.printf("[DEVICE CONTROL] Device %d Layer UP: %d -> %d\n", slot, currentLayer, newLayer);
      
      // Update LEDs
      updateLayerLED(getCurrentActiveLayer());
    }
  }
  else if (controlType == "layer_down_device") {
    if (pressed) {
      int currentLayer = shiftStates[slot].currentLayer;
      int newLayer = (currentLayer - 1 + configNumLayers) % configNumLayers;
      shiftStates[slot].currentLayer = newLayer;
      
      Serial.printf("[DEVICE CONTROL] Device %d Layer DOWN: %d -> %d\n", slot, currentLayer, newLayer);
      webSerial.printf("[DEVICE CONTROL] Device %d Layer DOWN: %d -> %d\n", slot, currentLayer, newLayer);
      
      // Update LEDs
      updateLayerLED(getCurrentActiveLayer());
    }
  }
  else if (controlType == "layer_up_global") {
    if (pressed) {
      globalLayerControl.enabled = true;
      int currentLayer = globalLayerControl.currentGlobalLayer;
      int newLayer = (currentLayer + 1) % configNumLayers;
      globalLayerControl.currentGlobalLayer = newLayer;
      
      Serial.printf("[DEVICE CONTROL] Global Layer UP: %d -> %d\n", currentLayer, newLayer);
      webSerial.printf("[DEVICE CONTROL] Global Layer UP: %d -> %d\n", currentLayer, newLayer);
      
      // Update LEDs
      updateLayerLED(newLayer);
      
      spiffsConfig.saveGlobalConfig(globalLayerControl.enabled, globalLayerControl.currentGlobalLayer);
    }
  }
  else if (controlType == "layer_down_global") {
    if (pressed) {
      globalLayerControl.enabled = true;
      int currentLayer = globalLayerControl.currentGlobalLayer;
      int newLayer = (currentLayer - 1 + configNumLayers) % configNumLayers;
      globalLayerControl.currentGlobalLayer = newLayer;
      
      Serial.printf("[DEVICE CONTROL] Global Layer DOWN: %d -> %d\n", currentLayer, newLayer);
      webSerial.printf("[DEVICE CONTROL] Global Layer DOWN: %d -> %d\n", currentLayer, newLayer);
      
      // Update LEDs
      updateLayerLED(newLayer);
      
      spiffsConfig.saveGlobalConfig(globalLayerControl.enabled, globalLayerControl.currentGlobalLayer);
    }
  }
}


// Add LED breathing effect for startup (optional)
void startupLEDSequence() {
  Serial.println("Starting LED sequence...");

  // Cycle through all layer colors briefly
  for (int layer = 0; layer < 4; layer++) {
    leds[0] = LAYER_COLORS[layer];
    FastLED.show();
    delay(300);
  }

  // Set to current active layer
  updateLayerLED(getCurrentActiveLayer());
}

#include "PequodLink.h"

// Main Setup and Loop
void setup() {
  Serial.begin(115200);
  Serial1.begin(31250, SERIAL_8N1, MIDI_UART_RX, MIDI_UART_TX);
  // ADD THIS: Initialize USB MIDI
   USB.productName("Longsleeves");
  USB.manufacturerName("Your Company Name");
  CS::Control_Surface.begin();
  usbmidi.begin();
  pqSetup();              // PEQUOD Link: SysEx + USB MIDI input
  Wire.begin(SDA_PIN, SCL_PIN);
  Wire.setClock(400000);  // 400 kHz fast mode — reduces per-transaction time 4x
  Wire.setClock(100000);
  Wire.setTimeout(100);

  initStackCanary();
  Serial.printf("Initial free heap: %d bytes\n", ESP.getFreeHeap());

  // Initialize SPIFFS instead of checking NVS
  if (!SPIFFS.begin(true)) {
    Serial.println("SPIFFS Mount Failed");
    // Could fall back to defaults or restart
  } else {
    Serial.println("SPIFFS mounted successfully");
    spiffsConfig.listConfigs();  // Show stored configurations
  }

  // Load WiFi config from SPIFFS
  spiffsConfig.loadWiFiConfig(wifiPrimarySSID, wifiPrimaryPass,
                              wifiFallbackSSID, wifiFallbackPass, wifiHostname);

  // Load UI settings
  spiffsConfig.loadUIConfig(THEME_COLOR, advancedOptionsEnabled, midiOutputMode);

  // Load global layer settings
  spiffsConfig.loadGlobalConfig(globalLayerControl.enabled,
                                globalLayerControl.currentGlobalLayer);

  Serial.println("=== ESP32 MIDI Controller Starting (Fixed Version v1.3.1) ===");

  // Initialize display
  startupTime = millis();
  initDisplay();

  // Load theme color
  prefs.begin("ui_settings", true);
  THEME_COLOR = prefs.getString("theme_color", "#4CAF50");
  prefs.end();


  //checkNVSStatus();
  //testSimpleDeviceControlSave();



  // Scan and configure devices
  scanSlaves();
  Serial.println("Loading device configurations with timeout protection...");
  unsigned long configStart = millis();
  for (int i = 0; i < numDetected; i++) {
    if (millis() - configStart > 10000) {  // 10 second timeout
      Serial.println("WARNING: Config load timeout");
      break;
    }

    if (!checkMemoryHealth()) {
      Serial.println("WARNING: Stopping config load due to low memory");
      break;
    }

    loadDeviceConfigWithLayers(i);
    yield();  // Allow other tasks
  }

  autoMapDevices();
  initializeShiftStates();  // Initialize shift states
initializeEncoderStates();
  Serial.println("=== Device Configuration Summary ===");
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (slave) {
      Serial.printf("Slot %d (%s): ", slot, idToShortKey(slave->uniqueID).c_str());
      for (int btn = 0; btn < 4; btn++) {
        Serial.printf("Btn%d=%s%d(Ch%d) ", btn,
                      slave->config.midiType[btn].c_str(),
                      slave->config.midiMapping[btn],
                      slave->config.midiChannel[btn]);
      }
      if (slave->type >= TYPE_PEDAL_MIN && slave->type <= TYPE_PEDAL_MAX) {
        Serial.printf("PedalCal=[%d-%d]inv=%s ",
                      slave->config.pedalCalMin, slave->config.pedalCalMax,
                      slave->config.pedalInvert ? "Y" : "N");
      }
      Serial.println();
    }

    static unsigned long lastDisplayUpdate = 0;
    if (millis() - lastDisplayUpdate >= 250) {
      updateDisplayWithShiftInfo();  // Use enhanced display function
      lastDisplayUpdate = millis();
    }
  }

  prefs.begin("ui_settings", true);
  advancedOptionsEnabled = prefs.getBool("advanced_enabled", false);
  prefs.end();

  // Verify device control configuration loading
  Serial.println("=== Device Control Configuration Verification ===");
  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (slave) {
      Serial.printf("Slot %d (%s):\n", slot, idToShortKey(slave->uniqueID).c_str());
      for (int btn = 0; btn < 4; btn++) {
        if (slave->config.deviceControlType[btn] != "none") {
          Serial.printf("  Btn%d: %s -> Layer %d (%s)\n", btn,
                        slave->config.deviceControlType[btn].c_str(),
                        slave->config.deviceControlTarget[btn],
                        slave->config.deviceControlMode[btn].c_str());
        }
      }
    }
  }


  //checkNVSStatus();

  dumpPreferences();

  // Load the configured number of active layers + LED brightness from NVS
  prefs.begin("ui", true);
  configNumLayers = constrain(prefs.getInt("numLayers", 4), 1, 4);
  globalLEDBrightness = (uint8_t)constrain(prefs.getInt("ledBright", 255), 5, 255);
  prefs.end();

  loadOscConfig();

  // Load MIDI 2.0 preference
  prefs.begin("midi", true);
  midi2Pref = prefs.getBool("midi2Enabled", false);
  prefs.end();

  // Increase web server buffer
  server.setContentLength(CONTENT_LENGTH_UNKNOWN);
  startWiFi();

  // Setup web server routes
  server.on("/", handleRoot);
  server.on("/scan_wifi_ajax", handleScanWiFiAjax);
  server.on("/scan_i2c_ajax", handleScanI2CAjax);
  server.on("/wifi_status_json", handleWiFiStatusJson);
  server.on("/save_wifi", HTTP_POST, handleSaveWiFi);
  server.on("/save_midi", handleSaveMidi);
  server.on("/save_pedal_cal", handleSavePedalCal);
  server.on("/get_pedal_cal", handleGetPedalCal);
  server.on("/save_midi_channel_name", handleSaveMidiChannelName);
  server.on("/get_midi_channel_names", handleGetMidiChannelNames);
  server.on("/save_theme", handleSaveTheme);
  server.on("/midi_panic", handleMidiPanic);
  server.on("/system_info", handleSystemInfo);
  server.on("/factory_reset", HTTP_POST, handleFactoryReset);
  server.on("/device_state_json", handleDeviceStateJson);
  server.on("/get_device_configs", handleGetDeviceConfigs);  // NEW: Endpoint for device configs
  server.on("/get_pedal_raw", handleGetPedalRawData);
  server.on("/save_slot_assignment", handleSaveSlotAssignment);
  server.on("/get_slot_mappings", handleGetSlotMappings);
  server.on("/save_advanced_toggle", handleSaveAdvancedToggle);
  server.on("/get_num_layers", handleGetNumLayers);
  server.on("/save_num_layers", handleSaveNumLayers);

  server.on("/get_midi2", []() {
    prefs.begin("midi", true);
    bool en = prefs.getBool("midi2Enabled", false);
    prefs.end();
    server.send(200, "text/plain", en ? "1" : "0");
  });
  server.on("/save_midi2", []() {
    bool en = server.arg("enabled") == "1";
    prefs.begin("midi", false);
    prefs.putBool("midi2Enabled", en);
    prefs.end();
    if (en && wifiConnected) startMidi2Network();
    else if (!en)            stopMidi2Network();
    server.send(200, "text/plain", "OK");
  });
  server.on("/save_i2c_speed", handleSaveI2CSpeed);
  server.on("/save_midi_polling", handleSaveMidiPolling);
  server.on("/toggle_debug", handleToggleDebug);
  server.on("/save_shift_config", handleSaveShiftConfig);
  server.on("/get_layer_midi", handleGetLayerMidi);
  server.on("/save_layer_midi", handleSaveLayerMidi);
  server.on("/get_device_shift_config", handleGetDeviceShiftConfig);
  server.on("/save_device_control", handleSaveDeviceControl);
  server.on("/get_device_control", handleGetDeviceControl);
  server.on("/save_global_layer_control", handleSaveGlobalLayerControl);
  server.on("/get_lcd_content", handleGetLCDContent);
  server.on("/save_midi_output", handleSaveMidiOutput);
  server.on("/get_midi_output", handleGetMidiOutput);
  server.on("/get_serial_log", handleGetSerialLog);
  server.on("/clear_serial_log", handleClearSerialLog);
  server.on("/toggle_web_serial", handleToggleWebSerial);
  server.on("/save_layer_color", handleSaveLayerColor);
  server.on("/get_theme_settings", handleGetThemeSettings);
  server.on("/refresh_led", handleRefreshLED);
  server.on("/get_encoder_config", handleGetEncoderConfig);
  server.on("/save_encoder_config", HTTP_POST, handleSaveEncoderConfig);
  server.on("/get_led_config", handleGetLEDConfig);
server.on("/save_led_config", HTTP_POST, handleSaveLEDConfig);
  server.on("/get_full_mapping", handleGetFullMapping);
  server.on("/save_led_brightness", handleSaveLedBrightness);
  server.on("/get_led_brightness", handleGetLedBrightness);
  server.on("/get_layer_state", handleGetLayerState);
  server.on("/get_osc", handleGetOsc);
  server.on("/save_osc", handleSaveOsc);
  server.on("/get_osc_log", handleGetOscLog);
  server.on("/osc_test", handleOscTest);
  server.on("/osc_send_test", handleOscSendTest);
  server.on("/set_global_layer", handleSetGlobalLayer);

  // Add this after your other initializations, before "Setup Complete"
  Serial.println("Initializing Layer LED...");
  initLayerLED();

  // Optional: Run startup sequence
  startupLEDSequence();
// Initialize all button device LEDs with current layer color
    Serial.println("Initializing device LEDs...");
    webSerial.println("Initializing device LEDs...");
    delay(500);  // Give devices time to fully initialize
    int initialLayer = getCurrentActiveLayer();
    updateAllDeviceLayerLEDs(initialLayer);


  Serial.println("=== Setup Complete ===");
  //checkFreeMemory();
}




// ─── MIDI 2.0 Network Transport (UDP, MIDI Association specification) ────────

static const uint16_t M2_CTRL = 5004;   // control / session port
static const uint16_t M2_DATA = 5005;   // MIDI data port

static WiFiUDP m2Ctrl;
static WiFiUDP m2Data;
static uint32_t m2SSRC           = 0;   // our random SSRC
static uint32_t m2RemoteSSRC     = 0;
static IPAddress m2RemoteIP;
static uint16_t m2RemoteCtrlPort = 0;
static uint16_t m2RemoteDataPort = 0;
static bool m2Active             = false;
static uint16_t m2SeqNum         = 0;
static unsigned long m2LastRx    = 0;

// ── scale helpers ─────────────────────────────────────────────────────────────
static inline uint32_t m2Scale7to32(uint8_t v7) {
  return ((uint32_t)v7<<25)|((uint32_t)v7<<18)|((uint32_t)v7<<11)|
         ((uint32_t)v7<<4)|(v7>>3);
}
static inline uint16_t m2Scale7to16(uint8_t v7) {
  return (uint16_t)(v7 * 512 + (v7 >> 2));
}

// ── send helpers ──────────────────────────────────────────────────────────────
static void m2WriteU32(uint8_t* buf, int& pos, uint32_t v) {
  buf[pos++]=(v>>24)&0xFF; buf[pos++]=(v>>16)&0xFF;
  buf[pos++]=(v>> 8)&0xFF; buf[pos++]= v     &0xFF;
}
static void m2WriteU64(uint8_t* buf, int& pos, uint64_t v) {
  for (int i=7;i>=0;i--) buf[pos++]=(v>>(i*8))&0xFF;
}

static void m2SendSession(IPAddress ip, uint16_t port,
                          uint16_t cmd, uint32_t tok, uint32_t ssrc,
                          const char* name = nullptr) {
  uint8_t buf[128]; int p=0;
  buf[p++]=0xFF; buf[p++]=0xFF;
  buf[p++]=(cmd>>8)&0xFF; buf[p++]=cmd&0xFF;
  buf[p++]=0x00; buf[p++]=0x02;           // protocol version 2
  m2WriteU32(buf,p,tok);
  m2WriteU32(buf,p,ssrc);
  if (name) { while(*name&&p<120) buf[p++]=*name++; }
  buf[p++]=0;
  m2Ctrl.beginPacket(ip,port); m2Ctrl.write(buf,p); m2Ctrl.endPacket();
}

static void m2SendClock(IPAddress ip, uint16_t port,
                        uint32_t tok, uint32_t ssrc,
                        uint64_t ck1, uint64_t ck2, uint64_t ck3) {
  uint8_t buf[40]; int p=0;
  buf[p++]=0xFF; buf[p++]=0xFF;
  buf[p++]=0x43; buf[p++]=0x4B;           // "CK"
  buf[p++]=0x00; buf[p++]=0x02;
  m2WriteU32(buf,p,tok);
  m2WriteU32(buf,p,ssrc);
  m2WriteU64(buf,p,ck1);
  m2WriteU64(buf,p,ck2);
  m2WriteU64(buf,p,ck3);
  m2Ctrl.beginPacket(ip,port); m2Ctrl.write(buf,p); m2Ctrl.endPacket();
}

static void m2SendUmp(const uint32_t* words, int n) {
  if (!m2Active || m2RemoteDataPort==0) return;
  if (n < 0 || n > 8) return;  // guard: max 8 UMP words = 32 bytes payload
  uint8_t buf[12 + 8*4];       // fixed max size: RTP header + 8 words
  int total = 12 + n*4;
  int p=0;
  uint32_t ts = (uint32_t)(micros()/100);
  buf[p++]=0x80;                           // V=2 P=0 X=0 CC=0
  buf[p++]=0x61;                           // M=0 PT=97
  buf[p++]=(m2SeqNum>>8)&0xFF; buf[p++]=m2SeqNum&0xFF;
  m2SeqNum++;
  m2WriteU32(buf,p,ts);
  m2WriteU32(buf,p,m2SSRC);
  for (int i=0;i<n;i++) m2WriteU32(buf,p,words[i]);
  m2Data.beginPacket(m2RemoteIP,m2RemoteDataPort);
  m2Data.write(buf,total);
  m2Data.endPacket();
}

// ── public MIDI send API ──────────────────────────────────────────────────────
void midi2NetNoteOn(uint8_t ch, uint8_t note, uint8_t vel) {
  if (!midi2Enabled) return;
  uint32_t w1=(0x4u<<28)|(0x9u<<20)|((uint32_t)((ch-1)&0xF)<<16)|((uint32_t)note<<8);
  uint32_t w2=(uint32_t)m2Scale7to16(vel)<<16;
  uint32_t p[2]={w1,w2}; m2SendUmp(p,2);
}
void midi2NetNoteOff(uint8_t ch, uint8_t note, uint8_t vel) {
  if (!midi2Enabled) return;
  uint32_t w1=(0x4u<<28)|(0x8u<<20)|((uint32_t)((ch-1)&0xF)<<16)|((uint32_t)note<<8);
  uint32_t w2=(uint32_t)m2Scale7to16(vel)<<16;
  uint32_t p[2]={w1,w2}; m2SendUmp(p,2);
}
void midi2NetCC(uint8_t ch, uint8_t cc, uint8_t val) {
  if (!midi2Enabled) return;
  // MIDI 2.0 Assignable Controller (status 0x3, 64-bit)
  uint32_t w1=(0x4u<<28)|(0x3u<<20)|((uint32_t)((ch-1)&0xF)<<16)|cc;
  uint32_t w2=m2Scale7to32(val);
  uint32_t p[2]={w1,w2}; m2SendUmp(p,2);
}

// ── process incoming control port ─────────────────────────────────────────────
static void m2PollCtrl() {
  int sz = m2Ctrl.parsePacket();
  if (sz < 10) return;
  uint8_t buf[256];
  int len = m2Ctrl.read(buf, sizeof(buf));
  if (len < 10 || buf[0]!=0xFF || buf[1]!=0xFF) return;

  uint16_t cmd       = ((uint16_t)buf[2]<<8)|buf[3];
  uint32_t initiator = len>=10 ? ((uint32_t)buf[6]<<24)|((uint32_t)buf[7]<<16)|
                                  ((uint32_t)buf[8]<<8)|buf[9] : 0;
  uint32_t rssrc     = len>=14 ? ((uint32_t)buf[10]<<24)|((uint32_t)buf[11]<<16)|
                                  ((uint32_t)buf[12]<<8)|buf[13] : 0;
  IPAddress rip   = m2Ctrl.remoteIP();
  uint16_t  rport = m2Ctrl.remotePort();

  switch (cmd) {
    case 0x494E:  // IN — invitation from host
      m2RemoteSSRC     = rssrc;
      m2RemoteIP       = rip;
      m2RemoteCtrlPort = rport;
      m2RemoteDataPort = rport + 1;   // data port is control + 1
      m2Active         = true;
      m2LastRx         = millis();
      m2SendSession(rip, rport, 0x4F4B, initiator, m2SSRC, "I2C MIDI 2.0");
      Serial.printf("[MIDI 2.0] Session accepted from %s\n", rip.toString().c_str());
      break;

    case 0x4259:  // BY — host ending session
      if (m2Active) {
        m2Active         = false;
        m2RemoteDataPort = 0;
        Serial.println("[MIDI 2.0] Session ended by host");
      }
      break;

    case 0x434B: { // CK — clock sync, echo ck1 → ck2
      if (len < 38) break;
      uint64_t ck1=0; for(int i=0;i<8;i++) ck1=(ck1<<8)|buf[14+i];
      uint64_t ck3=(uint64_t)(micros()/100);
      m2SendClock(rip, rport, initiator, m2SSRC, ck1, ck1, ck3);
      m2LastRx = millis();
      break;
    }
    default: break;
  }
}

// ── process incoming data port ────────────────────────────────────────────────
static void m2PollData() {
  int sz = m2Data.parsePacket();
  if (sz < 12) return;
  uint8_t buf[256];
  int len = m2Data.read(buf, sizeof(buf));
  if (len < 12) return;
  int off = 12;  // skip 12-byte RTP header
  while (off + 4 <= len) {
    uint32_t w1=((uint32_t)buf[off]<<24)|((uint32_t)buf[off+1]<<16)|
                ((uint32_t)buf[off+2]<<8)|buf[off+3]; off+=4;
    uint8_t mt=(w1>>28)&0xF;
    if (mt==0x2) {
      handleMidiInputMessage((((w1>>20)&0xF)<<4), ((w1>>16)&0xF)+1,
                             (w1>>8)&0x7F, w1&0x7F);
    } else if (mt==0x4) {
      if (off+4>len) break;
      uint32_t w2=((uint32_t)buf[off]<<24)|((uint32_t)buf[off+1]<<16)|
                  ((uint32_t)buf[off+2]<<8)|buf[off+3]; off+=4;
      uint8_t status=(w1>>20)&0xF, ch=((w1>>16)&0xF)+1;
      uint8_t d1=(w1>>8)&0x7F, d2=(uint8_t)((w2>>25)&0x7F);
      handleMidiInputMessage(status<<4, ch, d1, d2);
    }
  }
  m2LastRx = millis();
}

// ── lifecycle ─────────────────────────────────────────────────────────────────
void startMidi2Network() {
  m2SSRC = (uint32_t)esp_random();
  m2Ctrl.begin(M2_CTRL);
  m2Data.begin(M2_DATA);
  // mDNS: Windows MIDI Services scans for _midi2._udp
  ensureMDNS();
  MDNS.addService("midi2", "udp", M2_CTRL);
  MDNS.addServiceTxt("midi2", "udp", "ver",  "2");
  MDNS.addServiceTxt("midi2", "udp", "ssrc", String(m2SSRC, HEX).c_str());
  MDNS.addServiceTxt("midi2", "udp", "en",   "1");
  midi2Enabled = true;
  Serial.printf("[MIDI 2.0] UDP ready — ctrl:%d data:%d ssrc:%08X\n",
                M2_CTRL, M2_DATA, m2SSRC);
  Serial.println("[MIDI 2.0] mDNS: _midi2._udp / i2c-midi2.local");
}

void stopMidi2Network() {
  if (m2Active)
    m2SendSession(m2RemoteIP, m2RemoteCtrlPort, 0x4259, 0, m2SSRC);
  m2Ctrl.stop();
  m2Data.stop();
  m2Active     = false;
  midi2Enabled = false;
  Serial.println("[MIDI 2.0] Stopped");
}

void loopMidi2Network() {
  if (!midi2Enabled) return;
  m2PollCtrl();
  m2PollData();
  // Time out dead sessions after 30 s with no clock ping
  if (m2Active && millis()-m2LastRx > 30000) {
    m2Active         = false;
    m2RemoteDataPort = 0;
    Serial.println("[MIDI 2.0] Session timed out");
  }
}
// ─────────────────────────────────────────────────────────────────────────────

// ─── MIDI Input Processing ────────────────────────────────────────────────────
void handleMidiInputMessage(uint8_t type, uint8_t channel, uint8_t data1, uint8_t data2) {
  oscOnMidiIn(type, channel, data1, data2);
  bool isNoteOn  = (type == 0x90 && data2 > 0);
  bool isNoteOff = (type == 0x80 || (type == 0x90 && data2 == 0));
  bool isCCOn    = (type == 0xB0 && data2 >= 64);
  bool isCCOff   = (type == 0xB0 && data2 < 64);

  for (int slot = 0; slot < 8; slot++) {
    SlaveInfo* slave = getSlaveInfoForSlot(slot);
    if (!slave) continue;
    if (!(slave->type >= TYPE_FOOTSWITCH_MIN && slave->type <= TYPE_FOOTSWITCH_MAX)) continue;
    for (int btn = 0; btn < 4; btn++) {
      if (slave->config.ledMode[btn] != "midi_in") continue;
      uint8_t cfgNote    = (uint8_t)slave->config.ledMidiNote[btn];
      uint8_t cfgChannel = (uint8_t)slave->config.ledMidiChannel[btn];
      bool chMatch = (cfgChannel == 0 || cfgChannel == channel);
      if (!chMatch || data1 != cfgNote) continue;
      if (isNoteOn || isCCOn) {
        midiInLEDState[slot][btn] = true;
        updateDeviceLED(slot, btn, true);
      } else if (isNoteOff || isCCOff) {
        midiInLEDState[slot][btn] = false;
        updateDeviceLED(slot, btn, false);
      }
    }
  }
}

void processMidiInput() {
  static uint8_t state   = 0;
  static uint8_t midiCmd = 0;
  static uint8_t midiD1  = 0;
  while (Serial1.available()) {
    uint8_t b = (uint8_t)Serial1.read();
    if (b & 0x80) { midiCmd = b; state = 1; }
    else {
      if (state == 1)      { midiD1 = b; state = 2; }
      else if (state == 2) {
        handleMidiInputMessage(midiCmd & 0xF0, (midiCmd & 0x0F) + 1, midiD1, b);
        state = 1;
      }
    }
  }
}
// ─────────────────────────────────────────────────────────────────────────────

void loop() {
  unsigned long currentTime = millis();
  usbmidi.update();       // Handle USB MIDI output
  processMidiInput();     // Handle hardware MIDI input (Serial1 RX)
  loopMidi2Network();
  pqLoop();               // PEQUOD Link (USB SysEx handled in usbmidi.update)
  loopOSC();              // OSC in (UDP)  // MIDI 2.0 Network (UDP)                    // Accept/read MIDI 2.0 UMP clients

  // Auto-extinguish midi_out_triggered LEDs 100ms after MIDI was sent
  for (int slot = 0; slot < 8; slot++) {
    for (int btn = 0; btn < 4; btn++) {
      if (midiOutTriggerTime[slot][btn] > 0 &&
          millis() - midiOutTriggerTime[slot][btn] > 100) {
        midiOutTriggerTime[slot][btn] = 0;
        updateDeviceLED(slot, btn, false);
      }
    }
  }

  checkWiFi();
  if ((wifiConnected || apModeActive)) {
    server.handleClient();
  }

  static unsigned long lastPoll = 0;
  if (millis() - lastPoll >= 10) {
    pollDevices();           // Existing device polling
    pollI2CEncoders();       // NEW: Add encoder polling
    lastPoll = millis();
  }
  
  // Add periodic LED update (after device polling)
  static unsigned long lastLEDUpdate = 0;
  if (millis() - lastLEDUpdate >= 100) {  // Update every 100ms
    int currentLayer = getCurrentActiveLayer();
    static int previousLayer = -1;

    if (currentLayer != previousLayer) {
      updateLayerLED(currentLayer);
      previousLayer = currentLayer;
      oscSendLayer(currentLayer);
    }
    oscCheckSlotLayers();

    lastLEDUpdate = millis();
  }
  
  // Update display with button-triggered switching
  static unsigned long lastDisplayUpdate = 0;

  if (!pqOwnsDisplay() && millis() - lastDisplayUpdate >= 500) {   // PEQUOD draws the screen while connected
    // Show WiFi info until any button is pressed
    if (!anyButtonPressed) {
      if (wifiConnected) {
        showWiFiInfo();
      } else {
        display.clearDisplay();
        display.setTextSize(1);
        display.setCursor(0, 0);
        display.println("WiFi: Connecting...");
        display.println("Devices: " + String(numDetected));
        display.display();
      }
    } else {
      // After any button press, show global layer info
      showGlobalLayerInfo();
    }
    lastDisplayUpdate = millis();
  }

  yield();  // Prevent watchdog timeout
}
