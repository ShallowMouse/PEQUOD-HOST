#pragma once

#include "ControllerLink.h"
#include "ControlSurface.h"

class FirmwareUpdater;

//==============================================================================
/** Everything PEQUOD does with the foot controller: assignments, LED and screen
    feedback, device settings, backups and firmware updates. */
class ControllerManager : public juce::ChangeBroadcaster,
                          private ControllerLink::Listener,
                          private juce::Timer
{
public:
    ControllerManager (HostEngine&, ControlSurface&, juce::PropertiesFile& settings);
    ~ControllerManager() override;

    ControllerLink& getLink() noexcept { return link; }
    bool isConnected() const          { return link.isConnected(); }
    const ControllerInfo& getInfo() const { return link.getInfo(); }

    //== Controls
    static juce::String controlName (const ControllerUnit&, int control);
    juce::String describeSource (const Mapping&) const;
    juce::String shortLabel (const Mapping&) const;            // what the switch does, for the panel
    juce::Colour colourFor (const Mapping&) const;              // its LED colour
    static juce::Colour defaultColour (Mapping::Target);

    int currentLayerFor (const ControllerUnit& u) const { return u.layer; }
    bool isPressed (const juce::String& uid, int control) const;
    float pedalValue (const juce::String& uid) const;
    juce::Colour ledColourShown (const juce::String& uid, int control) const;   // as on the hardware

    /** True when PEQUOD owns this control (it sends no MIDI of its own). */
    bool isClaimed (const ControllerUnit&, int control, int layer) const;
    void release (const ControllerUnit&, int control, int layer);
    void setLedColour (const juce::String& uid, int control, int layer, juce::Colour);
    void identify (const ControllerUnit&, int control);
    void selectLayer (int layer);

    //== Device settings
    void setSettings (const juce::var& patch);                  // {"layers":..,"brightness":..,"wifi":{..}}
    const juce::var& getSettings() const { return settingsCache; }
    void setUnitConfig (int slot, const juce::var& patch);      // patch in the device JSON layout
    juce::var getUnitConfig (int slot) const;
    void rescanUnits();
    void watchPedal (int slot, bool on);
    int pedalRaw (int slot) const;

    //== Backups
    juce::File getBackupFolder() const;
    bool saveBackup (juce::File target = {});
    bool restoreBackup (const juce::File&, juce::String& report);

    //== Firmware
    static bool hasBundledFirmware();
    static juce::String bundledFirmwareVersion();
    static int compareVersions (const juce::String& a, const juce::String& b);
    bool updateAvailable() const;
    bool startFirmwareUpdate (juce::String& whyNot);
    bool isUpdating() const;
    float updateProgress() const;
    juce::String updateStatus() const;

    std::function<void (const juce::String&)> onStatus;

private:
    HostEngine& engine;
    ControlSurface& control;
    juce::PropertiesFile& props;
    ControllerLink link;

    std::map<int, juce::var> cfgCache;         // slot -> device JSON
    juce::var settingsCache;
    std::map<juce::String, double> pressedUntil;   // "uid:control" -> time it was pressed (0 = released)
    std::map<juce::String, bool> pressedNow;
    std::map<juce::String, float> pedals;
    std::map<int, int> rawPedals;
    std::map<juce::String, juce::String> lastLedSent;  // "slot:btn" -> colour hex
    std::map<juce::String, juce::Colour> ledShown;     // "uid:control"
    juce::String lastScreen;
    double lastScreenSent = 0, flashUntil = 0;
    juce::String flashText;
    bool backupPending = false;
    std::vector<std::tuple<juce::String, int, int>> pendingClaims;
    std::unique_ptr<FirmwareUpdater> updater;

    void controllerConnected() override;
    void controllerDisconnected() override;
    void controllerInfoChanged() override;
    void controllerMessage (const juce::String& type, const juce::var&) override;
    void timerCallback() override;

    void requestConfigs();
    void claim (const juce::String& uid, int control, int layer);
    juce::String controlType (int slot, const ControllerUnit&, int control, int layer) const;
    juce::var typePatch (const ControllerUnit&, int control, int layer, const juce::String& type) const;
    juce::String uidForSlot (int slot) const;
    void onPress (const ControllerUnit&, int control, int layer, float value, int delta);
    void updateLeds (bool force);
    void updateScreen (bool force);
    bool mappingIsOn (const Mapping&) const;
    juce::String flashFor (const Mapping&) const;
    void flash (const juce::String&);

    juce::var prevTypes() const;
    void setPrevType (const juce::String& key, const juce::String& type);

    friend class FirmwareUpdater;
    JUCE_DECLARE_NON_COPYABLE (ControllerManager)
};
