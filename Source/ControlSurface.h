#pragma once

#include "HostEngine.h"
#include "Tuner.h"

//==============================================================================
struct Mapping
{
    enum class Target { param, bypass, splitMix, inputGain, outputGain, presetNext, presetPrev, mute, bankNext, bankPrev, tuner,
                        stompSlot,      // on/off of the Nth effect in whatever preset is loaded (paramIndex = N, 1-based)
                        presetSelect }; // load preset N of the current bank (paramIndex = N, 1-based)
    enum class Source { midiCC, midiNote, osc, controller };

    // Controller sources: unit uid + control (0-3 switch, 100 pedal, 200 encoder turn, 201 encoder press) + layer
    enum { ctrlPedal = 100, ctrlEncoderTurn = 200, ctrlEncoderPress = 201 };

    Target target = Target::param;
    int slotUid = 0;        // slot uid (param/bypass) or rack item uid (splitMix)
    int paramIndex = -1;

    Source source = Source::midiCC;
    int midiChannel = 0;    // 1-16, 0 = any
    int number = 0;         // CC or note number
    juce::String oscAddress;

    juce::String ctrlUnit;
    int ctrlControl = 0, ctrlLayer = 0;
    juce::uint32 ledColour = 0;   // 0 = default colour for the target

    bool toggle = true;     // switch targets: true = each press toggles, false = value sets state

    bool lastPressed = false;

    bool isRackMapping() const { return target == Target::param || target == Target::bypass || target == Target::splitMix; }
    bool isSwitchTarget() const { return target == Target::bypass || target == Target::presetNext || target == Target::presetPrev || target == Target::mute
                                         || target == Target::bankNext || target == Target::bankPrev || target == Target::tuner
                                         || target == Target::stompSlot || target == Target::presetSelect; }

    bool isController() const { return source == Source::controller; }
    bool isControllerButton() const { return isController() && (ctrlControl < ctrlPedal || ctrlControl == ctrlEncoderPress); }
    bool sameControl (const juce::String& unit, int control, int layer) const
    {
        return isController() && ctrlUnit == unit && ctrlControl == control && ctrlLayer == layer;
    }
};

//==============================================================================
/** MIDI + OSC: incoming control, MIDI/OSC learn, and OSC feedback to controllers. */
class ControlSurface : public juce::ChangeBroadcaster,
                       public ParamObserver,
                       private juce::OSCReceiver::Listener<juce::OSCReceiver::MessageLoopCallback>,
                       private juce::MidiInputCallback,
                       private juce::ChangeListener,
                       private juce::Timer
{
public:
    explicit ControlSurface (HostEngine&);
    ~ControlSurface() override;

    //== OSC settings
    bool setOscInputPort (int port);
    bool setOscOutput (const juce::String& host, int port);
    int getOscInputPort() const { return oscInPort; }
    juce::String getOscOutHost() const { return oscOutHost; }
    int getOscOutPort() const { return oscOutPort; }
    bool isOscInConnected() const { return oscInOk; }
    bool isOscOutConnected() const { return oscOutOk; }

    double lastOscActivity = 0, lastMidiActivity = 0;

    //== Learn
    enum class LearnState { off, waitingForTarget, waitingForSource };
    LearnState getLearnState() const { return learnState.load(); }
    void armLearn();                                   // touch a plugin control, then move a MIDI/OSC control
    void armLearnFor (Mapping::Target, int uid = 0);   // learn a source for a fixed target
    void cancelLearn();
    juce::String getLearnStatusText() const;

    //== Foot controller (PEQUOD Link)
    /** A switch / pedal / encoder on the controller. Returns true if something was mapped to it. */
    bool handleControllerEvent (const juce::String& unit, int control, int layer, float value, int delta);
    /** Touch a plugin control next; it will be assigned to this controller control. */
    void armLearnForController (const juce::String& unit, int control, int layer, const juce::String& label);
    /** One function per control and layer: replaces whatever was assigned. */
    void setControllerMapping (const Mapping&);
    void clearControllerMapping (const juce::String& unit, int control, int layer);
    const Mapping* findControllerMapping (const juce::String& unit, int control, int layer) const;
    std::function<juce::String (const Mapping&)> controllerSourceName;
    std::function<void (const Mapping&)> onControllerMappingAdded;
    double lastControllerActivity = 0;

    void setTunerMode (bool);
    bool isTunerMode() const { return tunerMode; }

    //== Mappings
    const std::vector<Mapping>& getMappings() const { return mappings; }
    void removeMapping (int index);
    void toggleMappingMode (int index);
    void clearRackMappings();
    void slotRemoved (int slotUid);
    juce::String describeTarget (const Mapping&) const;
    juce::String describeSource (const Mapping&) const;

    std::unique_ptr<juce::XmlElement> createRackMappingsXml() const;
    void loadRackMappingsXml (const juce::XmlElement* rackRoot);
    std::unique_ptr<juce::XmlElement> createGlobalXml() const;
    void loadGlobalXml (const juce::XmlElement*);

    //== Hooks into the app (preset handling lives in MainComponent)
    std::function<void()> onPresetNext, onPresetPrev;
    std::function<void (int index)> onPresetIndex;        // 0-based
    std::function<void (const juce::String&)> onPresetName;
    std::function<juce::String()> getPresetName;
    std::function<int()> getPresetIndex;                  // 0-based within the bank, -1 if unsaved
    std::function<void()> onBankNext, onBankPrev;
    std::function<void (int index, bool loadPreset)> onBankIndex;   // 0-based
    std::function<void (const juce::String&)> onBankName;
    std::function<juce::String()> getBankName;
    std::function<int()> getBankIndex;

    void setTuner (Tuner* t) { tuner = t; }
    void setMuted (bool shouldMute);
    bool isMuted() const { return engine.getOutputStage()->muted.load(); }

    void sendFullState();
    void sendRackSummary();

    // ParamObserver
    void pluginParameterChanged (int slotUid, int paramIndex, float value) override;
    void pluginParameterGesture (int slotUid, int paramIndex) override;

    static float dbToNorm (float db) { return juce::jlimit (0.0f, 1.0f, (db + 36.0f) / 48.0f); }
    static float normToDb (float v)  { return -36.0f + 48.0f * juce::jlimit (0.0f, 1.0f, v); }

private:
    HostEngine& engine;

    juce::OSCReceiver receiver;
    juce::OSCSender sender;
    int oscInPort = 9000;
    juce::String oscOutHost { "127.0.0.1" };
    int oscOutPort = 9001;
    bool oscInOk = false, oscOutOk = false;

    std::vector<Mapping> mappings;
    std::vector<float> lastMix;
    Tuner* tuner = nullptr;
    int lastMuteSent = -1, tunerTick = 0;
    juce::String lastTunerNote;

    std::atomic<LearnState> learnState { LearnState::off };
    Mapping pending;
    bool pendingHasControllerSource = false;
    juce::String pendingControllerLabel;
    bool tunerMode = false;
    double learnArmedTime = 0;
    std::atomic<juce::int64> learnCandidate { -1 };

    struct ParamEvent { int slotUid; int index; float value; };
    juce::AbstractFifo fifo { 8192 };
    std::vector<ParamEvent> fifoData = std::vector<ParamEvent> (8192);

    void oscMessageReceived (const juce::OSCMessage&) override;
    void oscBundleReceived (const juce::OSCBundle&) override;
    void handleIncomingMidiMessage (juce::MidiInput*, const juce::MidiMessage&) override;
    void changeListenerCallback (juce::ChangeBroadcaster*) override;
    void timerCallback() override;

    void handleMidi (const juce::MidiMessage&);
    bool handleFixedOsc (const juce::OSCMessage&, float value);
    void apply (Mapping&, float value, bool isPress);
    void finishLearn (Mapping::Source, int channel, int number, const juce::String& address);
    void send (const juce::String& address, float v);
    void send (const juce::String& address, int v);
    void send (const juce::String& address, const juce::String& s);

    juce::AudioProcessorParameter* getParam (int slotUid, int index) const;

    JUCE_DECLARE_WEAK_REFERENCEABLE (ControlSurface)
};
