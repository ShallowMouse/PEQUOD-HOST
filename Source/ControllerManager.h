#pragma once

#include "ControllerLink.h"
#include "ControlSurface.h"
#include "Tuner.h"

//==============================================================================
/** Everything PEQUOD does with the PEQUOD FC foot controller: the controller's own
    settings (edited live), PEQUOD functions on its switches, LED + screen feedback,
    and firmware updates. See docs/controller-protocol.md. */
class ControllerManager : public juce::ChangeBroadcaster,
                          private ControllerLink::Listener,
                          private juce::ChangeListener,
                          private juce::Timer
{
public:
    ControllerManager (HostEngine&, ControlSurface&, Tuner&, juce::PropertiesFile& settings);
    ~ControllerManager() override;

    ControllerLink& getLink() noexcept         { return link; }
    bool isConnected() const                   { return link.isConnected(); }
    const ControllerInfo& getInfo() const      { return link.getInfo(); }
    juce::String describeConnection() const    { return link.describeConnection(); }

    void setManualAddress (const juce::String& ip);
    juce::String getManualAddress() const      { return link.getManualAddress(); }

    //== The controller's settings ("cfg" in the protocol). Paths use dots:
    //   "brightness", "wifi.ssid", "units.A1B2C3D4.controls.0.layers.1.mode"
    bool hasConfig() const                     { return config.isObject(); }
    juce::var get (const juce::String& path) const;
    void set (const juce::String& path, const juce::var& value);     // sent to the controller straight away
    void setName (const juce::String&);                               // also renames its USB port (restarts it)

    int getLayer() const                       { return layer; }
    int getNumLayers() const;
    void selectLayer (int);
    juce::Colour layerColour (int) const;

    //== Controls. code: 0-3 switch, 100 pedal, 200 encoder turn, 201 encoder push
    static juce::Array<int> controlCodes (const ControllerUnit&);
    static juce::String controlName (const ControllerUnit&, int code);
    static bool isSwitch (int code)            { return (code >= 0 && code < 4) || code == Mapping::ctrlEncoderPress; }
    static bool hasLed (const ControllerUnit&, int code);
    static juce::String controlPath (const juce::String& uid, int code)  { return "units." + uid + ".controls." + juce::String (code); }
    static juce::String actionPath (const juce::String& uid, int code, int lay) { return controlPath (uid, code) + ".layers." + juce::String (lay); }

    juce::String unitName (const juce::String& uid) const;
    juce::String describeControl (const juce::String& uid, int code, int lay) const;
    juce::String modeOf (const juce::String& uid, int code, int lay) const;           // "pequod", "cc", ...
    juce::String shortLabel (const juce::String& uid, int code, int lay) const;       // what it does, in a few letters
    juce::String shortLabelOf (const Mapping&, int code) const;
    juce::Colour colourOf (const juce::String& uid, int code) const;
    bool isLit (const juce::String& uid, int code) const;                              // as on the hardware (roughly)
    bool isPressed (const juce::String& uid, int code) const;
    float pedalValue (const juce::String& uid) const;
    int pedalRaw (const juce::String& uid) const;
    void watchPedal (const juce::String& uid, bool on);

    /** Called when any switch on the controller is pressed (whatever it is set to do). */
    std::function<void (const juce::String& uid, int code)> onControlPressed;

    //== PEQUOD functions
    /** scope: -1 = keep the scope of what's there now (or the usual one for that function), 0 = every preset, 1 = this preset */
    /** Adds a function to a control (or replaces `replace`, one of its functions). New functions take the
        control's scope, latching and light settings. */
    void assign (const juce::String& uid, int code, int lay, Mapping::Target, int uidOrIndex = 0, const Mapping* replace = nullptr);
    void removeFunction (const Mapping&);
    std::vector<const Mapping*> mappingsFor (const juce::String& uid, int code, int lay) const;
    bool controlIsOn (const juce::String& uid, int code, int lay) const;
    juce::Colour controlEffectColour (const juce::String& uid, int code, int lay) const;
    /** The colour of the plugin a mapping controls (PEQUOD category colours), or transparent. */
    juce::Colour effectColour (const Mapping&) const;
    /** Follow the colour of the effect it controls (default), or use the switch's own colour.
        Works before a function is chosen too; the choice is kept for when one is. */
    bool usesEffectColour (const juce::String& uid, int code, int lay) const;
    void setUseEffectColour (const juce::String& uid, int code, int lay, bool);
    bool isLatching (const juce::String& uid, int code, int lay) const;
    void setLatching (const juce::String& uid, int code, int lay, bool);
    void unassign (const juce::String& uid, int code, int lay);
    void learnPluginControl (const juce::String& uid, int code, int lay, const Mapping* replace = nullptr);
    const Mapping* mappingFor (const juce::String& uid, int code, int lay) const;
    bool quickSetup();      // a sensible layout for every control that isn't set up yet

    //== Maintenance
    void identify (const juce::String& uid, int code = -1);
    void rescan();
    void reboot();
    void factoryReset();
    void scanWifi();
    struct Network { juce::String ssid; int rssi = 0; bool lock = false; };
    const std::vector<Network>& getNetworks() const { return networks; }
    bool isScanningWifi() const               { return wifiScanning; }

    bool startFirmwareUpdate (const juce::File&, juce::String& whyNot);
    bool isUpdating() const                   { return fw.active; }
    float updateProgress() const              { return fw.total > 0 ? (float) fw.next / (float) fw.total : 0.0f; }
    juce::String updateStatus() const         { return fw.status; }

    std::function<void (const juce::String&)> onStatus;
    std::function<juce::StringArray()> getPresetNames;    // presets of the current bank, in order

private:
    HostEngine& engine;
    ControlSurface& control;
    Tuner& tuner;
    juce::PropertiesFile& props;
    ControllerLink link;

    juce::var config;
    bool configDirty = false;
    double configDirtySince = 0;
    int layer = 0;
    double rebootAt = 0;
    std::map<juce::String, bool> pendingFx, pendingLatch;
    struct LearnFor { bool perPreset, toggle; juce::uint32 led; };
    std::map<juce::String, LearnFor> pendingLearn;     // a plugin control being learned as an extra function   // "uid:code:layer", for controls without a function yet

    std::map<juce::String, bool> pressed;      // "uid:code"
    std::map<juce::String, float> pedals;
    std::map<juce::String, int> raws;
    std::set<juce::String> watched;
    std::vector<Network> networks;
    bool wifiScanning = false;

    juce::String lastLeds, lastScreen, pendingFlash;
    double lastLedsSent = 0, lastScreenSent = 0, lastParamFlash = 0;

    struct FirmwareJob
    {
        bool active = false;
        juce::MemoryBlock image;
        int next = 0, total = 0, retries = 0;
        double sentAt = 0;
        bool waitingBegin = false;
        juce::String status;
    } fw;
    static constexpr int fwChunk = 768;

    void controllerConnected() override;
    void controllerDisconnected() override;
    void controllerInfoChanged() override;
    void controllerMessage (const juce::String& type, const juce::var&) override;
    void changeListenerCallback (juce::ChangeBroadcaster*) override;
    void timerCallback() override;

    void sendCmd (const juce::String& type, std::initializer_list<std::pair<const char*, juce::var>> fields = {});
    void sendConfigNow();
    void updateLeds (bool force);
    void updateScreen (bool force);
    bool mappingIsOn (const Mapping&) const;
    juce::String flashFor (const Mapping&, bool afterPress) const;
    void handleEvent (const juce::var&);
    void sendFirmwareChunk();
    void finishFirmware (const juce::String& status);
    void status (const juce::String&);

    JUCE_DECLARE_NON_COPYABLE (ControllerManager)
};
