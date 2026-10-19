#pragma once

#include <JuceHeader.h>
#include <atomic>
#include <memory>
#include <vector>

//==============================================================================
/** Input conditioning + gain + peak metering. Used for the master IN and OUT stages. */
class GainMeterProcessor : public juce::AudioProcessor
{
public:
    enum class InputMode { Stereo = 0, Left, Right, Sum };

    GainMeterProcessor()
        : AudioProcessor (BusesProperties().withInput  ("In",  juce::AudioChannelSet::stereo(), true)
                                           .withOutput ("Out", juce::AudioChannelSet::stereo(), true)) {}

    std::atomic<float> gainDb { 0.0f };
    std::atomic<int>   inputMode { (int) InputMode::Stereo };
    std::atomic<float> peakL { 0.0f }, peakR { 0.0f };

    void prepareToPlay (double, int) override { smoothed.reset (getSampleRate() > 0 ? getSampleRate() : 48000.0, 0.02); smoothed.setCurrentAndTargetValue (juce::Decibels::decibelsToGain (gainDb.load())); }
    void releaseResources() override {}

    using AudioProcessor::processBlock;

    void processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer&) override
    {
        const int n = buffer.getNumSamples();
        if (buffer.getNumChannels() < 2) return;

        auto* l = buffer.getWritePointer (0);
        auto* r = buffer.getWritePointer (1);

        switch ((InputMode) inputMode.load())
        {
            case InputMode::Left:  juce::FloatVectorOperations::copy (r, l, n); break;
            case InputMode::Right: juce::FloatVectorOperations::copy (l, r, n); break;
            case InputMode::Sum:
                for (int i = 0; i < n; ++i) { auto s = 0.5f * (l[i] + r[i]); l[i] = s; r[i] = s; }
                break;
            case InputMode::Stereo: default: break;
        }

        smoothed.setTargetValue (juce::Decibels::decibelsToGain (gainDb.load(), -60.0f));
        float pl = 0.0f, pr = 0.0f;
        for (int i = 0; i < n; ++i)
        {
            auto g = smoothed.getNextValue();
            l[i] *= g; r[i] *= g;
            pl = juce::jmax (pl, std::abs (l[i]));
            pr = juce::jmax (pr, std::abs (r[i]));
        }
        peakL.store (juce::jmax (pl, peakL.load() * 0.92f));
        peakR.store (juce::jmax (pr, peakR.load() * 0.92f));
    }

    const juce::String getName() const override             { return "GainMeter"; }
    bool acceptsMidi() const override                        { return false; }
    bool producesMidi() const override                       { return false; }
    double getTailLengthSeconds() const override             { return 0.0; }
    juce::AudioProcessorEditor* createEditor() override      { return nullptr; }
    bool hasEditor() const override                          { return false; }
    int getNumPrograms() override                            { return 1; }
    int getCurrentProgram() override                         { return 0; }
    void setCurrentProgram (int) override                    {}
    const juce::String getProgramName (int) override         { return {}; }
    void changeProgramName (int, const juce::String&) override {}
    void getStateInformation (juce::MemoryBlock&) override   {}
    void setStateInformation (const void*, int) override     {}

private:
    juce::SmoothedValue<float> smoothed;
};

//==============================================================================
/** Crossfades the two lanes of a split module: in 0-1 = lane A, in 2-3 = lane B. */
class SplitMixerProcessor : public juce::AudioProcessor
{
public:
    SplitMixerProcessor()
        : AudioProcessor (BusesProperties().withInput  ("A",   juce::AudioChannelSet::stereo(), true)
                                           .withInput  ("B",   juce::AudioChannelSet::stereo(), true)
                                           .withOutput ("Out", juce::AudioChannelSet::stereo(), true)) {}

    std::atomic<float> mix { 0.5f }; // 0 = all A, 1 = all B, 0.5 = both at full level

    void prepareToPlay (double sr, int) override { smoothed.reset (sr > 0 ? sr : 48000.0, 0.02); smoothed.setCurrentAndTargetValue (mix.load()); }
    void releaseResources() override {}

    using AudioProcessor::processBlock;

    void processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer&) override
    {
        if (buffer.getNumChannels() < 4) return;
        const int n = buffer.getNumSamples();
        smoothed.setTargetValue (mix.load());

        auto* aL = buffer.getWritePointer (0);
        auto* aR = buffer.getWritePointer (1);
        auto* bL = buffer.getReadPointer (2);
        auto* bR = buffer.getReadPointer (3);

        for (int i = 0; i < n; ++i)
        {
            const float m = smoothed.getNextValue();
            // Guitar Rig style: centre = both lanes at unity, edges fade the other lane out.
            const float ga = juce::jlimit (0.0f, 1.0f, 2.0f * (1.0f - m));
            const float gb = juce::jlimit (0.0f, 1.0f, 2.0f * m);
            aL[i] = aL[i] * ga + bL[i] * gb;
            aR[i] = aR[i] * ga + bR[i] * gb;
        }
    }

    const juce::String getName() const override             { return "SplitMix"; }
    bool acceptsMidi() const override                        { return false; }
    bool producesMidi() const override                       { return false; }
    double getTailLengthSeconds() const override             { return 0.0; }
    juce::AudioProcessorEditor* createEditor() override      { return nullptr; }
    bool hasEditor() const override                          { return false; }
    int getNumPrograms() override                            { return 1; }
    int getCurrentProgram() override                         { return 0; }
    void setCurrentProgram (int) override                    {}
    const juce::String getProgramName (int) override         { return {}; }
    void changeProgramName (int, const juce::String&) override {}
    void getStateInformation (juce::MemoryBlock&) override   {}
    void setStateInformation (const void*, int) override     {}

private:
    juce::SmoothedValue<float> smoothed;
};

//==============================================================================
/** Receives every parameter change of every hosted plugin (from any thread). */
struct ParamObserver
{
    virtual ~ParamObserver() = default;
    virtual void pluginParameterChanged (int slotUid, int paramIndex, float value) = 0;
    virtual void pluginParameterGesture (int slotUid, int paramIndex) = 0;
};

//==============================================================================
struct PluginSlot
{
    int uid = 0;
    juce::AudioProcessorGraph::Node::Ptr node;
    juce::PluginDescription description;
    bool bypassed = false;

    juce::AudioPluginInstance* instance() const
    {
        return node != nullptr ? dynamic_cast<juce::AudioPluginInstance*> (node->getProcessor()) : nullptr;
    }

    struct Listener;
    std::unique_ptr<Listener> listener;
};

struct RackItem
{
    enum class Type { plugin, split };

    int uid = 0;
    Type type = Type::plugin;

    std::shared_ptr<PluginSlot> plugin;                    // Type::plugin
    std::vector<std::shared_ptr<PluginSlot>> lanes[2];     // Type::split
    juce::AudioProcessorGraph::Node::Ptr mixerNode;        // Type::split
    float mix = 0.5f;

    SplitMixerProcessor* mixer() const
    {
        return mixerNode != nullptr ? dynamic_cast<SplitMixerProcessor*> (mixerNode->getProcessor()) : nullptr;
    }
};

/** Where a plugin sits / should go. lane == -1 means the top level of the rack. */
struct RackLocation
{
    int itemIndex = -1;  // top-level index (insert position when lane == -1)
    int lane = -1;       // 0 = A, 1 = B
    int laneIndex = 0;   // insert position inside the lane
};

//==============================================================================
class HostEngine : public juce::ChangeBroadcaster,
                   private juce::ChangeListener
{
public:
    HostEngine();
    ~HostEngine() override;

    //== Devices
    juce::AudioDeviceManager deviceManager;
    juce::AudioPluginFormatManager formatManager;
    juce::KnownPluginList knownPlugins;
    juce::AudioProcessorPlayer player;

    void initialiseAudio (const juce::XmlElement* savedDeviceState);

    //== Rack
    const std::vector<RackItem>& getItems() const noexcept { return items; }

    /** Adds a plugin; returns the new slot uid or 0 on failure (errorOut filled). */
    int addPlugin (const juce::PluginDescription&, RackLocation where, juce::String& errorOut);
    int addSplit (int itemIndex);
    void removeSlot (int slotUid);
    void removeItem (int itemUid);
    void moveSlot (int slotUid, RackLocation where);
    void moveItem (int itemUid, int newIndex);
    void clearRack();

    void setBypassed (int slotUid, bool shouldBypass);
    void setSplitMix (int itemUid, float mix);

    std::shared_ptr<PluginSlot> findSlot (int slotUid) const;
    RackItem* findItem (int itemUid);
    bool locateSlot (int slotUid, RackLocation& out) const;

    /** Every plugin in rack order (lane A before lane B). Used for 1-based OSC numbering. */
    std::vector<std::shared_ptr<PluginSlot>> getFlatSlots() const;
    int getFlatIndex (int slotUid) const; // 1-based, 0 if not found
    /** Splits in rack order (1-based OSC numbering). */
    std::vector<RackItem*> getSplits();
    int getSplitNumber (int itemUid) const;

    //== Master
    GainMeterProcessor* getInputStage() const  { return inputStage; }
    GainMeterProcessor* getOutputStage() const { return outputStage; }

    //== Presets
    std::unique_ptr<juce::XmlElement> createRackXml() const;
    bool loadRackXml (const juce::XmlElement&, juce::StringArray& errors);

    juce::String currentPresetName { "Init" };

    //== Parameter observation (OSC feedback, learn)
    void setParamObserver (ParamObserver* o) { observer = o; }

    std::function<void (int slotUid)> onSlotAboutToBeRemoved;

    void rewire();

    juce::AudioProcessorGraph& getGraph() noexcept { return graph; }

    /** Increments on every rack change (used to show the preset as modified). */
    int getChangeCounter() const noexcept { return changeCounter; }
    void changed() { ++changeCounter; sendChangeMessage(); }

private:
    int changeCounter = 0;
    juce::AudioProcessorGraph graph;
    juce::AudioProcessorGraph::Node::Ptr audioIn, audioOut, midiIn, inNode, outNode;
    GainMeterProcessor* inputStage = nullptr;
    GainMeterProcessor* outputStage = nullptr;

    std::vector<RackItem> items;
    int nextUid = 1;
    ParamObserver* observer = nullptr;

    std::shared_ptr<PluginSlot> createSlot (const juce::PluginDescription&, juce::String& error);
    void destroySlot (const std::shared_ptr<PluginSlot>&);
    std::shared_ptr<PluginSlot> detachSlot (int slotUid);
    void insertSlot (std::shared_ptr<PluginSlot>, RackLocation where);
    void attachListener (PluginSlot&);

    void connectAudio (juce::AudioProcessorGraph::NodeID src, int srcChans,
                       juce::AudioProcessorGraph::NodeID dst, int dstChans, int dstOffset = 0);
    int numOuts (const juce::AudioProcessorGraph::Node::Ptr&) const;
    int numIns  (const juce::AudioProcessorGraph::Node::Ptr&) const;

    void changeListenerCallback (juce::ChangeBroadcaster*) override;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (HostEngine)
};
